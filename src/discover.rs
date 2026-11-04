//! USB enumeration and target resolution. The USB serial number carries the UID; the
//! identity register is always read back before a port is treated as that device.
use crate::{
    device::{Device, ErrorKind, OpError},
    link::{SerialLink, Transport},
    observe::{FaultCode, Recorder, new_trace_id},
    protocol::Uid,
};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

/// Development VID/PID from pid.codes; replace with the assigned pair before distribution.
pub const KNOWN_IDS: [(u16, u16); 1] = [(0x1209, 0x0001)];
pub const PRODUCT: &str = "Voloo Switch";

#[derive(Debug, Clone, Serialize)]
pub struct PortInfo {
    pub path: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
    pub product: Option<String>,
    pub manufacturer: Option<String>,
    /// UID parsed from the USB serial number, when it has that form.
    pub uid: Option<Uid>,
    pub is_voloo: bool,
}

pub fn list_ports() -> Result<Vec<PortInfo>, OpError> {
    let ports = serialport::available_ports().map_err(|e| {
        OpError::new(
            ErrorKind::LinkError,
            format!("cannot enumerate serial ports: {e}"),
        )
    })?;
    let mut out: Vec<PortInfo> = ports
        .into_iter()
        .map(|p| match p.port_type {
            serialport::SerialPortType::UsbPort(u) => {
                let product_match = u.product.as_deref().is_some_and(|s| s.contains(PRODUCT));
                PortInfo {
                    uid: u.serial_number.as_deref().and_then(|s| s.parse().ok()),
                    is_voloo: product_match || KNOWN_IDS.contains(&(u.vid, u.pid)),
                    path: p.port_name,
                    vid: Some(u.vid),
                    pid: Some(u.pid),
                    serial_number: u.serial_number,
                    product: u.product,
                    manufacturer: u.manufacturer,
                }
            }
            _ => PortInfo {
                path: p.port_name,
                vid: None,
                pid: None,
                serial_number: None,
                product: None,
                manufacturer: None,
                uid: None,
                is_voloo: false,
            },
        })
        .collect();
    // macOS lists each device as /dev/tty.* and /dev/cu.*; the callout device does not wait for DCD.
    let cu: Vec<String> = out
        .iter()
        .filter(|p| p.path.starts_with("/dev/cu."))
        .map(|p| p.path.clone())
        .collect();
    out.retain(|p| {
        !(p.path.starts_with("/dev/tty.")
            && cu.contains(&p.path.replacen("/dev/tty.", "/dev/cu.", 1)))
    });
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Opens `path` and confirms the identity register. `expected` comes from the USB serial number
/// or the user's request; a mismatch is refused.
pub fn open(
    path: &str,
    expected: Option<Uid>,
    recorder: &Arc<Recorder>,
) -> Result<Device, OpError> {
    if let Some(sim) = crate::sim::demo_link(path) {
        return attach(
            Transport::new(Box::new(sim), recorder.clone()),
            expected,
            recorder,
        );
    }
    let link = SerialLink::open(path).map_err(|e| {
        let code = match e {
            crate::link::LinkError::Busy(_) => FaultCode::LinkPortBusy,
            crate::link::LinkError::Gone(_) => FaultCode::LinkUnplugged,
            _ => FaultCode::LinkIo,
        };
        recorder.raise(
            code,
            expected.map(|u| u.to_string()),
            Some(path.into()),
            e.to_string(),
            None,
            Value::Null,
        );
        let kind = if code == FaultCode::LinkPortBusy {
            ErrorKind::Busy
        } else {
            ErrorKind::LinkError
        };
        OpError::new(kind, e.to_string())
    })?;
    attach(Transport::new(link, recorder.clone()), expected, recorder)
}

/// Reads identity over an already-open transport and returns the bound device.
pub fn attach(
    transport: Transport,
    expected: Option<Uid>,
    recorder: &Arc<Recorder>,
) -> Result<Device, OpError> {
    // Identity reads need a target UID; without one, use the broadcast-free identity read
    // addressed to the expected UID. Point-to-point USB has exactly one device per port.
    let Some(uid) = expected else {
        return Err(OpError::new(
            ErrorKind::NotFound,
            format!(
                "{}: USB serial number does not carry a UID; pass the full UID",
                transport.path()
            ),
        ));
    };
    let mut device = Device::new(transport, uid, recorder.clone());
    device.identity(&new_trace_id())?;
    Ok(device)
}

/// Resolves a user target (UID, nickname or `None` for "the only one") to an open device.
pub fn resolve(
    target: Option<&str>,
    port: Option<&str>,
    recorder: &Arc<Recorder>,
) -> Result<Device, OpError> {
    if let Some(path) = port {
        let uid = match target {
            Some(t) => Some(t.parse::<Uid>().map_err(|_| {
                OpError::new(
                    ErrorKind::InvalidArgument,
                    "with --port, the target must be a full UID (or omit it)",
                )
            })?),
            None => list_ports()?
                .into_iter()
                .find(|p| p.path == path)
                .and_then(|p| p.uid),
        };
        return open(path, uid, recorder);
    }
    let ports: Vec<PortInfo> = list_ports()?.into_iter().filter(|p| p.is_voloo).collect();
    if ports.is_empty() {
        return Err(OpError::new(
            ErrorKind::NotFound,
            "No Voloo found. Connect it with a USB data cable and run `voloo ports`.",
        ));
    }
    match target {
        None => match ports.as_slice() {
            [only] => open(&only.path, only.uid, recorder),
            _ => Err(OpError::new(
                ErrorKind::InvalidArgument,
                format!(
                    "{} Voloo devices are connected; name one by UID or nickname",
                    ports.len()
                ),
            )),
        },
        Some(t) => {
            if let Ok(uid) = t.parse::<Uid>() {
                let port = ports.iter().find(|p| p.uid == Some(uid)).ok_or_else(|| {
                    OpError::new(ErrorKind::NotFound, format!("Voloo {uid} is not connected"))
                })?;
                return open(&port.path, Some(uid), recorder);
            }
            let mut matches = Vec::new();
            for p in &ports {
                let Ok(mut device) = open(&p.path, p.uid, recorder) else {
                    continue;
                };
                if device.nickname(&new_trace_id()).ok().as_deref() == Some(t) {
                    matches.push(device);
                }
            }
            match matches.len() {
                0 => Err(OpError::new(
                    ErrorKind::NotFound,
                    format!("No connected Voloo is named {t:?}"),
                )),
                1 => Ok(matches.pop().unwrap()),
                n => Err(OpError::new(
                    ErrorKind::InvalidArgument,
                    format!("{n} devices are named {t:?}; use the UID"),
                )),
            }
        }
    }
}
