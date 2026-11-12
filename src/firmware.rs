//! USB firmware maintenance owned by the device worker. No power commands,
//! no automatic mutation retries, and no separate implementation in the UI/CLI.
use crate::{
    device::{Device, ErrorKind, OpError},
    discover,
    observe::{new_trace_id, now},
    protocol::{self, CAP_AB_UPDATE, CAP_USB_ISP, Identity, Matched, Status, Uid},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub const MAX_FILE: usize = 512 * 1024;
const SLOT_BYTES: usize = 24 * 1024;
const FACTORY_BYTES: usize = 58 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub stage: String,
    pub percent: u8,
    pub connected: bool,
    pub written: usize,
    pub total: usize,
}
impl Progress {
    fn new(stage: &str, percent: u8, connected: bool) -> Self {
        Self {
            stage: stage.into(),
            percent,
            connected,
            written: 0,
            total: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AbStatus {
    pub running_slot: u8,
    pub confirmed_slot: u8,
    pub previous_slot: Option<u8>,
    pub candidate_slot: Option<u8>,
    pub trial_state: u8,
    pub upload_slot: Option<u8>,
    pub upload_failed: bool,
    pub encrypted: bool,
    pub protection_flags: u8,
    pub versions: [u16; 2],
    pub lengths: [u16; 2],
    pub crcs: [u16; 2],
    pub received: u16,
    pub upload_length: u16,
    pub rollback_count: u32,
    pub sequence: u32,
}
impl AbStatus {
    pub fn decode(p: &[u8]) -> Result<Self, OpError> {
        if p.len() != 32
            || !matches!(p[0], 1 | 2)
            || p[1] > 1
            || p[2] > 1
            || p[5] > 2
            || p[7] > if p[0] == 2 { 15 } else { 1 }
        {
            return Err(failed("Invalid A/B status response"));
        }
        let slot = |n: u8| -> Result<Option<u8>, OpError> {
            match n {
                0 | 1 => Ok(Some(n)),
                255 => Ok(None),
                _ => Err(failed("Invalid A/B slot")),
            }
        };
        let w = |i| u16::from_be_bytes([p[i], p[i + 1]]);
        let d = |i| u32::from_be_bytes(p[i..i + 4].try_into().unwrap());
        let s = Self {
            running_slot: p[1],
            confirmed_slot: p[2],
            previous_slot: slot(p[3])?,
            candidate_slot: slot(p[4])?,
            trial_state: p[5],
            upload_slot: slot(p[6])?,
            upload_failed: p[7] & 1 != 0,
            encrypted: p[0] == 2,
            protection_flags: p[7] & 14,
            versions: [w(8), w(10)],
            lengths: [w(12), w(14)],
            crcs: [w(16), w(18)],
            received: w(20),
            upload_length: w(22),
            rollback_count: d(24),
            sequence: d(28),
        };
        if s.lengths.iter().any(|&n| n as usize > SLOT_BYTES)
            || s.received > s.upload_length
            || s.upload_length as usize > SLOT_BYTES
        {
            return Err(failed("Invalid A/B transfer bounds"));
        }
        Ok(s)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Bundle {
    pub format: String,
    pub hardware_version: u16,
    pub firmware_version: u16,
    pub slot_a: Vec<u8>,
    pub slot_b: Vec<u8>,
    pub factory: Vec<u8>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Recipient {
    pub uid: String,
    pub nonce: Vec<u8>,
    pub wrapped_key: Vec<u8>,
    pub tag: Vec<u8>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct SealedBundle {
    pub format: String,
    pub hardware_version: u16,
    pub firmware_version: u16,
    pub header: Vec<u8>,
    pub cipher_a: Vec<u8>,
    pub cipher_b: Vec<u8>,
    pub recipients: Vec<Recipient>,
}
impl SealedBundle {
    fn word(&self, offset: usize) -> u16 {
        u16::from_be_bytes([self.header[offset], self.header[offset + 1]])
    }
    fn validate(&self) -> Result<(), OpError> {
        if self.format != "voloo-x035-sealed-v1"
            || self.hardware_version != 0x300
            || self.firmware_version < 0x300
            || self.header.len() != 48
            || &self.header[..4] != b"VSE1"
            || self.word(4) != self.hardware_version
            || self.word(6) != self.firmware_version
        {
            return Err(invalid("Invalid encrypted X035 firmware header"));
        }
        if self.header[16..32] == self.header[32..48] {
            return Err(invalid("Encrypted slots must not reuse a nonce prefix"));
        }
        for (i, data) in [&self.cipher_a, &self.cipher_b].into_iter().enumerate() {
            let n = self.word(8 + 2 * i) as usize;
            if !(176..=SLOT_BYTES).contains(&n)
                || !n.is_multiple_of(4)
                || data.len() != n + n.div_ceil(128) * 16
            {
                return Err(invalid("Invalid encrypted slot size"));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for r in &self.recipients {
            let uid = r
                .uid
                .parse::<Uid>()
                .map_err(|_| invalid("Invalid recipient UID"))?;
            if !seen.insert(uid)
                || r.nonce.len() != 24
                || r.wrapped_key.len() != 32
                || r.tag.len() != 16
            {
                return Err(invalid("Invalid encrypted device ticket"));
            }
        }
        if seen.is_empty() {
            return Err(invalid("Encrypted firmware has no enrolled device ticket"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum Image {
    Ab(Bundle),
    Sealed(SealedBundle),
    Legacy(Vec<u8>),
}
impl Image {
    pub fn parse(data: &[u8]) -> Result<Self, OpError> {
        if data.is_empty() || data.len() > MAX_FILE {
            return Err(invalid("Firmware file is empty or exceeds 512 KiB"));
        }
        if data.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'{') {
            let shape: Value = serde_json::from_slice(data)
                .map_err(|e| invalid(format!("Invalid firmware JSON: {e}")))?;
            if shape["format"] == "voloo-x035-sealed-v1" {
                if [
                    "factory",
                    "slot_a",
                    "slot_b",
                    "content_key",
                    "device_key",
                    "devices",
                    "releases",
                ]
                .iter()
                .any(|key| shape.get(key).is_some())
                {
                    return Err(invalid(
                        "Private/plaintext fields cannot be included in a distributable encrypted package",
                    ));
                }
                let sealed: SealedBundle =
                    serde_json::from_value(shape).map_err(|e| invalid(e.to_string()))?;
                sealed.validate()?;
                return Ok(Self::Sealed(sealed));
            }
            let b: Bundle = serde_json::from_slice(data)
                .map_err(|e| invalid(format!("Invalid firmware package: {e}")))?;
            if b.format != "voloo-x035-ab-v1"
                || b.hardware_version != 0x0300
                || b.firmware_version == 0
                || b.factory.len() != FACTORY_BYTES
            {
                return Err(invalid("Not a supported X035 USB A/B package"));
            }
            for (slot, app) in [b.slot_a.as_slice(), b.slot_b.as_slice()]
                .into_iter()
                .enumerate()
            {
                let tag = format!(
                    "VolooSwitch:X035:AB:1:{}\0",
                    if slot == 0 { "A" } else { "B" }
                );
                if !(176..=SLOT_BYTES).contains(&app.len())
                    || app.len() % 4 != 0
                    || !contains(app, tag.as_bytes())
                {
                    return Err(invalid("Wrong target/slot or oversized A/B application"));
                }
                let base = if slot == 0 { 0x2000 } else { 0x8000 };
                if b.factory[base..base + app.len()] != *app {
                    return Err(invalid("Factory/application package contents disagree"));
                }
            }
            if !contains(&b.factory[..8192], b"VolooSwitch:X035:AB:BOOT:1\0") {
                return Err(invalid("Missing X035 A/B boot program"));
            }
            Ok(Self::Ab(b))
        } else if data.len() <= FACTORY_BYTES
            && contains(data, b"VolooSwitch:CH32X035F8U6:USB-ISP:1\0")
        {
            Ok(Self::Legacy(data.to_vec()))
        } else {
            Err(invalid(
                "Select an X035 .voloofw package or a USB-updatable legacy .bin",
            ))
        }
    }
}
/// Offline inspection is structural. Only the device can authenticate a ticket.
pub fn inspect(data: &[u8]) -> Result<Value, OpError> {
    let image = Image::parse(data)?;
    match image {
        Image::Sealed(bundle) => Ok(
            json!({"encrypted":true,"format":bundle.format,"hardware_version":protocol::version(bundle.hardware_version),"firmware_version":protocol::version(bundle.firmware_version),"recipients":bundle.recipients.len(),"bytes":data.len(),"authentication":"verified by the device, not by offline inspection"}),
        ),
        _ => Err(invalid(
            "This release requires an encrypted firmware package; plaintext publishing was refused",
        )),
    }
}

fn contains(data: &[u8], needle: &[u8]) -> bool {
    data.windows(needle.len()).any(|w| w == needle)
}
fn invalid(message: impl Into<String>) -> OpError {
    OpError::new(ErrorKind::InvalidArgument, message)
}
fn failed(message: impl Into<String>) -> OpError {
    OpError::new(ErrorKind::DeviceFailed, message)
}
fn unknown(message: impl Into<String>) -> OpError {
    OpError::new(ErrorKind::Unknown, message)
}
fn busy(message: impl Into<String>) -> OpError {
    OpError::new(ErrorKind::Busy, message)
}

pub fn require_off(s: &Status) -> Result<(), OpError> {
    if s.state_code != 0
        || s.output_enabled
        || s.discharge_enabled
        || s.busy
        || s.internal_error
        || s.discharge_timeout
        || !s.vin_valid
        || !s.vout_valid
        || s.vout_mv.is_none_or(|v| v > 1000)
    {
        return Err(busy(
            "Firmware maintenance requires idle OFF, valid measurements and VOUT <= 1 V. Turn off/discharge explicitly first.",
        ));
    }
    Ok(())
}
// Flash erase and USB enumeration can briefly outlive the last ADC group.
// Wait only for new valid measurements; never switch or discharge the load.
fn wait_off(device: &mut Device, timeout: Duration) -> Result<Status, OpError> {
    let deadline = Instant::now() + timeout;
    loop {
        let status = device.status()?.status;
        match require_off(&status) {
            Ok(()) => return Ok(status),
            Err(error) => {
                let settling = status.state_code == 0
                    && !status.output_enabled
                    && !status.discharge_enabled
                    && !status.busy
                    && !status.internal_error
                    && !status.discharge_timeout
                    && (!status.vin_valid || !status.vout_valid);
                if !settling || Instant::now() >= deadline {
                    return Err(error);
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

pub fn ab_request(
    device: &mut Device,
    trace: &str,
    action: u8,
    payload: &[u8],
) -> Result<AbStatus, OpError> {
    let req = protocol::firmware_request_on(device.uid, device.firmware_fc, action, payload)
        .map_err(|e| invalid(e.to_string()))?;
    match device
        .transport
        .request(trace, &req, Some(Duration::from_secs(3)))
    {
        Ok(Matched::Firmware(p)) => AbStatus::decode(&p),
        Ok(Matched::Exception(e)) => Err(OpError::new(
            if e.code == 6 {
                ErrorKind::Busy
            } else {
                ErrorKind::DeviceFailed
            },
            e.to_string(),
        )),
        Ok(_) => Err(failed("Unexpected firmware response")),
        Err(e) => Err(unknown(format!(
            "Firmware response unavailable ({e}); command was not resent"
        ))),
    }
}
pub fn status(device: &mut Device) -> Result<Value, OpError> {
    let info = device.info()?;
    let ab = if info.identity.capabilities & (CAP_AB_UPDATE | protocol::CAP_SEALED) != 0 {
        Some(ab_request(device, &new_trace_id(), 0, &[])?)
    } else {
        None
    };
    Ok(
        json!({"uid":device.uid,"identity":info.identity,"ab":ab,"supports_usb_update":info.identity.capabilities & (CAP_AB_UPDATE|CAP_USB_ISP|protocol::CAP_SEALED)!=0,"supports_rollback":ab.as_ref().is_some_and(|b|b.previous_slot.is_some())}),
    )
}

fn reopen(
    device: &mut Device,
    original_path: &str,
    scan_usb: bool,
    timeout: Duration,
) -> Result<(), OpError> {
    let (uid, recorder, source) = (device.uid, device.recorder.clone(), device.source);
    device.transport.release_for_update();
    let deadline = Instant::now() + timeout;
    let mut last = "device has not reappeared".to_string();
    while Instant::now() < deadline {
        let mut paths = vec![original_path.to_string()];
        if scan_usb && let Ok(ports) = discover::list_ports() {
            for p in ports
                .into_iter()
                .filter(|p| p.is_voloo && p.uid == Some(uid))
            {
                if !paths.contains(&p.path) {
                    paths.push(p.path);
                }
            }
        }
        for path in paths {
            match discover::open(&path, Some(uid), &recorder) {
                Ok(mut new) => {
                    new.source = source;
                    *device = new;
                    return Ok(());
                }
                Err(e) => last = e.message,
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(unknown(format!(
        "Could not reconnect to the same UID after firmware maintenance: {last}"
    )))
}

fn verify(
    device: &mut Device,
    nickname: &str,
    expected: Option<u16>,
) -> Result<(Identity, Status), OpError> {
    let info = device.info()?;
    if info.identity.uid != device.uid
        || info.identity.hardware_version != "3.0"
        || info.identity.device_type != 1
    {
        return Err(failed("Device identity changed across the update"));
    }
    if expected.is_some_and(|v| info.identity.firmware_version != protocol::version(v)) {
        return Err(failed(
            "Running firmware version does not match the selected package",
        ));
    }
    if info.nickname.as_deref() != Some(nickname) {
        return Err(failed("Nickname changed across the update"));
    }
    let s = wait_off(device, Duration::from_secs(1))?;
    Ok((info.identity, s))
}

struct TransferPlan {
    version: u16,
    length: usize,
    crc: u16,
    begin: Vec<u8>,
    packets: Vec<Vec<u8>>,
}
fn transfer_plan(image: &Image, uid: Uid, slot: u8) -> Result<TransferPlan, OpError> {
    match image {
        Image::Ab(bundle) => {
            let app = if slot == 0 {
                &bundle.slot_a
            } else {
                &bundle.slot_b
            };
            let crc = protocol::crc16(app);
            let mut begin = vec![slot, 0];
            begin.extend_from_slice(&(app.len() as u16).to_be_bytes());
            begin.extend_from_slice(&crc.to_be_bytes());
            begin.extend_from_slice(&bundle.firmware_version.to_be_bytes());
            let packets = app
                .chunks(128)
                .enumerate()
                .map(|(i, c)| {
                    let mut p = (i as u16 * 128).to_be_bytes().to_vec();
                    p.extend_from_slice(c);
                    p
                })
                .collect();
            Ok(TransferPlan {
                version: bundle.firmware_version,
                length: app.len(),
                crc,
                begin,
                packets,
            })
        }
        Image::Sealed(bundle) => {
            let recipient=bundle.recipients.iter().find(|r|r.uid.parse::<Uid>().ok()==Some(uid)).ok_or_else(||invalid("No encrypted ticket exists for this UID; trusted factory enrolment is required"))?;
            let length = bundle.word(8 + 2 * slot as usize) as usize;
            let cipher = if slot == 0 {
                &bundle.cipher_a
            } else {
                &bundle.cipher_b
            };
            let mut begin = vec![slot];
            begin.extend_from_slice(&bundle.header);
            begin.extend_from_slice(&recipient.nonce);
            begin.extend_from_slice(&recipient.wrapped_key);
            begin.extend_from_slice(&recipient.tag);
            let mut packets = Vec::new();
            let mut encrypted_offset = 0;
            for offset in (0..length).step_by(128) {
                let n = 128.min(length - offset);
                let mut p = (offset as u16).to_be_bytes().to_vec();
                p.extend_from_slice(&cipher[encrypted_offset..encrypted_offset + n + 16]);
                encrypted_offset += n + 16;
                packets.push(p);
            }
            Ok(TransferPlan {
                version: bundle.firmware_version,
                length,
                crc: bundle.word(12 + 2 * slot as usize),
                begin,
                packets,
            })
        }
        _ => Err(invalid("A/B updates require a complete firmware package")),
    }
}

/// Every mutation is sent once. A missing BEGIN/WRITE ACK is resolved only by
/// querying the session/offset; it never causes a second erase or write request.
fn transfer(
    device: &mut Device,
    trace: &str,
    image: &Image,
    initial: &AbStatus,
    progress: &mut dyn FnMut(Progress),
) -> Result<u8, OpError> {
    if initial.running_slot != initial.confirmed_slot || initial.trial_state == 2 {
        return Err(busy(
            "An unconfirmed firmware trial is running; wait for confirmation or automatic rollback",
        ));
    }
    let slot = 1 - initial.running_slot;
    let plan = transfer_plan(image, device.uid, slot)?;
    let crc = plan.crc;
    if initial.trial_state == 1
        && initial.candidate_slot == Some(slot)
        && initial.versions[slot as usize] == plan.version
        && initial.lengths[slot as usize] as usize == plan.length
        && initial.crcs[slot as usize] == crc
    {
        ab_request(device, trace, 6, &[])?;
        return Ok(slot);
    }
    if initial.upload_slot.is_some() || initial.trial_state == 1 {
        let cancel = ab_request(device, trace, 4, &[]);
        if cancel.is_err() && ab_request(device, trace, 0, &[])?.upload_slot.is_some() {
            return cancel.map(|_| slot);
        }
    }
    let started = ab_request(device, trace, 1, &plan.begin).or_else(|e| {
        let got = ab_request(device, trace, 0, &[])?;
        if !got.upload_failed
            && got.upload_slot == Some(slot)
            && got.received == 0
            && got.upload_length as usize == plan.length
        {
            Ok(got)
        } else {
            Err(e)
        }
    })?;
    if started.upload_slot != Some(slot) || started.upload_failed {
        return Err(failed("Device did not prepare the inactive slot"));
    }
    for (i, data) in plan.packets.iter().enumerate() {
        let received = ((i + 1) * 128).min(plan.length);
        let accepted = ab_request(device, trace, 2, data).or_else(|e| {
            let got = ab_request(device, trace, 0, &[])?;
            if !got.upload_failed
                && got.upload_slot == Some(slot)
                && got.received as usize == received
            {
                Ok(got)
            } else {
                Err(e)
            }
        })?;
        if accepted.upload_failed || accepted.received as usize != received {
            return Err(failed(
                "Encrypted/ordinary upload offset or flash verification failed",
            ));
        }
        progress(Progress {
            stage: "uploading".into(),
            percent: (15 + 65 * received / plan.length) as u8,
            connected: true,
            written: received,
            total: plan.length,
        });
    }
    progress(Progress::new("verifying", 82, true));
    wait_off(device, Duration::from_secs(1))?;
    // An END ACK may be lost to the intended reset. Do not repeat END. The boot
    // observer checks the committed descriptor or the trial application instead.
    let result = ab_request(device, trace, 3, &[]);
    if result
        .as_ref()
        .is_err_and(|e| !matches!(e.code, ErrorKind::Unknown | ErrorKind::LinkError))
    {
        result?;
    }
    Ok(slot)
}

struct Trial<'a> {
    target: u8,
    version: u16,
    before: &'a AbStatus,
}
fn await_boot(
    device: &mut Device,
    path: &str,
    scan_usb: bool,
    nickname: &str,
    trial: Trial<'_>,
    trace: &str,
    progress: &mut dyn FnMut(Progress),
) -> Result<Value, OpError> {
    let Trial {
        target,
        version,
        before,
    } = trial;
    progress(Progress::new("reconnecting", 85, false));
    device.transport.release_for_update();
    thread::sleep(Duration::from_millis(250));
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut verification_error = None;
    let mut reset_resumed = false;
    let mut confirmation_sent = false;
    while Instant::now() < deadline {
        if device.transport.broken.is_some()
            && reopen(device, path, scan_usb, Duration::from_secs(4)).is_err()
        {
            continue;
        }
        match ab_request(device, trace, 0, &[]) {
            Ok(state)
                if state.running_slot == target
                    && state.trial_state == 2
                    && verification_error.is_none()
                    && !confirmation_sent =>
            {
                progress(Progress::new("verifying", 92, true));
                match verify(device, nickname, Some(version)) {
                    Ok(_) => {
                        progress(Progress::new("confirming", 97, true));
                        confirmation_sent = true;
                        let confirm = ab_request(device, trace, 7, &[]);
                        let checked = match confirm
                            .or_else(|e| ab_request(device, trace, 0, &[]).map_err(|_| e))
                        {
                            Ok(state) => state,
                            Err(e) => {
                                verification_error = Some(e);
                                device.transport.release_for_update();
                                continue;
                            }
                        };
                        if checked.confirmed_slot != target
                            || checked.running_slot != target
                            || checked.trial_state != 0
                        {
                            verification_error =
                                Some(failed("Firmware confirmation was not committed"));
                        } else {
                            let (identity, status) = verify(device, nickname, Some(version))?;
                            return Ok(
                                json!({"outcome":"completed","uid":device.uid,"identity":identity,"status":status,"boot":checked,"rollback_available":checked.previous_slot.is_some()}),
                            );
                        }
                    }
                    Err(e) => {
                        verification_error = Some(e);
                    }
                }
            }
            Ok(state)
                if confirmation_sent
                    && state.running_slot == target
                    && state.confirmed_slot == target
                    && state.trial_state == 0 =>
            {
                let (identity, status) = verify(device, nickname, Some(version))?;
                return Ok(
                    json!({"outcome":"completed","uid":device.uid,"identity":identity,"status":status,"boot":state,"rollback_available":state.previous_slot.is_some()}),
                );
            }
            Ok(state)
                if state.running_slot == before.confirmed_slot
                    && state.trial_state == 0
                    && state.rollback_count > before.rollback_count =>
            {
                let (identity, status) = verify(
                    device,
                    nickname,
                    Some(before.versions[before.confirmed_slot as usize]),
                )?;
                let mut e = failed(
                    "New firmware did not pass the trial; the previous firmware was restored and verified OFF",
                );
                e.details = json!({"outcome":"rolled_back","recovered":true,"identity":identity,"status":status,"boot":state,"verification_error":verification_error.map(|e|e.message)});
                progress(Progress::new("rolled_back", 100, true));
                return Err(e);
            }
            Ok(state)
                if state.running_slot == before.confirmed_slot
                    && state.trial_state == 1
                    && state.candidate_slot == Some(target)
                    && !reset_resumed =>
            {
                // END's acknowledgement was not consumed, so the app cancelled
                // the reset while retaining a verified READY descriptor.
                ab_request(device, trace, 6, &[])?;
                reset_resumed = true;
                device.transport.release_for_update();
            }
            Ok(_) => {}
            Err(_) => {
                device.transport.release_for_update();
            }
        }
        if verification_error.is_some() {
            progress(Progress::new("waiting_for_rollback", 90, false));
        }
        thread::sleep(Duration::from_millis(150));
    }
    Err(unknown(
        "Firmware trial outcome could not be established; no confirmation or power command was resent",
    ))
}

/// Paired encrypted firmware asset in this public host release. The MCU source
/// repository is private; no private-repository credential is needed by users.
pub fn download_url() -> String {
    format!(
        "https://github.com/librezoo/voloo/releases/download/v{}/voloo-switch-x035.voloofw",
        env!("CARGO_PKG_VERSION")
    )
}
struct Downloaded {
    _file: TempImage,
    image: Image,
}
impl Downloaded {
    fn stage(mut reader: impl Read, parent: &Path, trace: &str) -> Result<Self, OpError> {
        use std::io::Write;
        let file = TempImage::create(parent, trace, "voloo-switch-x035.voloofw")?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file.path)
            .map_err(|e| invalid(format!("Cannot stage encrypted firmware: {e}")))?;
        let copied = std::io::copy(&mut reader.by_ref().take(MAX_FILE as u64 + 1), &mut output)
            .map_err(|e| {
                OpError::new(
                    ErrorKind::LinkError,
                    format!("Firmware download failed: {e}"),
                )
            })?;
        output.flush().map_err(|e| invalid(e.to_string()))?;
        drop(output);
        if copied > MAX_FILE as u64 {
            return Err(invalid("Downloaded firmware exceeds 512 KiB"));
        }
        let bytes = fs::read(&file.path).map_err(|e| invalid(e.to_string()))?;
        let image = Image::parse(&bytes)?;
        if !matches!(image, Image::Sealed(_)) {
            return Err(invalid(
                "The online asset must be encrypted firmware; plaintext was refused",
            ));
        }
        Ok(Self { _file: file, image })
    }
    fn fetch() -> Result<Self, OpError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(5)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        let mut response = agent
            .get(&download_url())
            .header("Accept", "application/octet-stream")
            .call()
            .map_err(|e| {
                OpError::new(
                    ErrorKind::LinkError,
                    format!("Cannot download the paired encrypted firmware: {e}"),
                )
            })?;
        Self::stage(
            response.body_mut().as_reader(),
            &std::env::temp_dir(),
            &new_trace_id(),
        )
    }
}

pub fn perform_online(
    device: &mut Device,
    scan_usb: bool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Value, OpError> {
    progress(Progress::new("checking", 0, true));
    let info = device.info()?;
    if info.identity.hardware_version != "3.0"
        || info.identity.capabilities & protocol::CAP_SEALED == 0
    {
        return Err(invalid(
            "Encrypted online updates require one-time trusted factory initialization; no plaintext firmware will be downloaded or flashed",
        ));
    }
    let security = ab_request(device, &new_trace_id(), 0, &[])?;
    if !security.encrypted || security.protection_flags != 14 {
        return Err(invalid(
            "Factory security initialization is incomplete: read protection, boot write protection and device-key write protection must be enabled before online updates",
        ));
    }
    wait_off(device, Duration::from_secs(1))?;
    progress(Progress::new("downloading", 0, true));
    let downloaded = Downloaded::fetch()?;
    let result = perform(device, Some(downloaded.image.clone()), scan_usb, progress);
    // The guard remains alive through reconnect/confirmation/rollback. All
    // exits, including download/parse errors, remove its private temp directory.
    drop(downloaded);
    result
}

struct TempImage {
    dir: PathBuf,
    path: PathBuf,
}
impl TempImage {
    fn new(data: &[u8], trace: &str) -> Result<Self, OpError> {
        let image = Self::create(&std::env::temp_dir(), trace, "voloo-switch-x035.bin")?;
        fs::write(&image.path, data)
            .map_err(|e| invalid(format!("Cannot write firmware image: {e}")))?;
        Ok(image)
    }
    fn create(parent: &Path, trace: &str, name: &str) -> Result<Self, OpError> {
        let dir = parent.join(format!("voloo-firmware-{trace}"));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&dir)
            .map_err(|e| invalid(format!("Cannot create private firmware directory: {e}")))?;
        let path = dir.join(name);
        Ok(Self { dir, path })
    }
}
impl Drop for TempImage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.dir);
    }
}
fn wchisp(args: &[&str], timeout: Duration) -> Result<String, OpError> {
    let executable = std::env::var_os("VOLOO_WCHISP").unwrap_or_else(|| "wchisp".into());
    let mut child = Command::new(executable)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            invalid(format!(
                "Cannot run wchisp ({e}); install it for legacy USB ISP updates"
            ))
        })?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let read = |pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut data = Vec::new();
            let _ = pipe.take(1024 * 1024).read_to_end(&mut data);
            data
        })
    };
    let out = read(Box::new(stdout));
    let err = read(Box::new(stderr));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let mut bytes = out.join().unwrap_or_default();
    bytes.extend(err.join().unwrap_or_default());
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if !status.is_some_and(|s| s.success()) {
        return Err(unknown(format!(
            "wchisp failed or timed out: {}",
            text.chars().take(4096).collect::<String>()
        )));
    }
    Ok(text)
}
fn rom_count(text: &str) -> Option<usize> {
    text.lines().find_map(|line| {
        let tail = line.split_once("Found ")?.1;
        tail.split_whitespace().next()?.parse().ok()
    })
}
fn rom_uid(text: &str) -> Option<Uid> {
    let raw = text.split_once("Chip UID:")?.1.split_whitespace().next()?;
    raw.split('-')
        .take(8)
        .collect::<Vec<_>>()
        .join("-")
        .parse()
        .ok()
}
fn legacy(
    device: &mut Device,
    image: &Image,
    nickname: &str,
    path: &str,
    scan_usb: bool,
    trace: &str,
    progress: &mut dyn FnMut(Progress),
) -> Result<Value, OpError> {
    wchisp(&["--version"], Duration::from_secs(5))?; // Before resetting anything.
    let data = match image {
        Image::Ab(b) => &b.factory,
        Image::Legacy(data) => data,
        Image::Sealed(_) => {
            return Err(invalid(
                "Encrypted firmware needs trusted factory initialization; it cannot migrate through a public factory image",
            ));
        }
    };
    if data.len().div_ceil(1024) + 1 > 0xEC00 / 1024 {
        return Err(invalid("Firmware erase range overlaps configuration"));
    }
    let file = TempImage::new(data, trace)?;
    progress(Progress::new("entering_update_mode", 10, true));
    let entry = protocol::enter_usb_isp(device.uid);
    match device
        .transport
        .request(trace, &entry, Some(Duration::from_secs(2)))
    {
        Ok(Matched::Echo) => {}
        Ok(Matched::Exception(e)) => return Err(failed(e.to_string())),
        Ok(_) => return Err(failed("Unexpected USB ISP response")),
        Err(_) => {} // Only inspect ROM state after an unknown acknowledgement.
    }
    device.transport.release_for_update();
    progress(Progress::new("reconnecting", 15, false));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match rom_count(&wchisp(&["--usb", "probe"], Duration::from_secs(4))?) {
            Some(1) => break,
            Some(0) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            Some(n) if n > 1 => {
                return Err(busy("Multiple ROM ISP devices; no firmware was erased"));
            }
            _ => {
                return Err(unknown(
                    "Selected device did not enter USB ROM ISP; nothing was erased",
                ));
            }
        }
    }
    let info = wchisp(
        &["--usb", "info", "--chip", "CH32X035F8U6"],
        Duration::from_secs(10),
    )?;
    if !info.contains("CH32X035F8U6") || rom_uid(&info) != Some(device.uid) {
        return Err(failed(
            "ROM ISP chip/UID differs from the selected device; nothing was erased",
        ));
    }
    progress(Progress::new(
        if matches!(image, Image::Ab(_)) {
            "installing_ab"
        } else {
            "uploading"
        },
        40,
        false,
    ));
    wchisp(
        &[
            "--usb",
            "flash",
            file.path
                .to_str()
                .ok_or_else(|| invalid("Invalid firmware path"))?,
        ],
        Duration::from_secs(90),
    )?;
    progress(Progress::new("verifying", 90, false));
    reopen(device, path, scan_usb, Duration::from_secs(12))?;
    let expected = match image {
        Image::Ab(b) => Some(b.firmware_version),
        _ => None,
    };
    let (identity, status) = verify(device, nickname, expected)?;
    let boot = if matches!(image, Image::Ab(_)) {
        if identity.capabilities & CAP_AB_UPDATE == 0 {
            return Err(failed(
                "Installed firmware does not report A/B update support",
            ));
        }
        let boot = ab_request(device, trace, 0, &[])?;
        if boot.running_slot != boot.confirmed_slot || boot.trial_state != 0 {
            return Err(failed(
                "A/B installation did not establish a confirmed application",
            ));
        }
        Some(boot)
    } else {
        None
    };
    Ok(
        json!({"outcome":"completed","uid":device.uid,"identity":identity,"status":status,"boot":boot,"initial_ab_installation":matches!(image,Image::Ab(_)),"expected_version_verified":expected.is_some()}),
    )
}

pub fn perform(
    device: &mut Device,
    input: Option<Image>,
    scan_usb: bool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Value, OpError> {
    let image = input;
    let trace = new_trace_id();
    let started = Instant::now();
    let recorder = Arc::clone(&device.recorder);
    let uid = device.uid;
    let source = device.source;
    let path = device.port().to_string();
    let action = if image.is_some() {
        "firmware_update"
    } else {
        "firmware_rollback"
    };
    progress(Progress::new("checking", 0, true));
    let before = wait_off(device, Duration::from_secs(1))?;
    let original = device.info()?;
    if original.identity.hardware_version != "3.0" || original.identity.device_type != 1 {
        return Err(invalid("Firmware updates require a direct X035 USB Switch"));
    }
    let nickname = original.nickname.unwrap_or_default();
    let mut result = (|| {
        if original.identity.capabilities & (CAP_AB_UPDATE | protocol::CAP_SEALED) != 0 {
            let initial = ab_request(device, &trace, 0, &[])?;
            let (target, version) = if let Some(image) = &image {
                let version = match image {
                    Image::Sealed(bundle) if initial.encrypted => bundle.firmware_version,
                    Image::Ab(bundle) if !initial.encrypted => bundle.firmware_version,
                    _ => {
                        return Err(invalid(
                            "Encrypted devices require sealed firmware; legacy devices require trusted initialization before sealed updates",
                        ));
                    }
                };
                (
                    transfer(device, &trace, image, &initial, progress)?,
                    version,
                )
            } else {
                let target = initial
                    .previous_slot
                    .ok_or_else(|| busy("No previous firmware is available for rollback"))?;
                ab_request(device, &trace, 5, &[])?;
                (target, initial.versions[target as usize])
            };
            await_boot(
                device,
                &path,
                scan_usb,
                &nickname,
                Trial {
                    target,
                    version,
                    before: &initial,
                },
                &trace,
                progress,
            )
        } else if original.identity.capabilities & CAP_USB_ISP != 0 {
            let image = image
                .as_ref()
                .ok_or_else(|| invalid("This firmware has no A/B rollback support"))?;
            legacy(device, image, &nickname, &path, scan_usb, &trace, progress)
        } else {
            Err(invalid("Device firmware does not support USB updates"))
        }
    })();
    if result.is_err() {
        // Release even a healthy old handle before reacquiring it. A failed
        // worker must not strand a port outside normal daemon polling.
        let recovered = reopen(device, &path, scan_usb, Duration::from_secs(5));
        if recovered.is_ok()
            && let Ok(state) = ab_request(device, &trace, 0, &[])
            && state.trial_state != 2
            && (state.upload_slot.is_some() || state.trial_state == 1)
        {
            let _ = ab_request(device, &trace, 4, &[]);
        }
        if let Err(e) = &mut result {
            let prior = e.details.clone();
            let state = device.status().ok().map(|r| r.status);
            e.trace_id = Some(trace.clone());
            e.uid = Some(uid.to_string());
            e.details = json!({"update":prior,"reconnected":recovered.is_ok(),"last_observed_status":state});
            if prior.get("outcome").and_then(Value::as_str) != Some("rolled_back") {
                progress(Progress::new("failed", 100, recovered.is_ok()));
            }
        }
    } else if let Ok(value) = &mut result {
        value["trace_id"] = json!(trace);
        value["elapsed_ms"] = json!(started.elapsed().as_millis());
        value["action"] = json!(action);
        progress(Progress::new("completed", 100, true));
    }
    recorder.operation(&json!({"at":now(),"trace_id":trace,"uid":uid,"source":source,"port":path,"action":action,"before":before,
        "outcome":result.as_ref().ok().and_then(|v|v.get("outcome")).cloned().unwrap_or_else(||json!("failed")),
        "after":result.as_ref().ok().and_then(|v|v.get("status")),"error":result.as_ref().err()}));
    result
}

#[cfg(test)]
mod download_tests {
    use super::*;
    fn encrypted_file() -> Vec<u8> {
        let mut header = vec![0; 48];
        header[..4].copy_from_slice(b"VSE1");
        header[4..8].copy_from_slice(&[3, 0, 3, 0]);
        header[8..12].copy_from_slice(&[2, 0, 2, 0]);
        header[32] = 1;
        serde_json::to_vec(&json!({"format":"voloo-x035-sealed-v1","hardware_version":0x300,"firmware_version":0x300,
            "header":header,"cipher_a":vec![0u8;576],"cipher_b":vec![0u8;576],
            "recipients":[{"uid":"01-02-03-04-05-06-07-08","nonce":vec![0u8;24],"wrapped_key":vec![0u8;32],"tag":vec![0u8;16]}]})).unwrap()
    }
    #[test]
    fn encrypted_download_lives_through_update_and_cleans_on_success_and_error() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = encrypted_file();
        {
            let downloaded = Downloaded::stage(&bytes[..], temp.path(), "success").unwrap();
            assert!(downloaded._file.path.exists());
            assert!(matches!(downloaded.image, Image::Sealed(_)));
            // Both successful confirmation and an update/rollback error leave
            // this scope. No caller needs a separate deletion command.
        }
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        let outcome: Result<(), OpError> = (|| {
            let _downloaded = Downloaded::stage(&bytes[..], temp.path(), "failed-update")?;
            Err(failed("device trial rolled back"))
        })();
        assert!(outcome.is_err());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }
    #[test]
    fn truncated_oversized_and_plaintext_downloads_leave_no_cache() {
        let temp = tempfile::tempdir().unwrap();
        for (name, bytes) in [
            ("truncated", b"{\"format\":".to_vec()),
            ("oversize", vec![0u8; MAX_FILE + 1]),
            (
                "plaintext",
                b"VolooSwitch:CH32X035F8U6:USB-ISP:1\0".to_vec(),
            ),
        ] {
            assert!(Downloaded::stage(&bytes[..], temp.path(), name).is_err());
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        }
        struct Broken(bool);
        impl Read for Broken {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0 {
                    Err(std::io::Error::other("interrupted network"))
                } else {
                    self.0 = true;
                    buf[..2].copy_from_slice(b"{\"");
                    Ok(2)
                }
            }
        }
        assert!(Downloaded::stage(Broken(false), temp.path(), "network").is_err());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }
    #[test]
    fn downloaded_asset_is_paired_with_this_public_release() {
        assert_eq!(
            download_url(),
            format!(
                "https://github.com/librezoo/voloo/releases/download/v{}/voloo-switch-x035.voloofw",
                env!("CARGO_PKG_VERSION")
            )
        );
    }
}
