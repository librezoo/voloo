//! End-to-end CLI runs against a simulated device on a PTY (`--port`), isolated VOLOO_HOME.
#![cfg(unix)]
use serde_json::Value;
use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use voloo::sim::{Shared, SimConfig, serve_tty, shared};

struct Rig {
    path: String,
    uid: String,
    home: tempfile::TempDir,
    state: Shared,
    stop: Arc<AtomicBool>,
    server: Option<std::thread::JoinHandle<()>>,
}
impl Rig {
    fn new(cfg: SimConfig) -> Rig {
        let (master, slave) = serialport::TTYPort::pair().unwrap();
        let path = serialport::SerialPort::name(&slave).unwrap();
        drop(slave);
        let uid = cfg.uid.to_string();
        let state = shared(cfg);
        let stop = Arc::new(AtomicBool::new(false));
        let server = {
            let (s, st) = (state.clone(), stop.clone());
            std::thread::spawn(move || serve_tty(master, s, st))
        };
        Rig {
            path,
            uid,
            home: tempfile::tempdir().unwrap(),
            state,
            stop,
            server: Some(server),
        }
    }
    fn run(&self, args: &[&str]) -> (i32, Value, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_voloo"))
            .env("VOLOO_HOME", self.home.path())
            .args(["--json", "--port", &self.path])
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        (
            out.status.code().unwrap(),
            serde_json::from_str(&stdout).unwrap_or(Value::Null),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(s) = self.server.take() {
            let _ = s.join();
        }
    }
}

#[test]
fn power_commands_logs_and_exit_codes() {
    let rig = Rig::new(SimConfig::default());
    let uid = rig.uid.as_str();
    let (code, v, _) = rig.run(&["status", uid]);
    assert_eq!(code, 0);
    assert_eq!(v["power_state"], "OFF");
    assert_eq!(v["vin_mv"], 7500);

    let (code, v, _) = rig.run(&["on", uid]);
    assert_eq!((code, v["outcome"].as_str()), (0, Some("completed")));
    let (code, v, _) = rig.run(&["on", uid]);
    assert_eq!(
        (code, v["outcome"].as_str()),
        (0, Some("already_satisfied"))
    );
    let (code, v, _) = rig.run(&["cycle", uid, "--off-ms", "0"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["on"]["status"]["power_state"], "ON");
    assert_eq!(rig.state.lock().unwrap().writes, vec![1, 2, 1]);

    let (code, v, _) = rig.run(&["log", "--kind", "operations"]);
    assert_eq!(code, 0);
    let actions: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["action"].as_str())
        .collect();
    assert_eq!(actions, ["on", "on", "discharge", "on", "cycle"]);
    assert!(v.as_array().unwrap().iter().all(|r| r["source"] == "cli"));

    let (code, v, _) = rig.run(&["name", uid, "bench-a"]);
    assert_eq!(code, 5, "nickname refused while ON: {v}");
    assert_eq!(v["error"]["code"], "busy");
}

#[test]
fn device_failure_is_exit_2_with_fault_and_trace() {
    let rig = Rig::new(SimConfig {
        discharge_fall: None,
        discharge_limit: Duration::from_millis(150),
        ..SimConfig::default()
    });
    let uid = rig.uid.as_str();
    let (code, v, _) = rig.run(&["discharge", uid]);
    assert_eq!(code, 2, "{v}");
    assert_eq!(v["error"]["code"], "device_failed");
    let faults: Vec<&str> = v["error"]["faults"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["code"].as_str())
        .collect();
    assert!(faults.contains(&"DEV_DISCHARGE_TIMEOUT"), "{faults:?}");
    let trace_id = v["error"]["trace_id"].as_str().unwrap().to_string();

    let (code, spans, _) = rig.run(&["trace", &trace_id]);
    assert_eq!(code, 0);
    assert!(
        spans
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["label"] == "control discharge")
    );

    let (code, v, _) = rig.run(&["faults"]);
    assert_eq!(code, 0);
    assert!(
        v.as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "DEV_DISCHARGE_TIMEOUT" && f["event"] == "fault.raised")
    );
}

#[test]
fn unknown_outcome_is_exit_3() {
    let rig = Rig::new(SimConfig::default());
    rig.state.lock().unwrap().inject.ignore_control = true;
    let (code, v, _) = rig.run(&["on", &rig.uid]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["error"]["code"], "unknown");
    assert_eq!(rig.state.lock().unwrap().writes, vec![1], "never resent");
}

#[test]
fn restart_between_runs_is_detected() {
    let rig = Rig::new(SimConfig::default());
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(rig.run(&["status", &rig.uid]).0, 0);
    rig.state.lock().unwrap().reset(4);
    let (code, v, _) = rig.run(&["status", &rig.uid]);
    assert_eq!(code, 0);
    let f = v["faults"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["code"] == "DEV_RESET_DETECTED")
        .expect("restart fault");
    assert!(f["message"].as_str().unwrap().contains("WATCHDOG"));
}

#[test]
fn usage_errors_are_exit_4() {
    let rig = Rig::new(SimConfig::default());
    assert_eq!(rig.run(&["cycle", &rig.uid, "--off-ms", "40000"]).0, 4);
    assert_eq!(
        rig.run(&["on", "bench-a"]).0,
        4,
        "--port needs a UID target"
    );
    assert_eq!(rig.run(&["frobnicate"]).0, 4);
    let out = Command::new(env!("CARGO_BIN_EXE_voloo"))
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
}
