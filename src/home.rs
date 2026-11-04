//! Voloo home directory (`$VOLOO_HOME`, default `~/.voloo`): logs and per-device state.
use crate::protocol::{CommDiagnostics, Uid};
use std::{fs, path::PathBuf};

pub fn dir() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("VOLOO_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(v));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".voloo"))
}
pub fn logs() -> Option<PathBuf> {
    dir().map(|d| d.join("logs"))
}

/// Last communication counters seen for a UID, used to detect restarts between runs.
#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot {
    uptime_s: u32,
    valid_request_count: u32,
    crc_error_count: u16,
    uart_error_count: u16,
    exception_response_count: u16,
    reset_reason: u16,
}
fn state_path(uid: Uid) -> Option<PathBuf> {
    dir().map(|d| {
        d.join("state")
            .join(format!("{}.json", uid.to_string().replace('-', "")))
    })
}
pub fn load_comm(uid: Uid) -> Option<CommDiagnostics> {
    let s: Snapshot = serde_json::from_slice(&fs::read(state_path(uid)?).ok()?).ok()?;
    Some(CommDiagnostics {
        uptime_s: s.uptime_s,
        valid_request_count: s.valid_request_count,
        crc_error_count: s.crc_error_count,
        uart_error_count: s.uart_error_count,
        exception_response_count: s.exception_response_count,
        reset_reason: s.reset_reason,
    })
}
pub fn save_comm(uid: Uid, c: &CommDiagnostics) {
    let Some(path) = state_path(uid) else { return };
    let snapshot = Snapshot {
        uptime_s: c.uptime_s,
        valid_request_count: c.valid_request_count,
        crc_error_count: c.crc_error_count,
        uart_error_count: c.uart_error_count,
        exception_response_count: c.exception_response_count,
        reset_reason: c.reset_reason,
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_vec(&snapshot).unwrap()).is_ok() {
        let _ = fs::rename(tmp, path);
    }
}
