//! `$VOLOO_HOME/config.toml` (all optional):
//!
//! ```toml
//! idle_exit_s = 600          # background process exits after this long without clients
//! poll_ms = 250              # status polling while someone watches (web page, SSE)
//! idle_poll_ms = 1000        # status polling otherwise (detects unplug and faults)
//! scan_usb = true            # false: use only [[ports]] below (VOLOO_SCAN_USB=0 too)
//!
//! [mcp]
//! read_only = false          # true: MCP tools cannot switch or rename
//!
//! [devices."CD-AB-05-F3-69-BE-3B-5D"]
//! locked = true              # refuse every write to this device, from any client
//!
//! [[ports]]                  # extra ports that USB enumeration does not report
//! path = "/dev/ttys004"
//! uid = "01-23-45-67-89-AB-CD-EF"
//! ```
//!
//! `VOLOO_PORTS="path=UID;path=UID"` adds ports the same way.
use crate::{home, protocol::Uid};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct File {
    idle_exit_s: Option<u64>,
    poll_ms: Option<u64>,
    idle_poll_ms: Option<u64>,
    scan_usb: Option<bool>,
    #[serde(default)]
    mcp: McpFile,
    #[serde(default)]
    devices: BTreeMap<String, DeviceFile>,
    #[serde(default)]
    ports: Vec<PortFile>,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct McpFile {
    #[serde(default)]
    read_only: bool,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct DeviceFile {
    #[serde(default)]
    locked: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PortFile {
    path: String,
    uid: String,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub idle_exit: Duration,
    pub poll: Duration,
    pub idle_poll: Duration,
    /// Enumerate USB serial ports. Off: only configured ports (tests, unusual setups).
    pub scan_usb: bool,
    pub mcp_read_only: bool,
    pub locked: BTreeSet<Uid>,
    pub static_ports: Vec<(String, Uid)>,
    /// In-process demo devices (`VOLOO_DEMO=N`, set by `voloo demo`); never real hardware.
    pub demo: Vec<(String, Uid)>,
}
impl Default for Config {
    fn default() -> Self {
        Config {
            idle_exit: Duration::from_secs(600),
            poll: Duration::from_millis(250),
            idle_poll: Duration::from_millis(1000),
            scan_usb: true,
            mcp_read_only: false,
            locked: BTreeSet::new(),
            static_ports: Vec::new(),
            demo: Vec::new(),
        }
    }
}

impl Config {
    /// Reads the config file (if any) and `VOLOO_PORTS`. Errors name the problem.
    pub fn load() -> Result<Config, String> {
        let mut config = Config::default();
        if let Some(path) = home::dir().map(|d| d.join("config.toml"))
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            let file: File =
                toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            if let Some(s) = file.idle_exit_s {
                config.idle_exit = Duration::from_secs(s);
            }
            if let Some(ms) = file.poll_ms {
                config.poll = Duration::from_millis(ms.clamp(50, 60_000));
            }
            if let Some(ms) = file.idle_poll_ms {
                config.idle_poll = Duration::from_millis(ms.clamp(100, 60_000));
            }
            if let Some(b) = file.scan_usb {
                config.scan_usb = b;
            }
            config.mcp_read_only = file.mcp.read_only;
            for (uid, d) in file.devices {
                let uid: Uid = uid
                    .parse()
                    .map_err(|_| format!("{}: bad UID {uid:?} in [devices]", path.display()))?;
                if d.locked {
                    config.locked.insert(uid);
                }
            }
            for p in file.ports {
                let uid = p
                    .uid
                    .parse()
                    .map_err(|_| format!("{}: bad UID {:?} in [[ports]]", path.display(), p.uid))?;
                config.static_ports.push((p.path, uid));
            }
        }
        if let Ok(list) = std::env::var("VOLOO_PORTS") {
            for item in list.split(';').filter(|s| !s.trim().is_empty()) {
                let (path, uid) = item
                    .rsplit_once('=')
                    .ok_or_else(|| format!("VOLOO_PORTS item {item:?} must be path=UID"))?;
                let uid = uid
                    .parse()
                    .map_err(|_| format!("VOLOO_PORTS: bad UID {uid:?}"))?;
                config.static_ports.push((path.to_string(), uid));
            }
        }
        if let Some(n) = std::env::var("VOLOO_DEMO")
            .ok()
            .and_then(|n| n.parse::<u8>().ok())
            .filter(|n| *n > 0)
        {
            config.demo = crate::sim::demo_devices(n);
            config.scan_usb = false;
        }
        if let Ok(v) = std::env::var("VOLOO_SCAN_USB") {
            config.scan_usb = !matches!(v.as_str(), "0" | "false" | "no" | "off");
        }
        if let Some(s) = std::env::var("VOLOO_IDLE_EXIT_S")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            config.idle_exit = Duration::from_secs(s);
        }
        Ok(config)
    }
}
