//! Daily, quiet look at the public firmware manifest so devices that could be updated can
//! be marked (`firmware_update_available`). At most one small GET a day, cached in
//! `$VOLOO_HOME/update-check.json`, failures are silent, nothing about the user or the
//! devices is sent, and nothing is ever installed automatically.
use crate::{firmware, home, manager::Manager};
use serde::{Deserialize, Serialize};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_AGE_S: u64 = 24 * 60 * 60;

#[derive(Serialize, Deserialize)]
struct Cache {
    checked_at: u64,
    manifest: firmware::Manifest,
}
fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
fn cache_path() -> Option<std::path::PathBuf> {
    home::dir().map(|d| d.join("update-check.json"))
}
fn load() -> Option<Cache> {
    serde_json::from_slice(&std::fs::read(cache_path()?).ok()?).ok()
}
fn save(cache: &Cache) {
    if let Some(path) = cache_path()
        && let Ok(bytes) = serde_json::to_vec(cache)
    {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}
/// Start-up delay: long enough not to compete with device discovery, jittered so a fleet
/// of machines does not hit the server at the same moment.
fn initial_delay() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("VOLOO_UPDATE_CHECK_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        return Duration::from_millis(ms);
    }
    let jitter = u64::from(std::process::id() % 120);
    Duration::from_secs(45 + jitter)
}

/// Reads the manifest now (blocking), stores it and caches it. Used by the daily thread and
/// by `voloo firmware update` with several devices so its plan uses current information.
pub fn refresh_now(manager: &Manager) -> bool {
    match firmware::fetch_manifest_for_check() {
        Some(manifest) => {
            manager.set_update_manifest(manifest.clone());
            save(&Cache {
                checked_at: now_s(),
                manifest,
            });
            true
        }
        None => false,
    }
}

/// Runs in a detached thread for the life of the background process.
pub fn spawn(manager: Arc<Manager>) {
    if !manager.config.update_check {
        return;
    }
    std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            if let Some(cache) = load() {
                manager.set_update_manifest(cache.manifest);
            }
            std::thread::sleep(initial_delay());
            loop {
                let stale =
                    load().is_none_or(|c| now_s().saturating_sub(c.checked_at) >= MAX_AGE_S);
                if stale {
                    if refresh_now(&manager) {
                    } else if let Some(mut cache) = load() {
                        // Offline: keep the cached answer, retry tomorrow rather than hourly.
                        cache.checked_at = now_s();
                        save(&cache);
                    }
                }
                std::thread::sleep(Duration::from_secs(60 * 60));
            }
        })
        .ok();
}

fn notified_path() -> Option<std::path::PathBuf> {
    home::dir().map(|d| d.join("update-notified.json"))
}
const UPGRADE: &str = "npm install -g @librezoo/voloo@latest";

/// Pure part of [`notices`]: `seen` maps a device UID (or "host") to the last version
/// already announced, and is updated for every line returned.
fn build_notices(
    rows: &serde_json::Value,
    host_latest: Option<&str>,
    only_uid: Option<&str>,
    seen: &mut std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut host_mentioned = false;
    for row in rows.as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let Some(uid) = row["uid"].as_str() else {
            continue;
        };
        if only_uid.is_some_and(|only| only != uid) {
            continue;
        }
        if let Some(version) = row["firmware_update_available"].as_str() {
            if seen.get(uid).is_none_or(|v| v != version) {
                lines.push(format!(
                    "Firmware {version} is available for {uid} (running {}): voloo firmware update {uid}",
                    row["identity"]["firmware_version"].as_str().unwrap_or("?"),
                ));
                seen.insert(uid.to_string(), version.to_string());
            }
        } else if let (Some(version), Some(min_host)) = (
            row["firmware_update_needs_host"]["version"].as_str(),
            row["firmware_update_needs_host"]["min_host"].as_str(),
        ) {
            let key = format!("{version}+{min_host}");
            if seen.get(uid).is_none_or(|v| *v != key) {
                lines.push(format!(
                    "Firmware {version} for {uid} needs voloo {min_host} or newer (you have {}): {UPGRADE}",
                    env!("CARGO_PKG_VERSION"),
                ));
                seen.insert(uid.to_string(), key);
                host_mentioned = true;
            }
        }
    }
    if let Some(latest) = host_latest
        && !host_mentioned
        && seen.get("host").is_none_or(|v| v != latest)
    {
        lines.push(format!(
            "voloo {latest} is available (you have {}): {UPGRADE}",
            env!("CARGO_PKG_VERSION"),
        ));
        seen.insert("host".into(), latest.into());
    } else if let Some(latest) = host_latest
        && host_mentioned
    {
        seen.insert("host".into(), latest.into());
    }
    lines
}

/// One-line notices (devices that could be updated, a newer voloo), each device/version
/// only ever once; later commands stay quiet until something newer appears.
pub fn notices(
    rows: &serde_json::Value,
    host_latest: Option<&str>,
    only_uid: Option<&str>,
) -> Vec<String> {
    let mut seen: std::collections::BTreeMap<String, String> = notified_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let before = seen.clone();
    let lines = build_notices(rows, host_latest, only_uid, &mut seen);
    if seen != before
        && let Some(path) = notified_path()
        && let Ok(bytes) = serde_json::to_vec(&seen)
    {
        let _ = std::fs::write(path, bytes);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn each_firmware_and_host_version_is_announced_once() {
        let rows = json!([{"uid":"A","identity":{"firmware_version":"3.3"},"firmware_update_available":"3.4"}]);
        let mut seen = Default::default();
        let first = build_notices(&rows, Some("99.0.0"), None, &mut seen);
        assert_eq!(first.len(), 2, "{first:?}");
        assert!(build_notices(&rows, Some("99.0.0"), None, &mut seen).is_empty());
        let newer = json!([{"uid":"A","identity":{"firmware_version":"3.3"},"firmware_update_available":"3.5"}]);
        assert_eq!(
            build_notices(&newer, Some("99.0.0"), None, &mut seen).len(),
            1
        );
    }
    #[test]
    fn firmware_blocked_by_host_is_one_combined_line() {
        let rows =
            json!([{"uid":"A","firmware_update_needs_host":{"version":"3.4","min_host":"0.8.0"}}]);
        let mut seen = Default::default();
        let lines = build_notices(&rows, Some("0.8.0"), None, &mut seen);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("needs voloo 0.8.0"));
        assert!(build_notices(&rows, Some("0.8.0"), None, &mut seen).is_empty());
    }
    #[test]
    fn single_device_commands_only_mention_that_device() {
        let rows = json!([
            {"uid":"A","firmware_update_available":"3.4"},
            {"uid":"B","firmware_update_available":"3.4"}]);
        let mut seen = Default::default();
        let lines = build_notices(&rows, None, Some("B"), &mut seen);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains(" B "));
    }
}
