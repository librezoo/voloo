#![allow(clippy::result_large_err)] // see lib.rs
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};
use voloo::{
    client::{Client, SendError},
    config::Config,
    daemon,
    device::{Device, Diagnostics, ErrorKind, OpError},
    discover, home,
    observe::{Level, Limits, Recorder, new_trace_id, read_log, rfc3339},
    protocol::{Command, Uid},
};

/// Control USB-C Voloo Switch devices connected to this computer.
///
/// Voloo switches externally supplied SOURCE power; it is not a power source.
/// OFF does not mean the load terminals are at zero volts.
#[derive(Parser)]
#[command(name = "voloo", version, about, long_about = None)]
struct Cli {
    /// Print machine-readable JSON on stdout.
    #[arg(long, global = true)]
    json: bool,
    /// Open the serial port directly instead of going through the background process.
    #[arg(long, global = true)]
    direct: bool,
    /// Use this serial port directly (implies --direct; for debugging).
    #[arg(long, global = true)]
    port: Option<String>,
    /// Log level for direct mode: info (default), debug (adds raw frames) or trace.
    #[arg(long, global = true, env = "VOLOO_LOG_LEVEL", default_value = "info")]
    log_level: Level,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Update firmware over USB, inspect A/B slots, or roll back to the previous image.
    Firmware {
        #[command(subcommand)]
        action: FirmwareCmd,
    },
    /// List serial ports with USB metadata, without opening them.
    Ports,
    /// List connected Voloo devices with their state.
    Ls,
    /// Read fresh state (DEVICE: UID or nickname; optional with one device).
    Status { device: Option<String> },
    /// Identity, versions, capabilities and nickname.
    Info { device: Option<String> },
    /// Diagnostic registers.
    Diag {
        device: Option<String>,
        #[arg(long, default_value = "communication", value_parser = ["communication", "adc", "fast"])]
        kind: String,
    },
    /// Turn the output ON and confirm it.
    On { device: String },
    /// Turn the output OFF (does not discharge the load).
    Off { device: String },
    /// Turn OFF and actively discharge until VOUT <= 1 V is confirmed.
    Discharge { device: String },
    /// OFF (discharge by default), wait, then ON. Stops at the first failure.
    Cycle {
        device: String,
        #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(0..=30000))]
        off_ms: u64,
        /// Plain OFF instead of a confirmed discharge.
        #[arg(long)]
        no_discharge: bool,
    },
    /// Set (or clear with "") the persistent nickname. Requires the output fully OFF.
    Name { device: String, nickname: String },
    /// Faults from the event log: active conditions and events of the last 24 h by default.
    Faults {
        device: Option<String>,
        /// Include cleared conditions and older events.
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Recent records from the logs.
    Log {
        #[arg(long, default_value = "operations", value_parser = ["operations", "faults", "trace"])]
        kind: String,
        /// Filter by UID.
        #[arg(long)]
        uid: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// All transaction spans of one operation.
    Trace { trace_id: String },
    /// Check USB, permissions, background process, devices and logs, with fixes.
    Doctor,
    /// Write a diagnostic report (one JSON file) to review and share with support.
    /// Read-only; nothing is sent anywhere.
    Report,
    /// Open the web dashboard (all connected Voloo devices, live).
    Ui {
        /// Print the one-time login URL instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Refuse every power or nickname change to a device, from any client.
    Lock { device: String },
    /// Lift a lock set with `voloo lock` or from the web page.
    Unlock { device: String },
    /// MCP server on stdio for AI agents (relays to the background process).
    Mcp,
    /// Try the dashboard with simulated devices: separate home, no hardware touched.
    Demo {
        #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=8))]
        count: u8,
        /// Print the one-time link instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Simulated devices on pseudo-terminals (exercises the real serial code path).
    #[command(hide = true)]
    DevSim {
        #[arg(long, default_value_t = 3)]
        count: u8,
    },
    /// Local API address and token for other programs (starts the background process).
    ApiInfo,
    /// Manage the background process that owns the serial ports.
    Daemon {
        #[command(subcommand)]
        action: DaemonCmd,
    },
}

#[derive(Subcommand)]
enum FirmwareCmd {
    /// Upload a local .voloofw package (or legacy USB .bin); verify and confirm it.
    Update {
        device: String,
        file: std::path::PathBuf,
    },
    /// Trial-boot the previous A/B firmware; verify it before confirming rollback.
    Rollback { device: String },
    /// Read USB update support, current slot, previous firmware and trial state.
    Status { device: String },
}

#[derive(Subcommand)]
enum DaemonCmd {
    /// Run in the foreground (clients normally start it automatically).
    Run,
    /// Show whether it is running.
    Status,
    /// Stop it. Device outputs are left unchanged.
    Stop,
}

enum Backend {
    Direct {
        recorder: Arc<Recorder>,
        port: Option<String>,
    },
    Daemon,
}

struct Ctx {
    json: bool,
    backend: Backend,
}

fn main() -> ExitCode {
    // Usage errors exit 4 (invalid argument); 2 is reserved for device-reported failure.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() {
                ErrorKind::InvalidArgument.exit_code() as u8
            } else {
                0
            });
        }
    };
    match cli.command {
        Cmd::Daemon {
            action: DaemonCmd::Run,
        } => {
            return match daemon::run() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("voloo daemon: {e}");
                    ExitCode::from(3)
                }
            };
        }
        Cmd::Mcp => return mcp_stdio(),
        Cmd::DevSim { count } => return dev_sim(count),
        Cmd::Demo { count, no_open } => {
            // A separate home keeps demo logs, locks and the demo background process apart
            // from the real one; USB scanning is off so no real device can be touched.
            let home = std::env::temp_dir().join(format!("voloo-demo-{count}"));
            // SAFETY: single-threaded at this point; the variables are inherited by the
            // background process this command starts.
            unsafe {
                std::env::set_var("VOLOO_HOME", &home);
                std::env::set_var("VOLOO_DEMO", count.to_string());
                std::env::set_var("VOLOO_SCAN_USB", "0");
                std::env::remove_var("VOLOO_PORTS");
            }
            let ctx = Ctx {
                json: cli.json,
                backend: Backend::Daemon,
            };
            return match run(&ctx, Cmd::Ui { no_open }) {
                Ok(_) => {
                    if !cli.json {
                        println!(
                            "Demo devices are simulated; logs are in {}. Stop with: VOLOO_HOME={} voloo daemon stop",
                            home.display(),
                            home.display()
                        );
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {}", e.message);
                    ExitCode::from(e.code.exit_code() as u8)
                }
            };
        }
        _ => {}
    }
    let backend = if cli.direct || cli.port.is_some() {
        let recorder = Arc::new(
            home::logs()
                .and_then(|dir| Recorder::with_dir(cli.log_level, dir, Limits::default()).ok())
                .unwrap_or_else(|| Recorder::memory(cli.log_level)),
        );
        Backend::Direct {
            recorder,
            port: cli.port,
        }
    } else {
        Backend::Daemon
    };
    let ctx = Ctx {
        json: cli.json,
        backend,
    };
    let result = run(&ctx, cli.command);
    if let Backend::Direct { recorder, .. } = &ctx.backend {
        recorder.flush();
    }
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            if ctx.json {
                println!("{}", json!({ "error": e }));
            } else {
                eprintln!(
                    "error[{}]: {}",
                    json!(e.code).as_str().unwrap_or("error"),
                    e.message
                );
                if let Some(t) = &e.trace_id {
                    eprintln!("  trace: voloo trace {t}");
                }
                for f in &e.faults {
                    eprintln!("  ! {}: {}", f.code.as_str(), f.suggestion);
                }
            }
            ExitCode::from(e.code.exit_code() as u8)
        }
    }
}

fn emit(ctx: &Ctx, value: &Value, human: impl FnOnce() -> String) {
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(value).unwrap());
    } else {
        println!("{}", human());
    }
}

fn daemon_client() -> Result<Client, OpError> {
    Client::connect("cli")
}
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Runs a device job directly or through the background process; both return the same JSON.
enum Op {
    Status,
    Info,
    Diag(String),
    Power(Command),
    Cycle(u64, bool),
    Name(String),
}
fn device_op(ctx: &Ctx, target: Option<&str>, op: Op) -> Result<Value, OpError> {
    match &ctx.backend {
        Backend::Direct { recorder, port } => {
            // Configured ports ([[ports]] / VOLOO_PORTS) are found by UID like USB devices.
            let configured = target.and_then(|t| t.parse::<Uid>().ok()).and_then(|uid| {
                Config::load()
                    .ok()?
                    .static_ports
                    .into_iter()
                    .find(|(_, u)| *u == uid)
                    .map(|(p, _)| p)
            });
            let port = port.clone().or(configured);
            let mut d = discover::resolve(target, port.as_deref(), recorder)?;
            let attach = |d: &Device, mut e: OpError| {
                e.faults = d.relevant_faults();
                e
            };
            let v = match op {
                Op::Status => {
                    check_restart(&mut d);
                    d.status().map(|r| json!(r))
                }
                Op::Info => d.info().map(|r| json!(r)),
                Op::Diag(kind) => d.diagnostics(&kind).map(|r| json!(r)),
                Op::Power(c) => d.set_power(c).map(|r| json!(r)),
                Op::Cycle(ms, dis) => d.cycle(ms, dis).map(|r| json!(r)),
                Op::Name(n) => d.set_nickname(&n).map(|r| json!(r)),
            };
            v.map_err(|e| attach(&d, e))
        }
        Backend::Daemon => {
            let c = daemon_client()?;
            let id = enc(target.unwrap_or("-"));
            match op {
                Op::Status => c.call("GET", &format!("/v1/devices/{id}"), None),
                Op::Info => c.call("GET", &format!("/v1/devices/{id}/info"), None),
                Op::Diag(kind) => c.call(
                    "GET",
                    &format!("/v1/devices/{id}/diagnostics?kind={}", enc(&kind)),
                    None,
                ),
                Op::Power(cmd) => c.call(
                    "POST",
                    &format!("/v1/devices/{id}/power"),
                    Some(&json!({"action": cmd.name()})),
                ),
                Op::Cycle(ms, dis) => c.call(
                    "POST",
                    &format!("/v1/devices/{id}/cycle"),
                    Some(&json!({"off_ms": ms, "discharge": dis})),
                ),
                Op::Name(n) => c.call(
                    "PUT",
                    &format!("/v1/devices/{id}/nickname"),
                    Some(&json!({"nickname": n})),
                ),
            }
        }
    }
}

/// Direct mode: compare communication counters with the last run to record restarts.
fn check_restart(device: &mut Device) {
    if let Ok(report) = device.diagnostics("communication")
        && let Some(Diagnostics::Communication(c)) = &report.data
    {
        device.check_restart(&report.trace_id, home::load_comm(device.uid).as_ref(), c);
        home::save_comm(device.uid, c);
    }
}

fn volts(mv: &Value) -> String {
    mv.as_u64()
        .map_or("invalid".into(), |v| format!("{:.3} V", v as f64 / 1000.0))
}
fn describe(s: &Value) -> String {
    let mut state = s["power_state"].as_str().unwrap_or("?").to_string();
    if s["busy"] == true {
        state.push_str(" (busy)");
    }
    format!(
        "{state:<12} VIN {:<10} VOUT {:<10} last: {}",
        volts(&s["vin_mv"]),
        volts(&s["vout_mv"]),
        s["last_result"].as_str().unwrap_or("?")
    )
}
fn fault_lines(faults: &Value) -> String {
    faults
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let tag = if f["active"] == true { "" } else { " (event)" };
            format!(
                "\n  ! {}{tag}: {}\n    {}",
                f["code"].as_str().unwrap_or(""),
                f["message"].as_str().unwrap_or(""),
                f["suggestion"].as_str().unwrap_or("")
            )
        })
        .collect()
}

fn run(ctx: &Ctx, cmd: Cmd) -> Result<u8, OpError> {
    match cmd {
        Cmd::Firmware { action } => {
            if matches!(ctx.backend, Backend::Direct { .. }) {
                return Err(OpError::new(
                    ErrorKind::InvalidArgument,
                    "Firmware maintenance is managed by the background process; omit --direct/--port so it can release and reconnect the selected device safely",
                ));
            }
            let client = daemon_client()?;
            let (device, method, suffix, body) = match action {
                FirmwareCmd::Status { device } => (device, "GET", "", None),
                FirmwareCmd::Rollback { device } => (device, "POST", "/rollback", Some(json!({}))),
                FirmwareCmd::Update { device, file } => {
                    use std::io::Read;
                    let source = std::fs::File::open(&file).map_err(|e| {
                        OpError::new(
                            ErrorKind::InvalidArgument,
                            format!("Cannot read firmware {}: {e}", file.display()),
                        )
                    })?;
                    let mut data = Vec::new();
                    source
                        .take(voloo::firmware::MAX_FILE as u64 + 1)
                        .read_to_end(&mut data)
                        .map_err(|e| OpError::new(ErrorKind::InvalidArgument, e.to_string()))?;
                    voloo::firmware::Image::parse(&data)?;
                    (device, "POST", "", Some(json!({"data":data})))
                }
            };
            if !ctx.json && method == "POST" {
                eprintln!(
                    "Firmware maintenance for {device}: the background process will reconnect and verify the device. Keep USB connected."
                );
            }
            let value = client.call(
                method,
                &format!("/v1/devices/{}/firmware{suffix}", enc(&device)),
                body.as_ref(),
            )?;
            emit(ctx, &value, || {
                if method == "GET" {
                    serde_json::to_string_pretty(&value).unwrap()
                } else {
                    format!(
                        "{} firmware maintenance completed: {} · output OFF · connection verified",
                        value["uid"].as_str().unwrap_or(""),
                        value["identity"]["firmware_version"].as_str().unwrap_or("")
                    )
                }
            });
        }
        Cmd::Ports => {
            let ports = json!(discover::list_ports()?);
            emit(ctx, &ports, || {
                ports
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| {
                        let ids = p["vid"].as_u64().map_or(String::new(), |v| {
                            format!(" {v:04x}:{:04x}", p["pid"].as_u64().unwrap_or(0))
                        });
                        let mark = if p["is_voloo"] == true {
                            " [voloo]"
                        } else {
                            ""
                        };
                        format!(
                            "{}{ids} {}{mark}",
                            p["path"].as_str().unwrap_or(""),
                            p["product"].as_str().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        Cmd::Ls => {
            let rows = list_devices(ctx)?;
            emit(ctx, &rows, || {
                let rows = rows.as_array().unwrap();
                if rows.is_empty() {
                    return "No Voloo connected. Check the USB data cable, then run `voloo doctor`.".into();
                }
                rows.iter()
                    .map(|r| {
                        let name = r["nickname"]
                            .as_str()
                            .filter(|n| !n.is_empty())
                            .map(|n| format!(" ({n})"))
                            .unwrap_or_default();
                        let head = format!(
                            "{}{name}  {}",
                            r["uid"].as_str().unwrap_or("?"),
                            r["port"].as_str().unwrap_or("")
                        );
                        if r["online"] != true {
                            let why = r["error"]["message"].as_str().unwrap_or("not reachable");
                            return format!("{head}\n  OFFLINE: output state unknown ({why})");
                        }
                        format!(
                            "{head}\n  {}{}",
                            describe(&r["status"]),
                            fault_lines(&r["faults"])
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        Cmd::Status { device } => {
            let r = device_op(ctx, device.as_deref(), Op::Status)?;
            emit(ctx, &r, || {
                format!(
                    "{}  {}\n  {}{}",
                    r["uid"].as_str().unwrap_or(""),
                    r["port"].as_str().unwrap_or(""),
                    describe(&r),
                    fault_lines(&r["faults"])
                )
            });
        }
        Cmd::Info { device } => {
            let r = device_op(ctx, device.as_deref(), Op::Info)?;
            emit(ctx, &r, || {
                format!(
                    "{}  {}\n  nickname: {}\n  firmware {}  protocol {}  hardware {}  capabilities 0x{:04X}",
                    r["uid"].as_str().unwrap_or(""),
                    r["port"].as_str().unwrap_or(""),
                    r["nickname"]
                        .as_str()
                        .filter(|n| !n.is_empty())
                        .unwrap_or("-"),
                    r["firmware_version"].as_str().unwrap_or("?"),
                    r["protocol_version"].as_str().unwrap_or("?"),
                    r["hardware_version"].as_str().unwrap_or("?"),
                    r["capabilities"].as_u64().unwrap_or(0)
                )
            });
        }
        Cmd::Diag { device, kind } => {
            let r = device_op(ctx, device.as_deref(), Op::Diag(kind))?;
            emit(ctx, &r, || serde_json::to_string_pretty(&r).unwrap());
        }
        Cmd::On { device } => power(ctx, &device, Command::On)?,
        Cmd::Off { device } => power(ctx, &device, Command::Off)?,
        Cmd::Discharge { device } => power(ctx, &device, Command::Discharge)?,
        Cmd::Cycle {
            device,
            off_ms,
            no_discharge,
        } => {
            let r = device_op(ctx, Some(&device), Op::Cycle(off_ms, !no_discharge))?;
            emit(ctx, &r, || {
                format!(
                    "{}  cycle done: {} -> {} ms -> ON\n  {}",
                    r["uid"].as_str().unwrap_or(""),
                    r["off"]["action"].as_str().unwrap_or("off"),
                    r["off_ms"],
                    describe(&r["on"]["status"])
                )
            });
        }
        Cmd::Name { device, nickname } => {
            let r = device_op(ctx, Some(&device), Op::Name(nickname))?;
            emit(ctx, &r, || {
                format!(
                    "{}  nickname set to {:?} (verified)",
                    r["uid"].as_str().unwrap_or(""),
                    r["nickname"].as_str().unwrap_or("")
                )
            });
        }
        Cmd::Faults { device, all, limit } => faults(ctx, device, all, limit)?,
        Cmd::Log { kind, uid, limit } => {
            let dir = home::logs()
                .ok_or_else(|| OpError::new(ErrorKind::NotFound, "No Voloo home directory"))?;
            let stem = if kind == "faults" {
                "events"
            } else {
                kind.as_str()
            };
            let uid = uid.map(|u| u.parse::<Uid>().map(|u| u.to_string()).unwrap_or(u));
            let records = json!(read_log(&dir, stem, uid.as_deref(), limit));
            emit(ctx, &records, || {
                records
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        Cmd::Trace { trace_id } => trace(ctx, &trace_id)?,
        Cmd::Doctor => return doctor(ctx),
        Cmd::Report => {
            let gathered = gather_for_report(ctx)?;
            let n = gathered.devices.len();
            let path = voloo::report::write(voloo::report::build(gathered)).map_err(|e| {
                OpError::new(
                    ErrorKind::LinkError,
                    format!("cannot write the report: {e}"),
                )
            })?;
            let v = json!({"path": path, "devices": n, "format": voloo::report::FORMAT});
            emit(ctx, &v, || {
                format!(
                    "Report written: {}\n{n} device(s), faults, recent operations and traces, logs and settings.\nNothing was sent. Open it to check what it contains before sharing it with support.",
                    path.display()
                )
            });
        }
        Cmd::Ui { no_open } => {
            let c = daemon_client()?;
            let v = c.call("POST", "/v1/ui/code", Some(&json!({})))?;
            let url = v["url"].as_str().unwrap_or_default().to_string();
            let opened = !no_open && open_browser(&url);
            emit(
                ctx,
                &json!({"url": url, "opened": opened, "expires_s": v["expires_s"]}),
                || {
                    if opened {
                        "Opened the Voloo dashboard in your browser.".into()
                    } else {
                        format!("Open this one-time link within 60 s:\n{url}")
                    }
                },
            );
        }
        Cmd::Lock { device } | Cmd::Unlock { device }
            if matches!(ctx.backend, Backend::Direct { .. }) =>
        {
            let _ = device;
            return Err(OpError::new(
                ErrorKind::InvalidArgument,
                "Locks are kept by the background process; omit --direct/--port",
            ));
        }
        Cmd::Lock { device } => {
            let v = daemon_client()?.call(
                "POST",
                &format!("/v1/devices/{}/lock", enc(&device)),
                Some(&json!({"locked": true})),
            )?;
            emit(ctx, &v, || {
                format!(
                    "{} locked: no client can switch or rename it until `voloo unlock`",
                    v["uid"].as_str().unwrap_or("")
                )
            });
        }
        Cmd::Unlock { device } => {
            let v = daemon_client()?.call(
                "POST",
                &format!("/v1/devices/{}/lock", enc(&device)),
                Some(&json!({"locked": false})),
            )?;
            emit(ctx, &v, || {
                format!("{} unlocked", v["uid"].as_str().unwrap_or(""))
            });
        }
        Cmd::ApiInfo => {
            let c = daemon_client()?;
            let url = format!("http://127.0.0.1:{}", c.info.port);
            let info = json!({
                "url": url, "token": c.info.token, "pid": c.info.pid, "version": c.info.version,
                "openapi": format!("{url}/openapi.json"), "mcp": format!("{url}/mcp"),
                "auth_header": format!("Authorization: Bearer {}", c.info.token),
                "info_file": daemon::info_path(),
            });
            emit(ctx, &info, || {
                format!(
                    "url:    {url}\ntoken:  {}\nopenapi: {url}/openapi.json\nmcp:    {url}/mcp (Streamable HTTP)\n\ncurl -H 'Authorization: Bearer {}' {url}/v1/devices\n\nThe token changes whenever the background process restarts; read it from {} at startup.",
                    c.info.token,
                    c.info.token,
                    daemon::info_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                )
            });
        }
        Cmd::Daemon { action } => match action {
            DaemonCmd::Status => {
                let v = match Client::existing("cli") {
                    Some(c) => {
                        let (_, health) = c
                            .send("GET", "/v1/health", None, Duration::from_secs(2))
                            .map_err(|_| OpError::new(ErrorKind::LinkError, "unreachable"))?;
                        json!({"running": true, "port": c.info.port, "health": health})
                    }
                    None => json!({"running": false}),
                };
                emit(ctx, &v, || {
                    if v["running"] == true {
                        format!(
                            "running: pid {} port {} uptime {} s, {} device(s)",
                            v["health"]["pid"],
                            v["port"],
                            v["health"]["uptime_s"],
                            v["health"]["devices"]
                        )
                    } else {
                        "not running (it starts automatically when needed)".into()
                    }
                });
            }
            DaemonCmd::Stop => {
                let v = match Client::existing("cli") {
                    Some(c) => {
                        c.call("POST", "/v1/shutdown", Some(&json!({})))?;
                        // Return once it has released the ports, so the next command can
                        // start a new one right away.
                        let exited = voloo::daemon::wait_exited(Duration::from_secs(10));
                        json!({"stopped": true, "exited": exited})
                    }
                    None => json!({"stopped": false, "note": "not running"}),
                };
                emit(ctx, &v, || {
                    if v["stopped"] == true {
                        "stopping; device outputs are unchanged".into()
                    } else {
                        "not running".into()
                    }
                });
            }
            DaemonCmd::Run => unreachable!(),
        },
        Cmd::Mcp | Cmd::DevSim { .. } | Cmd::Demo { .. } => unreachable!(),
    }
    Ok(0)
}

fn power(ctx: &Ctx, target: &str, command: Command) -> Result<(), OpError> {
    let r = device_op(ctx, Some(target), Op::Power(command))?;
    emit(ctx, &r, || {
        let what = match r["outcome"].as_str() {
            Some("already_satisfied") => format!("already {}", command.name()),
            _ => format!("{} completed in {} ms", command.name(), r["elapsed_ms"]),
        };
        let ack = if r["acknowledged"] == false {
            " (ACK lost; confirmed by status)"
        } else {
            ""
        };
        let mut out = format!(
            "{}  {what}{ack}\n  {}",
            r["uid"].as_str().unwrap_or(""),
            describe(&r["status"])
        );
        if command == Command::Off {
            out.push_str("\n  note: OFF does not discharge the load; use `voloo discharge` to confirm VOUT <= 1 V");
        }
        out.push_str(&fault_lines(&r["faults"]));
        out
    });
    Ok(())
}

/// Same row shape in both modes: uid, port, online, nickname, status, faults, error.
fn list_devices(ctx: &Ctx) -> Result<Value, OpError> {
    match &ctx.backend {
        Backend::Daemon => {
            let c = daemon_client()?;
            let mut devices = c.call("GET", "/v1/devices", None)?;
            // A freshly started process may not have finished its first scan.
            if devices.as_array().is_some_and(Vec::is_empty) {
                std::thread::sleep(Duration::from_millis(1500));
                devices = c.call("GET", "/v1/devices", None)?;
            }
            Ok(devices)
        }
        Backend::Direct { recorder, port } => {
            let config = Config::load().map_err(|e| OpError::new(ErrorKind::InvalidArgument, e))?;
            let mut ports: Vec<(String, Option<Uid>)> = if config.scan_usb {
                discover::list_ports()?
                    .into_iter()
                    .filter(|p| p.is_voloo)
                    .map(|p| (p.path, p.uid))
                    .collect()
            } else {
                Vec::new()
            };
            ports.extend(
                config
                    .static_ports
                    .iter()
                    .map(|(p, u)| (p.clone(), Some(*u))),
            );
            if let Some(p) = port {
                ports = vec![(p.clone(), None)];
            }
            let mut rows = Vec::new();
            for (path, uid) in ports {
                let mut row =
                    json!({"uid": uid.map(|u| u.to_string()), "port": path, "online": false});
                match discover::open(&path, uid, recorder) {
                    Ok(mut d) => {
                        check_restart(&mut d);
                        row["nickname"] = json!(d.nickname(&new_trace_id()).ok());
                        match d.status() {
                            Ok(s) => {
                                row["online"] = json!(true);
                                row["faults"] = json!(s.faults);
                                row["status"] = json!(s.status);
                            }
                            Err(e) => row["error"] = json!(e),
                        }
                    }
                    Err(e) => row["error"] = json!(e),
                }
                rows.push(row);
            }
            Ok(json!(rows))
        }
    }
}

fn faults(ctx: &Ctx, device: Option<String>, all: bool, limit: usize) -> Result<(), OpError> {
    let dir =
        home::logs().ok_or_else(|| OpError::new(ErrorKind::NotFound, "No Voloo home directory"))?;
    let uid = device.map(|d| d.parse::<Uid>().map(|u| u.to_string()).unwrap_or(d));
    // Latest event per (device, code) decides whether it is still active.
    let mut latest: BTreeMap<(String, String), Value> = BTreeMap::new();
    for e in read_log(&dir, "events", uid.as_deref(), 10_000) {
        let key = (
            e["uid"]
                .as_str()
                .or(e["port"].as_str())
                .unwrap_or("")
                .to_string(),
            e["code"].as_str().unwrap_or("").to_string(),
        );
        latest.insert(key, e);
    }
    let since = rfc3339(std::time::SystemTime::now() - Duration::from_secs(86_400));
    let mut list: Vec<Value> = latest
        .into_values()
        .filter(|e| {
            all || e["event"] == "fault.raised"
                || (e["event"] == "fault.occurred"
                    && e["last_seen"].as_str().is_some_and(|t| t >= since.as_str()))
        })
        .collect();
    list.sort_by(|a, b| a["last_seen"].as_str().cmp(&b["last_seen"].as_str()));
    let skip = list.len().saturating_sub(limit);
    let list = json!(list.into_iter().skip(skip).collect::<Vec<_>>());
    emit(ctx, &list, || {
        let items = list.as_array().unwrap();
        if items.is_empty() {
            return "No active faults or recent events recorded.".into();
        }
        items
            .iter()
            .map(|f| {
                let state = match f["event"].as_str() {
                    Some("fault.raised") => "active",
                    Some("fault.occurred") => "occurred",
                    _ => "cleared",
                };
                format!(
                    "{}  {}  {}  [{state}]\n  {}\n  {}",
                    f["last_seen"].as_str().unwrap_or(""),
                    f["uid"].as_str().or(f["port"].as_str()).unwrap_or("-"),
                    f["code"].as_str().unwrap_or(""),
                    f["message"].as_str().unwrap_or(""),
                    f["suggestion"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(())
}

fn trace(ctx: &Ctx, trace_id: &str) -> Result<(), OpError> {
    // Spans of a running background process may not be flushed to disk yet.
    let mut spans: Vec<Value> = match (&ctx.backend, Client::existing("cli")) {
        (Backend::Daemon, Some(c)) => c
            .call(
                "GET",
                &format!("/v1/trace?trace_id={}&limit=10000", enc(trace_id)),
                None,
            )?
            .as_array()
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    if spans.is_empty() {
        let dir = home::logs()
            .ok_or_else(|| OpError::new(ErrorKind::NotFound, "No Voloo home directory"))?;
        spans = read_log(&dir, "trace", None, 100_000)
            .into_iter()
            .filter(|s| s["trace_id"] == trace_id)
            .collect();
    }
    if spans.is_empty() {
        return Err(OpError::new(
            ErrorKind::NotFound,
            format!("No spans for {trace_id}"),
        ));
    }
    let spans = json!(spans);
    emit(ctx, &spans, || {
        spans
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                format!(
                    "{}  {:<22} {:>7} us  {}{}",
                    s["at"].as_str().unwrap_or(""),
                    s["label"].as_str().unwrap_or(""),
                    s["elapsed_us"],
                    s["result"].as_str().unwrap_or(""),
                    s["detail"]
                        .as_str()
                        .map(|d| format!(" ({d})"))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(())
}

/// Read-only health check with a fix for every problem. Exit 1 when something fails.
fn doctor(ctx: &Ctx) -> Result<u8, OpError> {
    let mut checks: Vec<Value> = Vec::new();
    let mut add = |name: &str, level: &str, detail: String, fix: Option<&str>| {
        checks.push(json!({"check": name, "level": level, "detail": detail, "fix": fix}));
    };
    match Config::load() {
        Ok(_) => add(
            "config",
            "ok",
            home::dir()
                .map(|d| d.join("config.toml").display().to_string())
                .unwrap_or_default(),
            None,
        ),
        Err(e) => add("config", "fail", e, Some("Fix or remove the config file.")),
    }
    match home::logs().map(|d| {
        std::fs::create_dir_all(&d)
            .and_then(|_| std::fs::write(d.join(".doctor"), b"ok"))
            .map(|_| d)
    }) {
        Some(Ok(d)) => {
            let _ = std::fs::remove_file(d.join(".doctor"));
            add("logs", "ok", d.display().to_string(), None);
        }
        Some(Err(e)) => add(
            "logs",
            "fail",
            e.to_string(),
            Some("Make the Voloo home directory writable or set VOLOO_HOME."),
        ),
        None => add(
            "logs",
            "fail",
            "no home directory".into(),
            Some("Set VOLOO_HOME."),
        ),
    }
    let config = Config::load().unwrap_or_default();
    let ports = if config.scan_usb {
        discover::list_ports()?
    } else {
        Vec::new()
    };
    if !config.scan_usb {
        add(
            "usb_scan",
            "warn",
            "USB scanning is off (scan_usb = false or VOLOO_SCAN_USB=0)".into(),
            Some("Remove the setting to find Voloo devices automatically."),
        );
    }
    let voloo: Vec<_> = ports.iter().filter(|p| p.is_voloo).collect();
    let configured: Vec<String> = config.static_ports.iter().map(|(p, _)| p.clone()).collect();
    if !configured.is_empty() {
        add("configured_ports", "ok", configured.join(", "), None);
    }
    if voloo.is_empty() && configured.is_empty() {
        add(
            "usb",
            "fail",
            format!("no Voloo among {} serial port(s)", ports.len()),
            Some(
                "Connect the Voloo with a USB data cable (charge-only cables have no data lines) and check `voloo ports`.",
            ),
        );
    } else {
        add(
            "usb",
            "ok",
            format!(
                "{} Voloo port(s): {}",
                voloo.len(),
                voloo
                    .iter()
                    .map(|p| p.path.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None,
        );
    }
    for p in &voloo {
        if p.vid == Some(0x1209) && p.pid == Some(0x0001) {
            add(
                "usb_id",
                "warn",
                format!("{} uses the development VID:PID 1209:0001", p.path),
                Some("Firmware must use an assigned VID/PID before distribution."),
            );
        }
        if p.uid.is_none() {
            add(
                "usb_serial",
                "warn",
                format!(
                    "{}: USB serial number {:?} is not a UID",
                    p.path, p.serial_number
                ),
                Some("Update the Switch firmware; the UID is expected in the USB serial number."),
            );
        }
        #[cfg(unix)]
        {
            let c = std::ffi::CString::new(p.path.as_str()).unwrap();
            if unsafe { libc::access(c.as_ptr(), libc::R_OK | libc::W_OK) } != 0 {
                add(
                    "permission",
                    "fail",
                    format!("no read/write access to {}", p.path),
                    Some("On Linux add yourself to the dialout (or uucp) group and log in again."),
                );
            }
        }
    }
    let daemon = Client::existing("cli");
    add(
        "background_process",
        "ok",
        match &daemon {
            Some(c) => format!(
                "running (pid {}, port {}, version {})",
                c.info.pid, c.info.port, c.info.version
            ),
            None => "not running; it starts automatically when needed".into(),
        },
        None,
    );
    if let Some(c) = &daemon
        && c.info.version != env!("CARGO_PKG_VERSION")
    {
        add(
            "version",
            "warn",
            format!(
                "background process {} vs CLI {}",
                c.info.version,
                env!("CARGO_PKG_VERSION")
            ),
            Some("Run `voloo daemon stop`; the new version starts on next use."),
        );
    }
    // Device reads go through the running process (it owns the ports) or open them directly.
    let rows = if daemon.is_some() {
        list_devices(&Ctx {
            json: true,
            backend: Backend::Daemon,
        })?
    } else {
        let recorder = Arc::new(Recorder::memory(Level::Info));
        list_devices(&Ctx {
            json: true,
            backend: Backend::Direct {
                recorder,
                port: None,
            },
        })?
    };
    for r in rows.as_array().into_iter().flatten() {
        let uid = r["uid"].as_str().unwrap_or("?");
        if r["online"] != true {
            let why = r["error"]["message"]
                .as_str()
                .unwrap_or("not reachable")
                .to_string();
            let fix = if r["error"]["code"] == "busy" {
                "Another program holds the port (for example a Web Serial page or another tool). Close it."
            } else {
                "Reconnect the USB cable, then run `voloo status`."
            };
            add("device", "fail", format!("{uid}: {why}"), Some(fix));
            continue;
        }
        let s = &r["status"];
        add("device", "ok", format!("{uid}: {}", describe(s)), None);
        if s["vin_valid"] == false || s["vout_valid"] == false {
            add(
                "measurement",
                "warn",
                format!(
                    "{uid}: VIN valid={} VOUT valid={}",
                    s["vin_valid"], s["vout_valid"]
                ),
                Some(
                    "Invalid VIN/VOUT is not 0 V. Check SOURCE wiring and the PWR_GND/BUS_GND connection (NT100 on the 2026-11-03 board).",
                ),
            );
        }
        for f in r["faults"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|f| f["active"] == true)
        {
            add(
                "fault",
                "warn",
                format!(
                    "{uid}: {} {}",
                    f["code"].as_str().unwrap_or(""),
                    f["message"].as_str().unwrap_or("")
                ),
                f["suggestion"].as_str(),
            );
        }
    }
    let failed = checks.iter().any(|c| c["level"] == "fail");
    let report = json!({"ok": !failed, "checks": checks});
    emit(ctx, &report, || {
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                let mark = match c["level"].as_str() {
                    Some("ok") => "ok  ",
                    Some("warn") => "warn",
                    _ => "FAIL",
                };
                let fix = c["fix"]
                    .as_str()
                    .map(|f| format!("\n       fix: {f}"))
                    .unwrap_or_default();
                format!(
                    "[{mark}] {:<18} {}{fix}",
                    c["check"].as_str().unwrap_or(""),
                    c["detail"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(if failed { 1 } else { 0 })
}

/// stdio MCP: one JSON-RPC message per line in, one response per line out. Nothing else is
/// written to stdout. The background process is (re)started transparently.
fn mcp_stdio() -> ExitCode {
    let mut client: Option<Client> = None;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(_) => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}})
                );
                let _ = stdout.flush();
                continue;
            }
        };
        let id = message.get("id").cloned();
        let mut reply = None;
        for attempt in 0..2 {
            if client.is_none() {
                match Client::connect("mcp") {
                    Ok(c) => client = Some(c),
                    Err(e) => {
                        reply = id.clone().map(|id| json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": e.message}}));
                        break;
                    }
                }
            }
            match client.as_ref().unwrap().send(
                "POST",
                "/mcp",
                Some(&message),
                Duration::from_secs(120),
            ) {
                Ok((202, _)) => break,
                Ok((_, v)) if v.get("jsonrpc").is_some() => {
                    reply = Some(v);
                    break;
                }
                Ok((status, v)) => {
                    // 401 after a restart of the background process: reconnect once.
                    client = None;
                    if attempt == 1 || status != 401 {
                        reply = id.clone().map(|id| json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": format!("HTTP {status}: {v}")}}));
                        break;
                    }
                }
                Err(SendError::NotConnected(_)) if attempt == 0 => client = None,
                Err(e) => {
                    client = None;
                    let text = match e {
                        SendError::Broken(e) => format!(
                            "Lost the connection to the voloo background process ({e}); the outcome of a power action is unknown, read status before acting"
                        ),
                        SendError::NotConnected(e) => {
                            format!("Cannot reach the voloo background process: {e}")
                        }
                    };
                    reply = id.clone().map(|id| json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": text}}));
                    break;
                }
            }
        }
        if let Some(r) = reply {
            let _ = writeln!(stdout, "{r}");
            let _ = stdout.flush();
        }
    }
    ExitCode::SUCCESS
}

fn open_browser(url: &str) -> bool {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Serves simulated devices on pseudo-terminals until interrupted, and prints how to use them.
#[cfg(unix)]
fn dev_sim(count: u8) -> ExitCode {
    use std::sync::atomic::AtomicBool;
    use voloo::sim::{SimConfig, serve_tty, shared};
    let names = [
        "bench-a", "camera", "router", "", "dut-5", "dut-6", "dut-7", "dut-8",
    ];
    let mut ports = Vec::new();
    let mut threads = Vec::new();
    let mut slaves = Vec::new();
    let stop = Arc::new(AtomicBool::new(false));
    for i in 0..count.clamp(1, 8) {
        let (master, slave) = match serialport::TTYPort::pair() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("cannot create a pseudo-terminal: {e}");
                return ExitCode::from(3);
            }
        };
        let path = serialport::SerialPort::name(&slave).unwrap_or_default();
        // Keep our slave end open for the simulator's lifetime. When the last slave
        // descriptor closes, macOS can leave the reopened pseudo-terminal passing data
        // in one direction only, which looked like a device that stopped answering.
        slaves.push(slave);
        let mut uid = [0x5Eu8, 0x11, 0, 0, 0, 0, 0, i + 1];
        uid[2] = 0xD0;
        let cfg = SimConfig {
            uid: voloo::protocol::Uid(uid),
            nickname: names[i as usize].into(),
            vin_mv: [12000, 5000, 24000, 7500, 3300, 9000, 15000, 48000][i as usize],
            discharge_fall: Some(Duration::from_millis(400)),
            ..SimConfig::default()
        };
        ports.push(format!("{path}={}", cfg.uid));
        let (state, stop) = (shared(cfg), stop.clone());
        threads.push(std::thread::spawn(move || serve_tty(master, state, stop)));
    }
    let list = ports.join(";");
    println!("Simulated Voloo devices are running. In another terminal:\n");
    println!("  export VOLOO_HOME=$(mktemp -d) VOLOO_SCAN_USB=0 VOLOO_PORTS='{list}'");
    println!("  voloo ui\n\nPress Ctrl-C to stop the simulator.");
    let _ = std::io::stdout().flush();
    for t in threads {
        let _ = t.join();
    }
    drop(slaves);
    ExitCode::SUCCESS
}
#[cfg(not(unix))]
fn dev_sim(_count: u8) -> ExitCode {
    eprintln!("The simulator needs pseudo-terminals (macOS or Linux).");
    ExitCode::from(4)
}

/// Read-only data for `voloo report`: through the background process when it runs (it owns
/// the ports), otherwise by opening each Voloo port directly.
fn gather_for_report(ctx: &Ctx) -> Result<voloo::report::Gathered, OpError> {
    let kinds = ["communication", "adc", "fast"];
    let config = Config::load().map_err(|e| OpError::new(ErrorKind::InvalidArgument, e))?;
    // Demo devices only exist inside a background process.
    let client = if !config.demo.is_empty() {
        Some(Client::connect("cli")?)
    } else {
        Client::existing("cli")
    };
    if !matches!(ctx.backend, Backend::Direct { .. })
        && let Some(c) = client
    {
        let health = c.call("GET", "/v1/health", None).unwrap_or(Value::Null);
        let metrics = c.call("GET", "/v1/metrics", None).unwrap_or(Value::Null);
        let mut devices = c
            .call("GET", "/v1/devices", None)?
            .as_array()
            .cloned()
            .unwrap_or_default();
        for d in devices.iter_mut().filter(|d| d["online"] == true) {
            let id = enc(d["uid"].as_str().unwrap_or("-"));
            d["info"] = c
                .call("GET", &format!("/v1/devices/{id}/info"), None)
                .unwrap_or_else(|e| json!({"error": e}));
            d["diagnostics"] = json!(
                kinds
                    .iter()
                    .map(|k| c
                        .call(
                            "GET",
                            &format!("/v1/devices/{id}/diagnostics?kind={k}"),
                            None
                        )
                        .unwrap_or_else(|e| json!({"error": e})))
                    .collect::<Vec<_>>()
            );
        }
        // After the device calls above, so their own spans are part of the trace.
        let live_trace = c
            .call("GET", "/v1/trace?limit=1000", None)
            .ok()
            .and_then(|v| v.as_array().cloned());
        return Ok(voloo::report::Gathered {
            devices,
            background_process: json!({"running": true, "health": health, "metrics": metrics}),
            live_trace,
        });
    }
    let recorder = Arc::new(Recorder::memory(Level::Info));
    let mut devices = Vec::new();
    // Same reach as the background process: USB only when scanning is on, plus configured ports.
    let mut ports: Vec<(String, Option<Uid>)> = if config.scan_usb {
        discover::list_ports()?
            .into_iter()
            .filter(|p| p.is_voloo)
            .map(|p| (p.path, p.uid))
            .collect()
    } else {
        Vec::new()
    };
    ports.extend(
        config
            .static_ports
            .iter()
            .map(|(p, u)| (p.clone(), Some(*u))),
    );
    for (path, uid) in ports {
        let mut row = json!({"port": path, "uid": uid.map(|u| u.to_string())});
        match discover::open(&path, uid, &recorder) {
            Ok(mut d) => {
                row["online"] = json!(true);
                row["status"] = d
                    .status()
                    .map(|r| json!(r))
                    .unwrap_or_else(|e| json!({"error": e}));
                row["info"] = d
                    .info()
                    .map(|r| json!(r))
                    .unwrap_or_else(|e| json!({"error": e}));
                row["diagnostics"] = json!(
                    kinds
                        .iter()
                        .map(|k| d
                            .diagnostics(k)
                            .map(|r| json!(r))
                            .unwrap_or_else(|e| json!({"error": e})))
                        .collect::<Vec<_>>()
                );
                row["faults"] = json!(d.relevant_faults());
            }
            Err(e) => {
                row["online"] = json!(false);
                row["error"] = json!(e);
            }
        }
        devices.push(row);
    }
    Ok(voloo::report::Gathered {
        devices,
        background_process: json!({"running": false}),
        live_trace: None,
    })
}
