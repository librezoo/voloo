//! `voloo report`: one reviewable JSON file with what support needs to diagnose a problem.
//! It is written locally and never sent anywhere; the user decides whether to share it.
//! Excluded: the access token, host name and user name; home paths become `~`; serial
//! numbers of non-Voloo USB devices are removed.
use crate::{discover, home, observe::read_log};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

pub const FORMAT: &str = "voloo-report/1";

/// Device data gathered by the caller (through the background process or directly).
pub struct Gathered {
    pub devices: Vec<Value>,
    pub background_process: Value,
    /// Spans held in memory by the running background process (newest included); when
    /// present they replace the on-disk trace tail, which may lag behind.
    pub live_trace: Option<Vec<Value>>,
}

fn os_version() -> Option<String> {
    let out = if cfg!(target_os = "macos") {
        std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()?
    } else if cfg!(unix) {
        std::process::Command::new("uname")
            .arg("-r")
            .output()
            .ok()?
    } else {
        std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .output()
            .ok()?
    };
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

fn tail(path: &std::path::Path, lines: usize) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub fn build(gathered: Gathered) -> Value {
    let logs = home::logs();
    let usb: Vec<Value> = discover::list_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let mut v = json!(p);
            if !p.is_voloo {
                v["serial_number"] = Value::Null; // not ours to collect
            }
            v
        })
        .collect();
    let config = home::dir().and_then(|d| fs::read_to_string(d.join("config.toml")).ok());
    let env: Vec<Value> = std::env::vars()
        .filter(|(k, _)| k.starts_with("VOLOO_"))
        .map(|(k, v)| json!({"name": k, "value": if k.contains("TOKEN") { "<redacted>".into() } else { v }}))
        .collect();
    let log = |stem: &str, n: usize| {
        logs.as_deref()
            .map(|d| read_log(d, stem, None, n))
            .unwrap_or_default()
    };
    json!({
        "manifest": {
            "format": FORMAT,
            "created_at": crate::observe::now(),
            "voloo_version": env!("CARGO_PKG_VERSION"),
            "sections": ["system", "usb", "background_process", "devices", "faults", "operations", "trace", "daemon_log", "config", "environment"],
            "privacy": "Written locally only. No access token, host name or user name; home paths shown as ~; serial numbers of non-Voloo USB devices removed. Device UIDs are included because support needs them. Review before sharing.",
        },
        "system": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH, "os_version": os_version()},
        "usb": usb,
        "background_process": gathered.background_process,
        "devices": gathered.devices,
        "faults": log("events", 500),
        "operations": log("operations", 200),
        "trace": gathered.live_trace.unwrap_or_else(|| log("trace", 1000)),
        "daemon_log": logs.as_deref().map(|d| tail(&d.join("daemon.log"), 200)).unwrap_or_default(),
        "config": config,
        "environment": env,
    })
}

/// Replaces the user's home directory in every string so paths do not reveal the user name.
pub fn redact(value: &mut Value) {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    fn walk(v: &mut Value, home: &str) {
        match v {
            Value::String(s) if !home.is_empty() && s.contains(home) => *s = s.replace(home, "~"),
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, home)),
            Value::Object(o) => o.values_mut().for_each(|x| walk(x, home)),
            _ => {}
        }
    }
    if home.len() > 1 {
        walk(value, &home);
    }
}

/// Writes the report under `$VOLOO_HOME/reports/` (mode 0600) and returns its path.
pub fn write(mut report: Value) -> std::io::Result<PathBuf> {
    redact(&mut report);
    let dir = home::dir()
        .ok_or_else(|| std::io::Error::other("no home directory; set VOLOO_HOME"))?
        .join("reports");
    fs::create_dir_all(&dir)?;
    let stamp: String = crate::observe::now()
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(14)
        .collect();
    let path = dir.join(format!("voloo-report-{stamp}.json"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(
        &mut options.open(&path)?,
        serde_json::to_string_pretty(&report).unwrap().as_bytes(),
    )?;
    Ok(path)
}
