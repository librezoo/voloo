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
                    if let Some(manifest) = firmware::fetch_manifest_for_check() {
                        manager.set_update_manifest(manifest.clone());
                        save(&Cache {
                            checked_at: now_s(),
                            manifest,
                        });
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
/// One-line notices for devices (rows of `/v1/devices`) that could be updated, each device
/// and version only ever once; later commands stay quiet until a newer version appears.
pub fn notices(rows: &serde_json::Value, only_uid: Option<&str>) -> Vec<String> {
    let Some(rows) = rows.as_array() else {
        return Vec::new();
    };
    let mut seen: std::collections::BTreeMap<String, String> = notified_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut lines = Vec::new();
    for row in rows {
        let (Some(uid), Some(version)) = (
            row["uid"].as_str(),
            row["firmware_update_available"].as_str(),
        ) else {
            continue;
        };
        if only_uid.is_some_and(|only| only != uid) || seen.get(uid).is_some_and(|v| v == version) {
            continue;
        }
        lines.push(format!(
            "Firmware {version} is available for {uid} (running {}): voloo firmware update {uid}",
            row["identity"]["firmware_version"].as_str().unwrap_or("?"),
        ));
        seen.insert(uid.to_string(), version.to_string());
    }
    if !lines.is_empty()
        && let Some(path) = notified_path()
        && let Ok(bytes) = serde_json::to_vec(&seen)
    {
        let _ = std::fs::write(path, bytes);
    }
    lines
}
