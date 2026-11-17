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

#[test]
fn hardware_revision_survives_updates_and_old_identity_software_is_refused() {
    let rig = Rig::new("", 600);
    {
        let mut state = rig.sims[0].state.lock().unwrap();
        state.enable_ab();
        state.firmware.as_mut().unwrap().versions = [0x302, 0x302];
    }
    rig.ready();
    let file = rig.home().join("new.voloofw");
    std::fs::write(&file, serde_json::to_vec(&firmware_package(0x303)).unwrap()).unwrap();
    let (code, result) = rig.run(&["firmware", "update", UID_A, file.to_str().unwrap()]);
    assert_eq!(code, 0, "{result}");
    let (code, info) = rig.run(&["info", UID_A]);
    assert_eq!(code, 0);
    assert_eq!(info["hardware_version"], "0.2");
    let (code, result) = rig.run(&["firmware", "rollback", UID_A]);
    assert_eq!(code, 0, "{result}");
    let (_, info) = rig.run(&["info", UID_A]);
    assert_eq!(info["hardware_version"], "0.2");
    let before = rig.sims[0]
        .state
        .lock()
        .unwrap()
        .firmware
        .as_ref()
        .unwrap()
        .actions
        .clone();
    std::fs::write(&file, serde_json::to_vec(&firmware_package(0x301)).unwrap()).unwrap();
    let (code, _) = rig.run(&["firmware", "update", UID_A, file.to_str().unwrap()]);
    assert_ne!(code, 0);
    let after = rig.sims[0]
        .state
        .lock()
        .unwrap()
        .firmware
        .as_ref()
        .unwrap()
        .actions
        .clone();
    assert!(
        after[before.len()..].iter().all(|op| *op == 0),
        "Refused downgrade emitted modifying operations"
    );
}

struct Sim {
    path: String,
    slave: Option<serialport::TTYPort>,
    state: Shared,
    stop: Arc<AtomicBool>,
    server: Option<std::thread::JoinHandle<()>>,
}
impl Sim {
    fn new(cfg: SimConfig) -> Sim {
        let (master, slave) = serialport::TTYPort::pair().unwrap();
        let path = serialport::SerialPort::name(&slave).unwrap();
        // As in `voloo dev-sim`, keep the original slave open until unplug.
        // Closing the final slave during daemon/update reconnection can leave
        // a macOS PTY passing traffic in only one direction.
        let state = shared(cfg);
        let stop = Arc::new(AtomicBool::new(false));
        let server = {
            let (s, st) = (state.clone(), stop.clone());
            std::thread::spawn(move || serve_tty(master, s, st))
        };
        Sim {
            path,
            slave: Some(slave),
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
        drop(self.slave.take());
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
    fn mcp(&self, name: &str, args: Value) -> Value {
        let (status, response) = self.http(
            "POST",
            "/mcp",
            Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": name, "arguments": args}})),
        );
        assert_eq!(status, 200, "{response}");
        let result = response["result"].clone();
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            text, result["structuredContent"],
            "text and structured replies agree"
        );
        result
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
fn mcp_power_nickname_and_target_resolution() {
    let rig = Rig::new("", 600);
    rig.ready();
    let call = |name, args| {
        let r = rig.mcp(name, args);
        assert_eq!(r["isError"], false, "{r}");
        r["structuredContent"].clone()
    };
    let on = call("voloo_power", json!({"device": "bench-a", "action": "on"}));
    assert_eq!(on["status"]["power_state"], "ON");
    let r = call("voloo_power", json!({"device": UID_A, "action": "on"}));
    assert_eq!(r["outcome"], "already_satisfied");
    let r = rig.mcp(
        "voloo_set_nickname",
        json!({"device": UID_A, "nickname": "busy"}),
    );
    assert_eq!(r["structuredContent"]["error"]["code"], "busy");
    assert_eq!(rig.sims[0].state.lock().unwrap().nickname_writes, 0);
    let off = call("voloo_power", json!({"device": UID_A, "action": "off"}));
    assert_eq!(off["status"]["power_state"], "OFF");
    let r = call("voloo_power", json!({"device": UID_A, "action": "off"}));
    assert_eq!(r["outcome"], "already_satisfied");
    call("voloo_power", json!({"device": UID_A, "action": "on"}));
    let r = call(
        "voloo_power",
        json!({"device": UID_A, "action": "discharge"}),
    );
    assert_eq!(r["status"]["low_qualified"], true);
    assert!(r["status"]["vout_mv"].as_u64().unwrap() <= 1000);
    let r = call(
        "voloo_power",
        json!({"device": UID_A, "action": "discharge"}),
    );
    assert_eq!(r["outcome"], "already_satisfied");
    for discharge in [true, false] {
        let r = call(
            "voloo_power",
            json!({"device": UID_A, "action": "cycle", "off_ms": 20, "discharge": discharge}),
        );
        assert_eq!(
            r["off"]["action"],
            if discharge { "discharge" } else { "off" }
        );
        assert_eq!(r["on"]["status"]["power_state"], "ON");
    }
    call("voloo_power", json!({"device": UID_A, "action": "off"}));
    for nickname in ["temporary", "零号测试零号测试", ""] {
        let r = call(
            "voloo_set_nickname",
            json!({"device": UID_A, "nickname": nickname}),
        );
        assert_eq!(r["verified"], true);
        let r = call(
            "voloo_get_status",
            json!({"device": UID_A, "include": ["info", "communication", "adc"]}),
        );
        assert_eq!(r["info"]["nickname"], nickname);
        assert_eq!(r["adc"]["supported"], true);
        if !nickname.is_empty() {
            let r = call("voloo_get_status", json!({"device": nickname}));
            assert_eq!(r["uid"], UID_A);
        }
    }
    for uid in [UID_A, UID_B] {
        call(
            "voloo_set_nickname",
            json!({"device": uid, "nickname": "duplicate"}),
        );
    }
    let writes = rig.sims[0].state.lock().unwrap().writes.clone();
    let r = rig.mcp(
        "voloo_power",
        json!({"device": "duplicate", "action": "on"}),
    );
    assert_eq!(
        r["isError"], true,
        "ambiguous nicknames must not select a device: {r}"
    );
    assert_eq!(rig.sims[0].state.lock().unwrap().writes, writes);
    assert!(rig.sims[1].state.lock().unwrap().writes.is_empty());
    let r = call(
        "voloo_get_log",
        json!({"device": UID_A, "kind": "operations", "limit": 100}),
    );
    assert!(
        r["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["source"] == "mcp")
    );
    let r = call(
        "voloo_get_log",
        json!({"device": UID_A, "kind": "trace", "trace_id": on["trace_id"]}),
    );
    assert!(!r["records"].as_array().unwrap().is_empty());
}

#[test]
fn mcp_invalid_arguments_and_write_restrictions_do_not_mutate_devices() {
    let rig = Rig::new("", 600);
    rig.ready();
    let cases = [
        ("voloo_power", json!({"action": "on"})),
        ("voloo_power", json!({"device": UID_A})),
        ("voloo_power", json!({"device": UID_A, "action": "invalid"})),
        (
            "voloo_power",
            json!({"device": UID_A, "action": "cycle", "off_ms": -1}),
        ),
        (
            "voloo_power",
            json!({"device": UID_A, "action": "cycle", "off_ms": 30001}),
        ),
        (
            "voloo_power",
            json!({"device": UID_A, "action": "cycle", "off_ms": 1.5}),
        ),
        (
            "voloo_power",
            json!({"device": UID_A, "action": "cycle", "discharge": "false"}),
        ),
        (
            "voloo_power",
            json!({"device": "no-such-device", "action": "on"}),
        ),
        ("voloo_set_nickname", json!({"nickname": "x"})),
        ("voloo_set_nickname", json!({"device": UID_A})),
        (
            "voloo_set_nickname",
            json!({"device": UID_A, "nickname": "零".repeat(9)}),
        ),
        ("voloo_get_status", json!({})), // Two devices: target is required.
        (
            "voloo_get_status",
            json!({"device": UID_A, "include": ["invalid"]}),
        ),
        ("voloo_get_log", json!({"kind": "invalid"})),
    ];
    for (name, args) in cases {
        let r = rig.mcp(name, args.clone());
        assert_eq!(r["isError"], true, "{name} {args}: {r}");
    }
    for sim in &rig.sims {
        let s = sim.state.lock().unwrap();
        assert!(s.writes.is_empty());
        assert_eq!(s.nickname_writes, 0);
    }
    drop(rig);
    let rig = Rig::new(
        &format!("[mcp]\nread_only = true\n[devices.\"{UID_B}\"]\nlocked = true\n"),
        600,
    );
    rig.ready();
    for (name, args) in [
        ("voloo_power", json!({"device": UID_A, "action": "on"})),
        (
            "voloo_set_nickname",
            json!({"device": UID_A, "nickname": "x"}),
        ),
    ] {
        assert_eq!(rig.mcp(name, args)["isError"], true);
    }
    assert_eq!(
        rig.mcp("voloo_get_status", json!({"device": UID_A}))["isError"],
        false
    );
    for sim in &rig.sims {
        assert!(sim.state.lock().unwrap().writes.is_empty());
        assert_eq!(sim.state.lock().unwrap().nickname_writes, 0);
    }
    drop(rig);
    let rig = Rig::new(&format!("[devices.\"{UID_A}\"]\nlocked = true\n"), 600);
    rig.ready();
    for (name, args) in [
        ("voloo_power", json!({"device": UID_A, "action": "on"})),
        (
            "voloo_set_nickname",
            json!({"device": UID_A, "nickname": "x"}),
        ),
    ] {
        let r = rig.mcp(name, args);
        assert_eq!(r["isError"], true);
        assert!(
            r["structuredContent"]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("locked")
        );
    }
    assert_eq!(
        rig.mcp("voloo_get_status", json!({"device": UID_A}))["isError"],
        false
    );
    assert!(rig.sims[0].state.lock().unwrap().writes.is_empty());
    assert_eq!(rig.sims[0].state.lock().unwrap().nickname_writes, 0);
}

#[test]
fn mcp_lost_ack_and_unknown_outcomes_never_resend() {
    let rig = Rig::new("idle_poll_ms = 60000\n", 600);
    rig.ready();
    rig.sims[0].state.lock().unwrap().inject.drop_control_ack = 1;
    let r = rig.mcp("voloo_power", json!({"device": UID_A, "action": "on"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["outcome"], "completed");
    assert_eq!(r["structuredContent"]["acknowledged"], false);
    assert_eq!(rig.sims[0].state.lock().unwrap().writes, vec![1]);
    rig.sims[1].state.lock().unwrap().inject.ignore_control = true;
    let r = rig.mcp("voloo_power", json!({"device": UID_B, "action": "on"}));
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["error"]["code"], "unknown");
    assert_eq!(rig.sims[1].state.lock().unwrap().writes, vec![1]);
    // Follow the error instruction: read status rather than repeat the write.
    let r = rig.mcp("voloo_get_status", json!({"device": UID_B}));
    assert_eq!(r["structuredContent"]["power_state"], "OFF");
    assert_eq!(rig.sims[1].state.lock().unwrap().writes, vec![1]);
    let r = rig.mcp(
        "voloo_get_log",
        json!({"device": UID_B, "kind": "operations"}),
    );
    assert!(
        r["structuredContent"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["outcome"] == "unknown")
    );
}

#[test]
fn mcp_failed_discharge_stops_cycle_and_invalid_measurements_stay_null() {
    let rig = Rig::new("idle_poll_ms = 60000\n", 600);
    rig.ready();
    {
        let mut s = rig.sims[0].state.lock().unwrap();
        s.cfg.discharge_fall = None;
        s.cfg.discharge_limit = Duration::from_millis(100);
    }
    assert_eq!(
        rig.mcp("voloo_power", json!({"device": UID_A, "action": "on"}))["isError"],
        false
    );
    let r = rig.mcp(
        "voloo_power",
        json!({"device": UID_A, "action": "cycle", "off_ms": 0}),
    );
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["error"]["code"], "device_failed");
    assert_eq!(
        rig.sims[0].state.lock().unwrap().writes,
        vec![1, 2],
        "no ON after discharge failure"
    );
    {
        let mut s = rig.sims[1].state.lock().unwrap();
        s.inject.vin_invalid = true;
        s.inject.vout_invalid = true;
        s.inject.missing_adc_block = true;
    }
    let r = rig.mcp(
        "voloo_get_status",
        json!({"device": UID_B, "include": ["adc"]}),
    );
    let s = &r["structuredContent"];
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(s["vin_valid"], false);
    assert_eq!(s["vout_valid"], false);
    assert!(s["vin_mv"].is_null() && s["vout_mv"].is_null());
    assert_eq!(s["adc"]["supported"], false);
    assert!(
        s["faults"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| !f["suggestion"].as_str().unwrap().is_empty())
    );
    let r = rig.mcp(
        "voloo_power",
        json!({"device": UID_B, "action": "cycle", "off_ms": 0}),
    );
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(rig.sims[1].state.lock().unwrap().writes, vec![2]);
    let r = rig.mcp("voloo_get_log", json!({"device": UID_B, "kind": "faults"}));
    assert!(
        !r["structuredContent"]["records"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mcp_connection_lost_after_command_is_unknown_without_replay() {
    let rig = Rig::new("idle_poll_ms = 60000\n", 600);
    rig.ready();
    rig.sims[0].state.lock().unwrap().inject.drop_control_ack = 1;
    // Close the OS pseudo-terminal after the command arrives, before readback.
    let (state, stop) = (rig.sims[0].state.clone(), rig.sims[0].stop.clone());
    let unplug = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.lock().unwrap().writes.is_empty() {
            assert!(Instant::now() < deadline, "control write never arrived");
            std::thread::sleep(Duration::from_millis(1));
        }
        stop.store(true, Ordering::Relaxed);
    });
    let r = rig.mcp("voloo_power", json!({"device": UID_A, "action": "on"}));
    unplug.join().unwrap();
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["error"]["code"], "unknown");
    assert_eq!(rig.sims[0].state.lock().unwrap().writes, vec![1]);
    let r = rig.mcp("voloo_get_status", json!({"device": UID_A}));
    assert_eq!(
        r["isError"], true,
        "unreachable device must not appear OFF: {r}"
    );
    assert_eq!(
        rig.sims[0].state.lock().unwrap().writes,
        vec![1],
        "readback does not replay the command"
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
    assert!(page.contains("<title>voloo</title>"));
    let (status, locale) = raw_request(
        port,
        &format!("GET /ui/i18n.js HTTP/1.1\r\n{host}Connection: close\r\n\r\n"),
    );
    assert_eq!(status, 200);
    assert!(
        header(&locale, "content-type")
            .unwrap()
            .contains("javascript")
    );
    assert!(locale.contains("zh-Hans") && locale.contains("zh-Hant"));
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

fn firmware_package(version: u16) -> Value {
    let make = |slot: u8| {
        let mut app = vec![0u8; 512];
        let mut tag = b"VolooSwitch:X035:AB:1:".to_vec();
        tag.push(slot);
        tag.push(0);
        app[176..176 + tag.len()].copy_from_slice(&tag);
        app
    };
    let a = make(b'A');
    let b = make(b'B');
    let mut factory = vec![255; 58 * 1024];
    let tag = b"VolooSwitch:X035:AB:BOOT:1\0";
    factory[192..192 + tag.len()].copy_from_slice(tag);
    factory[0x2000..0x2200].copy_from_slice(&a);
    factory[0x8000..0x8200].copy_from_slice(&b);
    json!({"format":"voloo-x035-ab-v1","hardware_version":0x300,"firmware_version":version,"slot_a":a,"slot_b":b,"factory":factory})
}
#[test]
fn firmware_update_and_rollback_keep_daemon_and_other_device_available() {
    let rig = Rig::new("", 600);
    rig.sims[0].state.lock().unwrap().enable_ab();
    rig.ready();
    let pid = rig.info().unwrap().pid;
    rig.sims[0]
        .state
        .lock()
        .unwrap()
        .inject
        .firmware_settling_reads = 3;
    let file = rig.home().join("firmware.voloofw");
    std::fs::write(&file, serde_json::to_vec(&firmware_package(0x20a)).unwrap()).unwrap();
    rig.sims[0]
        .state
        .lock()
        .unwrap()
        .inject
        .firmware_write_delay = Some(Duration::from_millis(350));
    let update = rig
        .cmd(&[
            "--json",
            "firmware",
            "update",
            UID_A,
            file.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, devices) = rig.http("GET", "/v1/devices", None);
        if devices
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["uid"] == UID_A && d["firmware"]["stage"] == "uploading")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "firmware update never began: {devices}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        rig.http(
            "POST",
            &format!("/v1/devices/{UID_A}/power"),
            Some(json!({"action":"on"}))
        )
        .0,
        409
    );
    assert_eq!(
        rig.http(
            "PUT",
            &format!("/v1/devices/{UID_A}/nickname"),
            Some(json!({"nickname":"queued"}))
        )
        .0,
        409
    );
    assert_eq!(
        rig.http("GET", &format!("/v1/devices/{UID_B}"), None).0,
        200
    );
    let out = update.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["identity"]["firmware_version"], "2.10");
    assert_eq!(result["boot"]["confirmed_slot"], 1);
    assert_eq!(result["status"]["power_state"], "OFF");
    assert_eq!(rig.info().unwrap().pid, pid);
    assert!(rig.sims[0].state.lock().unwrap().writes.is_empty());
    let (code, rolled) = rig.run(&["firmware", "rollback", UID_A]);
    assert_eq!(code, 0, "{rolled}");
    assert_eq!(rolled["identity"]["firmware_version"], "2.9");
    assert_eq!(rolled["boot"]["confirmed_slot"], 0);
    assert_eq!(
        rig.http("GET", &format!("/v1/devices/{UID_A}"), None).0,
        200
    );
}
#[test]
fn firmware_ack_loss_uses_evidence_without_repeating_mutations() {
    let rig = Rig::new("", 600);
    rig.sims[0].state.lock().unwrap().enable_ab();
    rig.ready();
    for (i, action) in [1, 2, 3, 7].into_iter().enumerate() {
        let before = rig.sims[0]
            .state
            .lock()
            .unwrap()
            .firmware
            .as_ref()
            .unwrap()
            .actions
            .len();
        rig.sims[0].state.lock().unwrap().inject.drop_update_ack = Some(action);
        let bytes = serde_json::to_vec(&firmware_package(0x20a + i as u16)).unwrap();
        let (code, result) = rig.http(
            "POST",
            &format!("/v1/devices/{UID_A}/firmware"),
            Some(json!({"data":bytes})),
        );
        assert_eq!(code, 200, "action {action}: {result}");
        let state = rig.sims[0].state.lock().unwrap();
        let actions = &state.firmware.as_ref().unwrap().actions[before..];
        assert_eq!(
            actions.iter().filter(|&&n| n == action).count(),
            if action == 2 { 4 } else { 1 },
            "{actions:?}"
        );
        assert!(state.writes.is_empty());
    }
}
#[test]
fn firmware_trial_retries_only_transient_reads_before_one_confirmation() {
    for (dropped, succeeds) in [(1, true), (u32::MAX, false)] {
        let rig = Rig::new("", 600);
        {
            let mut state = rig.sims[0].state.lock().unwrap();
            state.enable_ab();
            state.inject.drop_trial_status_replies = dropped;
        }
        rig.ready();
        let data = serde_json::to_vec(&firmware_package(0x20a)).unwrap();
        let (code, result) = rig.http(
            "POST",
            &format!("/v1/devices/{UID_A}/firmware"),
            Some(json!({"data":data})),
        );
        assert_eq!(code, if succeeds { 200 } else { 502 }, "{result}");
        if succeeds {
            assert_eq!(result["status"]["power_state"], "OFF");
            assert_eq!(result["identity"]["firmware_version"], "2.10");
        } else {
            assert_eq!(
                result["error"]["details"]["update"]["outcome"],
                "rolled_back"
            );
        }
        let state = rig.sims[0].state.lock().unwrap();
        let fw = state.firmware.as_ref().unwrap();
        assert_eq!(
            fw.actions.iter().filter(|&&action| action == 7).count(),
            usize::from(succeeds)
        );
        assert!(state.inject.drop_trial_status_replies < dropped);
        assert!(state.writes.is_empty());
    }
}
#[test]
fn firmware_failed_trial_rolls_back_and_failed_upload_releases_maintenance() {
    let rig = Rig::new("", 600);
    rig.sims[0].state.lock().unwrap().enable_ab();
    rig.ready();
    let data = serde_json::to_vec(&firmware_package(0x20a)).unwrap();
    rig.sims[0].state.lock().unwrap().inject.fail_firmware_trial = true;
    let (code, result) = rig.http(
        "POST",
        &format!("/v1/devices/{UID_A}/firmware"),
        Some(json!({"data":data})),
    );
    assert_eq!(code, 502, "{result}");
    assert_eq!(
        result["error"]["details"]["update"]["outcome"],
        "rolled_back"
    );
    let (status_code, status) = rig.run(&["status", UID_A]);
    assert_eq!(status_code, 0, "post-rollback status: {status}");
    assert_eq!(status["power_state"], "OFF", "{status}");
    {
        let mut state = rig.sims[0].state.lock().unwrap();
        state.inject.fail_firmware_trial = false;
        state.inject.fail_firmware_write = true;
    }
    let data = serde_json::to_vec(&firmware_package(0x20a)).unwrap();
    let (code, result) = rig.http(
        "POST",
        &format!("/v1/devices/{UID_A}/firmware"),
        Some(json!({"data":data})),
    );
    assert_eq!(code, 502, "{result}");
    let state = rig.sims[0].state.lock().unwrap();
    assert_eq!(state.firmware.as_ref().unwrap().confirmed, 0);
    assert_eq!(state.firmware.as_ref().unwrap().upload_slot, None);
    assert!(state.writes.is_empty());
}

#[test]
fn idle_shutdown_waits_for_cycle_activity_and_requires_authentication() {
    let rig = Rig::new("idle_poll_ms = 60000\n", 600);
    rig.ready();
    let info = rig.info().unwrap();
    let unauthenticated = raw_request(
        info.port,
        &format!(
            "POST /v1/shutdown/idle HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            info.port
        ),
    );
    assert_eq!(unauthenticated.0, 401, "{unauthenticated:?}");

    let (status, _) = rig.http(
        "POST",
        &format!("/v1/devices/{UID_A}/power"),
        Some(json!({"action":"on"})),
    );
    assert_eq!(status, 200);

    let cycle_client = Client {
        info,
        source: "http",
    };
    let cycle_path = format!("/v1/devices/{UID_A}/cycle");
    let cycle_body = json!({"off_ms": 1500, "discharge": false});
    let cycle = std::thread::spawn(move || {
        cycle_client
            .send(
                "POST",
                &cycle_path,
                Some(&cycle_body),
                Duration::from_secs(30),
            )
            .unwrap()
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while rig.sims[0].state.lock().unwrap().writes.len() < 2 {
        assert!(Instant::now() < deadline, "cycle never reached OFF wait");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(rig.sims[0].state.lock().unwrap().writes, vec![1, 0]);

    // The cached status stays idle while the device waits between OFF and ON.
    let (status, devices) = rig.http("GET", "/v1/devices", None);
    assert_eq!(status, 200);
    let target = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["uid"] == UID_A)
        .unwrap();
    assert!(!target["status"]["busy"].as_bool().unwrap(), "{target}");
    let (status, response) = rig.http("POST", "/v1/shutdown/idle", None);
    assert_eq!(status, 409, "{response}");
    assert_eq!(rig.sims[0].state.lock().unwrap().writes, vec![1, 0]);

    let (cycle_status, cycle_response) = cycle.join().unwrap();
    assert_eq!(cycle_status, 200, "{cycle_response}");
    assert_eq!(rig.sims[0].state.lock().unwrap().writes, vec![1, 0, 1]);

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let (status, response) = rig.http("POST", "/v1/shutdown/idle", None);
        if status == 200 {
            assert!(response["idle_verified"].as_bool().unwrap(), "{response}");
            break;
        }
        assert_eq!(status, 409, "{response}");
        assert!(
            Instant::now() < deadline,
            "activity did not release: {response}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A request body that arrives after the daemon has answered must not turn the reply into
/// a TCP reset: the rollback handler used to ignore its body, so macOS reported
/// "Connection reset by peer" even though the rollback had completed.
#[test]
fn rollback_reply_survives_a_late_request_body() {
    // No A/B support on the simulated device: the handler answers at once with an error,
    // which is exactly when an unread body used to reset the connection.
    let rig = Rig::new("", 600);
    rig.ready();
    let info = rig.info().unwrap();
    for delay_ms in [20u64, 50, 100, 200, 20, 50, 100, 200] {
        let mut s = TcpStream::connect(("127.0.0.1", info.port)).unwrap();
        let head = format!(
            "POST /v1/devices/{UID_A}/firmware/rollback HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n",
            info.port, info.token
        );
        s.write_all(head.as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(delay_ms));
        let _ = s.write_all(b"{}");
        let mut out = String::new();
        s.read_to_string(&mut out)
            .unwrap_or_else(|e| panic!("reply lost after {delay_ms} ms body delay: {e}"));
        assert!(
            out.starts_with("HTTP/1.1 200") || out.contains(" 4"),
            "{out}"
        );
    }
}

/// Serves `body` as the firmware manifest on a loopback port for the life of the test.
fn serve_manifest(body: String) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/manifest.json",
        listener.local_addr().unwrap().port()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        }
    });
    url
}
fn newer_manifest(rig: &Rig) -> (String, String) {
    let (_, info) = rig.run(&["info", UID_A]);
    let hardware = info["hardware_version"].as_str().unwrap().to_string();
    let (a, b) = info["firmware_version"]
        .as_str()
        .unwrap()
        .split_once('.')
        .unwrap();
    let next = format!("{a}.{}", b.parse::<u16>().unwrap() + 1);
    let code = (a.parse::<u16>().unwrap() << 8) | (b.parse::<u16>().unwrap() + 1);
    let manifest = json!({"schema":1,"product":"voloo-switch-x035","host":{"latest":"99.0.0"},"firmware":[{
        "version": next, "code": code, "sha256": "0".repeat(64), "size": 1,
        "urls": ["https://voloo.librezoo.com/firmware/x035/x.voloofw"],
        "hardware": [hardware], "min_host": "0.0.1"}]});
    (manifest.to_string(), next)
}

#[test]
fn background_update_check_marks_devices_that_could_be_updated() {
    let mut rig = Rig::new("check_updates = true\n", 600);
    rig.ready();
    let (manifest, next) = newer_manifest(&rig);
    let url = serve_manifest(manifest);
    rig.run(&["daemon", "stop"]);
    rig.env.push(("VOLOO_FIRMWARE_MANIFEST_URL".into(), url));
    rig.env
        .push(("VOLOO_UPDATE_CHECK_DELAY_MS".into(), "50".into()));
    rig.run(&["ls"]);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (_, rows) = rig.http("GET", "/v1/devices", None);
        let row = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["uid"] == UID_A)
            .unwrap()
            .clone();
        if row["firmware_update_available"] == json!(next) {
            assert!(
                rig.home().join("update-check.json").exists(),
                "answer is cached for a day"
            );
            return;
        }
        assert!(Instant::now() < deadline, "no update marker: {row}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn update_check_is_off_unless_scanning_usb_or_enabled() {
    let mut rig = Rig::new("", 600);
    rig.ready();
    let (manifest, _) = newer_manifest(&rig);
    let url = serve_manifest(manifest);
    rig.run(&["daemon", "stop"]);
    rig.env.push(("VOLOO_FIRMWARE_MANIFEST_URL".into(), url));
    rig.env
        .push(("VOLOO_UPDATE_CHECK_DELAY_MS".into(), "50".into()));
    rig.run(&["ls"]);
    std::thread::sleep(Duration::from_millis(1500));
    let (_, rows) = rig.http("GET", "/v1/devices", None);
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|d| d.get("firmware_update_available").is_none())
    );
    assert!(!rig.home().join("update-check.json").exists());
}

#[test]
fn firmware_update_without_a_device_lists_plan_and_refuses_to_guess_without_all() {
    let mut rig = Rig::new("", 600);
    rig.ready();
    let (manifest, _) = newer_manifest(&rig);
    let url = serve_manifest(manifest);
    rig.run(&["daemon", "stop"]);
    rig.env.push(("VOLOO_FIRMWARE_MANIFEST_URL".into(), url));
    rig.ready();
    // Two devices have an update, nobody can be asked (no terminal): exit 4 and nothing is written.
    let out = rig.cmd(&["firmware", "update"]).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(4), "{err}");
    assert!(err.contains("--all"), "{err}");
    assert!(
        err.contains(UID_A) && err.contains(UID_B),
        "plan lists both devices: {err}"
    );
    // --all cannot be combined with a device.
    let out = rig
        .cmd(&["firmware", "update", "--all", UID_A])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
}

#[test]
fn firmware_update_all_skips_devices_without_an_update() {
    let mut rig = Rig::new("", 600);
    rig.ready();
    // Manifest announces nothing newer than the simulated devices run.
    let (_, info) = rig.run(&["info", UID_A]);
    let hardware = info["hardware_version"].as_str().unwrap().to_string();
    let (a, b) = info["firmware_version"]
        .as_str()
        .unwrap()
        .split_once('.')
        .unwrap();
    let code = (a.parse::<u16>().unwrap() << 8) | b.parse::<u16>().unwrap();
    let manifest = json!({"schema":1,"product":"voloo-switch-x035","firmware":[{
        "version": info["firmware_version"], "code": code, "sha256": "0".repeat(64), "size": 1,
        "urls": ["https://voloo.librezoo.com/firmware/x035/x.voloofw"],
        "hardware": [hardware], "min_host": "0.0.1"}]});
    let url = serve_manifest(manifest.to_string());
    rig.run(&["daemon", "stop"]);
    rig.env.push(("VOLOO_FIRMWARE_MANIFEST_URL".into(), url));
    rig.ready();
    let (code, result) = rig.run(&["firmware", "update", "--all"]);
    assert_eq!(code, 0, "{result}");
    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|r| r["outcome"] == "skipped" && r["reason"] == "already up to date"),
        "{result}"
    );
}
