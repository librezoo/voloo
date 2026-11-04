//! Background process end to end: auto-start, HTTP auth, SSE, MCP over stdio, locking,
//! unplug and idle exit, against two simulated devices on PTYs (VOLOO_PORTS).
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use voloo::{
    client::Client,
    daemon::DaemonInfo,
    sim::{Shared, SimConfig, serve_tty, shared},
};

const UID_A: &str = "01-23-45-67-89-AB-CD-EF";
const UID_B: &str = "11-22-33-44-55-66-77-88";

struct Sim {
    path: String,
    state: Shared,
    stop: Arc<AtomicBool>,
    server: Option<std::thread::JoinHandle<()>>,
}
impl Sim {
    fn new(cfg: SimConfig) -> Sim {
        let (master, slave) = serialport::TTYPort::pair().unwrap();
        let path = serialport::SerialPort::name(&slave).unwrap();
        drop(slave);
        let state = shared(cfg);
        let stop = Arc::new(AtomicBool::new(false));
        let server = {
            let (s, st) = (state.clone(), stop.clone());
            std::thread::spawn(move || serve_tty(master, s, st))
        };
        Sim {
            path,
            state,
            stop,
            server: Some(server),
        }
    }
    /// Closes the PTY master: the port disappears like an unplugged USB device.
    fn unplug(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(s) = self.server.take() {
            let _ = s.join();
        }
    }
}
impl Drop for Sim {
    fn drop(&mut self) {
        self.unplug();
    }
}

/// Background-process tests run one at a time: closed PTY names are recycled by the system,
/// and a concurrent test could otherwise be handed a name another test's process still uses.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Rig {
    _serial: std::sync::MutexGuard<'static, ()>,
    home: tempfile::TempDir,
    sims: Vec<Sim>,
    env: Vec<(String, String)>,
}
impl Rig {
    fn new(config: &str, idle_exit_s: u64) -> Rig {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let a = Sim::new(SimConfig {
            nickname: "bench-a".into(),
            ..SimConfig::default()
        });
        let b = Sim::new(SimConfig {
            uid: UID_B.parse().unwrap(),
            ..SimConfig::default()
        });
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("config.toml"), config).unwrap();
        let env = vec![
            ("VOLOO_HOME".into(), home.path().display().to_string()),
            (
                "VOLOO_PORTS".into(),
                format!("{}={UID_A};{}={UID_B}", a.path, b.path),
            ),
            ("VOLOO_IDLE_EXIT_S".into(), idle_exit_s.to_string()),
            // Never touch real hardware from tests: only the simulated ports above.
            ("VOLOO_SCAN_USB".into(), "0".into()),
        ];
        Rig {
            _serial: serial,
            home,
            sims: vec![a, b],
            env,
        }
    }
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_voloo"));
        c.envs(self.env.iter().map(|(k, v)| (k, v))).args(args);
        c
    }
    fn run(&self, args: &[&str]) -> (i32, Value) {
        let out = self.cmd(&[&["--json"], args].concat()).output().unwrap();
        (
            out.status.code().unwrap(),
            serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
        )
    }
    fn info(&self) -> Option<DaemonInfo> {
        serde_json::from_slice(&std::fs::read(self.home.path().join("daemon.json")).ok()?).ok()
    }
    fn client(&self) -> Client {
        Client {
            info: self.info().expect("daemon running"),
            source: "http",
        }
    }
    fn http(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let c = self.client();
        c.send(method, path, body.as_ref(), Duration::from_secs(30))
            .map_err(|e| format!("{e:?}"))
            .unwrap()
    }
    /// Waits until the background process sees both devices online.
    fn ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (_, v) = self.run(&["ls"]);
            if v.as_array()
                .is_some_and(|a| a.len() == 2 && a.iter().all(|d| d["online"] == true))
            {
                return;
            }
            assert!(Instant::now() < deadline, "devices not online: {v}");
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    fn home(&self) -> PathBuf {
        self.home.path().to_path_buf()
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.cmd(&["daemon", "stop"]).output();
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.info().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn raw_request(port: u16, head: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(head.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    (out.split_whitespace().nth(1).unwrap().parse().unwrap(), out)
}

#[test]
fn autostart_cli_http_and_auth() {
    let rig = Rig::new("", 600);
    rig.ready();
    let pid = rig.info().unwrap().pid;
    let (_, v) = rig.run(&["ls"]);
    let a = v
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["uid"] == UID_A)
        .unwrap();
    assert_eq!(a["nickname"], "bench-a");
    assert_eq!(a["status"]["power_state"], "OFF");

    // CLI goes through the same process; devices by nickname or UID.
    let (code, v) = rig.run(&["on", "bench-a"]);
    assert_eq!((code, v["outcome"].as_str()), (0, Some("completed")), "{v}");
    let (code, v) = rig.run(&["cycle", UID_B, "--off-ms", "0"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(rig.info().unwrap().pid, pid, "one background process");
    let (code, v) = rig.run(&["status"]);
    assert_eq!(code, 4, "two devices: the target is required: {v}");
    let (code, _) = rig.run(&["--direct", "status", UID_A]);
    assert_eq!(
        code, 5,
        "direct mode cannot open a port the background process owns"
    );

    // HTTP API with the token.
    let (status, v) = rig.http(
        "POST",
        &format!("/v1/devices/{UID_A}/power"),
        Some(json!({"action": "discharge"})),
    );
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["status"]["low_qualified"], true);
    let (status, v) = rig.http(
        "POST",
        "/v1/devices/nobody/power",
        Some(json!({"action": "on"})),
    );
    assert_eq!(
        (status, v["error"]["code"].as_str()),
        (404, Some("not_found"))
    );
    let (status, v) = rig.http("GET", "/v1/operations?limit=10", None);
    assert_eq!(status, 200);
    let sources: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["source"].as_str())
        .collect();
    assert!(
        sources.contains(&"cli") && sources.contains(&"http"),
        "{sources:?}"
    );
    let requests = |v: &Value| -> u64 {
        v["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["metrics"]["link"]["requests"].as_u64().unwrap())
            .sum()
    };
    let before = requests(&rig.http("GET", "/v1/metrics", None).1);
    for _ in 0..20 {
        assert_eq!(
            rig.http("GET", &format!("/v1/devices/{UID_A}"), None).0,
            200
        );
    }
    let (status, v) = rig.http("GET", "/v1/metrics", None);
    assert_eq!(status, 200);
    assert!(
        requests(&v) >= before + 20,
        "client requests are counted right away"
    );
    assert!(
        v["devices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["metrics"]["polls"].as_u64().unwrap() > 0)
    );
    let (status, v) = rig.http("GET", "/openapi.json", None);
    assert_eq!((status, v["openapi"].as_str()), (200, Some("3.1.0")));

    // Local-only guards.
    let port = rig.info().unwrap().port;
    let token = rig.info().unwrap().token;
    assert_eq!(
        raw_request(
            port,
            &format!(
                "GET /v1/devices HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            )
        )
        .0,
        401
    );
    assert_eq!(raw_request(port, &format!("GET /v1/devices HTTP/1.1\r\nHost: evil.example:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n")).0, 403);
    assert_eq!(
        raw_request(port, &format!("GET /v1/devices HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://evil.example\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n")).0,
        403
    );
    let (code, v) = rig.run(&["api-info"]);
    assert_eq!((code, v["token"].as_str()), (0, Some(token.as_str())));
    let (code, v) = rig.run(&["doctor"]);
    assert_eq!(code, 0, "{v}");
    let checks = v["checks"].as_array().unwrap();
    assert!(
        checks
            .iter()
            .any(|c| c["check"] == "usb_scan" && c["level"] == "warn")
    );
    assert_eq!(
        checks
            .iter()
            .filter(|c| c["check"] == "device" && c["level"] == "ok")
            .count(),
        2,
        "{v}"
    );
}

#[test]
fn sse_streams_status_operations_and_faults() {
    let rig = Rig::new("", 600);
    rig.ready();
    let info = rig.info().unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", info.port)).unwrap();
    write!(stream, "GET /v1/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nAccept: text/event-stream\r\n\r\n", info.port, info.token).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut seen = Vec::new();
    let mut acted = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut line = String::new();
    while Instant::now() < deadline {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if let Some(kind) = line.trim().strip_prefix("event:") {
            seen.push(kind.trim().to_string());
        }
        if !acted && seen.iter().any(|k| k == "hello") {
            acted = true;
            let (_, health) = rig.http("GET", "/v1/health", None);
            assert_eq!(health["watchers"], 1);
            rig.sims[0].state.lock().unwrap().inject.vin_invalid = true;
            assert_eq!(rig.run(&["on", UID_A]).0, 0);
        }
        if [
            "operation.started",
            "operation.finished",
            "status",
            "fault.raised",
        ]
        .iter()
        .all(|k| seen.iter().any(|s| s == k))
        {
            return;
        }
    }
    panic!("missing events, saw {seen:?}");
}

#[test]
fn mcp_over_stdio() {
    let rig = Rig::new("", 600);
    rig.ready();
    let mut child = rig
        .cmd(&["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |msg: Value| -> Option<Value> {
        writeln!(stdin, "{msg}").unwrap();
        msg.get("id")?; // notifications get no reply
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        Some(serde_json::from_str(&line).unwrap())
    };
    let init = ask(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}})).unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("unknown")
    );
    assert!(ask(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
    let tools = ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "voloo_list_devices",
            "voloo_get_status",
            "voloo_power",
            "voloo_set_nickname",
            "voloo_get_log"
        ]
    );

    let call = |id: u64, name: &str, args: Value| json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let list = ask(call(3, "voloo_list_devices", json!({}))).unwrap();
    assert_eq!(list["result"]["isError"], false);
    assert_eq!(
        list["result"]["structuredContent"]["devices"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let on = ask(call(
        4,
        "voloo_power",
        json!({"device": "bench-a", "action": "on"}),
    ))
    .unwrap();
    assert_eq!(
        on["result"]["structuredContent"]["outcome"], "completed",
        "{on}"
    );
    let cycle = ask(call(
        5,
        "voloo_power",
        json!({"device": "bench-a", "action": "cycle", "off_ms": 0}),
    ))
    .unwrap();
    assert_eq!(
        cycle["result"]["structuredContent"]["on"]["outcome"], "completed",
        "{cycle}"
    );
    let status = ask(call(
        6,
        "voloo_get_status",
        json!({"device": UID_B, "include": ["info", "communication"]}),
    ))
    .unwrap();
    assert_eq!(
        status["result"]["structuredContent"]["info"]["firmware_version"],
        "2.7"
    );
    let no_device = ask(call(7, "voloo_power", json!({"action": "on"}))).unwrap();
    assert_eq!(no_device["result"]["isError"], true);
    let log = ask(call(
        8,
        "voloo_get_log",
        json!({"kind": "operations", "device": "bench-a"}),
    ))
    .unwrap();
    let records = log["result"]["structuredContent"]["records"]
        .as_array()
        .unwrap();
    assert!(
        records
            .iter()
            .all(|r| r["source"] == "mcp" && r["uid"] == UID_A),
        "{records:?}"
    );
    let unknown = ask(call(9, "frobnicate", json!({}))).unwrap();
    assert_eq!(unknown["error"]["code"], -32602);
    let missing = ask(json!({"jsonrpc": "2.0", "id": 10, "method": "resources/list"})).unwrap();
    assert_eq!(missing["error"]["code"], -32601);
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn read_only_mcp_and_locked_devices() {
    let rig = Rig::new(
        &format!("[mcp]\nread_only = true\n\n[devices.\"{UID_B}\"]\nlocked = true\n"),
        600,
    );
    rig.ready();
    let (status, v) = rig.http("POST", "/mcp", Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "voloo_power", "arguments": {"device": UID_A, "action": "on"}}})));
    assert_eq!(status, 200);
    assert_eq!(v["result"]["isError"], true);
    assert!(
        v["result"]["structuredContent"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("read-only")
    );
    let (code, v) = rig.run(&["on", UID_B]);
    assert_eq!(code, 5, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("locked"));
    assert_eq!(
        rig.run(&["on", UID_A]).0,
        0,
        "CLI is not affected by MCP read-only"
    );
    assert!(
        rig.sims[1].state.lock().unwrap().writes.is_empty(),
        "nothing sent to the locked device"
    );
}

#[test]
fn unplugged_device_goes_offline_with_unknown_state() {
    let mut rig = Rig::new("", 600);
    rig.ready();
    assert_eq!(rig.run(&["on", UID_B]).0, 0);
    rig.sims[1].unplug();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, v) = rig.run(&["ls"]);
        let b = v
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["uid"] == UID_B)
            .cloned()
            .unwrap();
        if b["online"] == false {
            break;
        }
        assert!(Instant::now() < deadline, "still online: {b}");
        std::thread::sleep(Duration::from_millis(200));
    }
    let (code, v) = rig.run(&["status", UID_B]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("unknown"),
        "{v}"
    );
    assert_eq!(
        rig.run(&["status", UID_A]).0,
        0,
        "the other device is unaffected"
    );
    let (code, v) = rig.run(&["faults", UID_B]);
    assert_eq!(code, 0);
    // A PTY whose other end closed stops answering (like a hung device); a real USB unplug
    // also removes the port and reports LINK_UNPLUGGED.
    assert!(
        v.as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "LINK_UNPLUGGED" || f["code"] == "LINK_NO_RESPONSE"),
        "{v}"
    );
}

#[test]
fn vanished_port_records_unplug_and_recovers() {
    // Configure the device through a symlink so deleting it looks like the USB port vanishing.
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let sim = Sim::new(SimConfig::default());
    let home = tempfile::tempdir().unwrap();
    let link = home.path().join("voloo-port");
    std::os::unix::fs::symlink(&sim.path, &link).unwrap();
    let rig = Rig {
        _serial: serial,
        env: vec![
            ("VOLOO_HOME".into(), home.path().display().to_string()),
            ("VOLOO_PORTS".into(), format!("{}={UID_A}", link.display())),
            ("VOLOO_IDLE_EXIT_S".into(), "600".into()),
            ("VOLOO_SCAN_USB".into(), "0".into()),
        ],
        home,
        sims: vec![sim],
    };
    let online = |want: bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (_, v) = rig.run(&["ls"]);
            let d = v
                .as_array()
                .and_then(|a| a.first().cloned())
                .unwrap_or(Value::Null);
            if d["online"] == want {
                return d;
            }
            assert!(Instant::now() < deadline, "online != {want}: {v}");
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    online(true);
    std::fs::remove_file(&link).unwrap();
    let d = online(false);
    assert!(
        d["error"]["message"]
            .as_str()
            .unwrap()
            .contains("disappeared"),
        "{d}"
    );
    assert!(
        d["faults"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "LINK_UNPLUGGED" && f["active"] == true),
        "{d}"
    );
    std::os::unix::fs::symlink(&rig.sims[0].path, &link).unwrap();
    let d = online(true);
    assert!(
        !d["faults"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "LINK_UNPLUGGED" && f["active"] == true),
        "cleared on return: {d}"
    );
}

#[test]
fn idle_background_process_exits_without_touching_outputs() {
    let rig = Rig::new("", 2);
    rig.ready();
    assert_eq!(rig.run(&["on", UID_A]).0, 0);
    let deadline = Instant::now() + Duration::from_secs(8);
    while rig.info().is_some() {
        assert!(Instant::now() < deadline, "did not exit when idle");
        std::thread::sleep(Duration::from_millis(100));
    }
    let writes = rig.sims[0].state.lock().unwrap().writes.clone();
    assert_eq!(writes, vec![1], "exit sends nothing");
    assert!(
        rig.home()
            .join("logs/operations-")
            .parent()
            .unwrap()
            .exists()
    );
    // Next use starts it again.
    let (code, v) = rig.run(&["status", UID_A]);
    assert_eq!((code, v["power_state"].as_str()), (0, Some("ON")), "{v}");
}

fn header<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    raw.split("\r\n\r\n").next()?.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

#[test]
fn web_login_cookie_and_page() {
    let rig = Rig::new("", 600);
    rig.ready();
    let info = rig.info().unwrap();
    let port = info.port;
    let host = format!("Host: 127.0.0.1:{port}\r\n");
    // The page itself is public and locked down by CSP; the API is not.
    let (status, page) = raw_request(
        port,
        &format!("GET /ui/ HTTP/1.1\r\n{host}Connection: close\r\n\r\n"),
    );
    assert_eq!(status, 200);
    assert!(
        header(&page, "content-security-policy")
            .unwrap()
            .contains("script-src 'self'")
    );
    assert!(page.contains("<title>Voloo</title>"));
    assert_eq!(
        raw_request(
            port,
            &format!("GET /ui/app.js HTTP/1.1\r\n{host}Connection: close\r\n\r\n")
        )
        .0,
        200
    );
    assert_eq!(
        raw_request(
            port,
            &format!("GET /v1/devices HTTP/1.1\r\n{host}Connection: close\r\n\r\n")
        )
        .0,
        401
    );

    // `voloo ui --no-open` issues a one-time code; it becomes an HttpOnly cookie once.
    let (code, v) = rig.run(&["ui", "--no-open"]);
    assert_eq!((code, v["opened"].as_bool()), (0, Some(false)), "{v}");
    let url = v["url"].as_str().unwrap();
    let path = &url[url.find("/ui/login").unwrap()..];
    let (status, login) = raw_request(
        port,
        &format!("GET {path} HTTP/1.1\r\n{host}Connection: close\r\n\r\n"),
    );
    assert_eq!(status, 303);
    let set_cookie = header(&login, "set-cookie").unwrap();
    assert!(
        set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict"),
        "{set_cookie}"
    );
    let cookie = set_cookie.split(';').next().unwrap().to_string();
    assert!(cookie.starts_with(&format!("voloo_session_{port}=")));
    assert_eq!(
        raw_request(
            port,
            &format!("GET {path} HTTP/1.1\r\n{host}Connection: close\r\n\r\n")
        )
        .0,
        403,
        "codes are single-use"
    );

    let (status, body) = raw_request(
        port,
        &format!("GET /v1/devices HTTP/1.1\r\n{host}Cookie: {cookie}\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(raw_request(port, &format!("GET /v1/devices HTTP/1.1\r\n{host}Cookie: voloo_session_{port}=forged\r\nConnection: close\r\n\r\n")).0, 401);
    // A page on another origin cannot ride on the cookie.
    let post = |origin: &str| {
        let body = r#"{"action":"on"}"#;
        raw_request(
            port,
            &format!(
                "POST /v1/devices/bench-a/power HTTP/1.1\r\n{host}Cookie: {cookie}\r\nOrigin: {origin}\r\nX-Voloo-Source: ui\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        )
    };
    assert_eq!(post("http://evil.example").0, 403);
    assert!(rig.sims[0].state.lock().unwrap().writes.is_empty());
    let (status, body) = post(&format!("http://127.0.0.1:{port}"));
    assert_eq!(status, 200, "{body}");
    let (_, ops) = rig.http("GET", "/v1/operations?limit=5", None);
    assert_eq!(ops.as_array().unwrap().last().unwrap()["source"], "ui");
}

#[test]
fn locks_apply_to_every_client_and_survive_restart() {
    let rig = Rig::new("", 600);
    rig.ready();
    let (code, v) = rig.run(&["lock", "bench-a"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = rig.run(&["on", UID_A]);
    assert_eq!(code, 5, "{v}");
    let (_, mcp) = rig.http("POST", "/mcp", Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "voloo_power", "arguments": {"device": UID_A, "action": "on"}}})));
    assert_eq!(mcp["result"]["isError"], true);
    assert_eq!(rig.run(&["status", UID_A]).0, 0, "reads still work");
    assert!(rig.sims[0].state.lock().unwrap().writes.is_empty());
    // Restart: the lock is kept.
    assert_eq!(rig.run(&["daemon", "stop"]).0, 0);
    while rig.info().is_some() {
        std::thread::sleep(Duration::from_millis(50));
    }
    rig.ready();
    let (_, devices) = rig.http("GET", "/v1/devices", None);
    let a = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["uid"] == UID_A)
        .unwrap()
        .clone();
    assert_eq!(
        (a["locked"].as_bool(), a["lock_source"].as_str()),
        (Some(true), Some("runtime"))
    );
    assert_eq!(rig.run(&["unlock", UID_A]).0, 0);
    assert_eq!(rig.run(&["on", UID_A]).0, 0);
}

#[test]
fn stop_ends_open_event_streams() {
    let rig = Rig::new("", 600);
    rig.ready();
    let info = rig.info().unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", info.port)).unwrap();
    write!(
        stream,
        "GET /v1/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\n\r\n",
        info.port, info.token
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    assert_eq!(rig.run(&["daemon", "stop"]).0, 0);
    while rig.info().is_some() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "stop hung with an event stream open"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut rest = Vec::new();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _ = stream.read_to_end(&mut rest);
    assert!(
        rig.sims[0].state.lock().unwrap().writes.is_empty(),
        "stopping sends nothing"
    );
}

#[test]
fn demo_mode_uses_only_simulated_devices() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_voloo"))
            .env("VOLOO_HOME", home.path())
            .env("VOLOO_DEMO", "3")
            .env_remove("VOLOO_PORTS")
            .env_remove("VOLOO_SCAN_USB")
            .args([&["--json"], args].concat())
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            serde_json::from_slice::<Value>(&out.stdout).unwrap_or(Value::Null),
        )
    };
    let (code, v) = run(&["ls"]);
    assert_eq!(code, 0, "{v}");
    let ports: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["port"].as_str().unwrap())
        .collect();
    assert_eq!(
        ports,
        ["sim://1", "sim://2", "sim://3"],
        "demo never scans USB"
    );
    let (code, v) = run(&["on", "bench-a"]);
    assert_eq!((code, v["outcome"].as_str()), (0, Some("completed")), "{v}");
    let (code, v) = run(&["discharge", "bench-a"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["status"]["vout_mv"].as_u64().unwrap() <= 1000);
    let _ = run(&["daemon", "stop"]);
}

#[test]
fn report_is_local_redacted_and_complete() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_voloo"))
            .env("VOLOO_HOME", home.path())
            .env("VOLOO_DEMO", "2")
            .env_remove("VOLOO_PORTS")
            .args([&["--json"], args].concat())
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            serde_json::from_slice::<Value>(&out.stdout).unwrap_or(Value::Null),
        )
    };
    assert_eq!(run(&["on", "bench-a"]).0, 0);
    let (code, v) = run(&["report"]);
    assert_eq!(code, 0, "{v}");
    let path = PathBuf::from(v["path"].as_str().unwrap());
    let text = std::fs::read_to_string(&path).unwrap();
    let report: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["manifest"]["format"], "voloo-report/1");
    assert_eq!(report["devices"].as_array().unwrap().len(), 2);
    assert!(
        report["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["action"] == "on")
    );
    // The switch's spans are still buffered in the running process; the report asks it.
    assert!(
        report["trace"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["label"].as_str().is_some_and(|l| !l.is_empty())),
        "trace of the running process is included"
    );
    let token =
        serde_json::from_slice::<Value>(&std::fs::read(home.path().join("daemon.json")).unwrap())
            .unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string();
    assert!(
        !text.contains(&token),
        "the access token never enters a report"
    );
    #[cfg(unix)]
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions())
            & 0o777,
        0o600
    );
    let _ = run(&["daemon", "stop"]);
}

#[test]
fn stop_waits_for_exit_so_the_next_command_starts_a_new_process() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_voloo"))
            .env("VOLOO_HOME", home.path())
            .env("VOLOO_DEMO", "1")
            .env_remove("VOLOO_PORTS")
            .args([&["--json"], args].concat())
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            serde_json::from_slice::<Value>(&out.stdout).unwrap_or(Value::Null),
        )
    };
    for _ in 0..3 {
        assert_eq!(run(&["status", "bench-a"]).0, 0);
        let (code, v) = run(&["daemon", "stop"]);
        assert_eq!(code, 0, "{v}");
        assert_eq!(v["exited"], true, "{v}");
    }
    let (code, v) = run(&["api-info"]);
    assert_eq!(code, 0, "{v}");
    let _ = run(&["daemon", "stop"]);
}
