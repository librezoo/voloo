//! Voloo Switch UID-extended RTU protocol v2.5 (vectors in tests/fixtures/protocol-examples.json).
//! Pure encoding/decoding: no I/O, no clocks.
use serde::{Serialize, Serializer};
use std::{fmt, str::FromStr};

pub const UNIT: u8 = 0xF7;
pub const FC_READ_HOLDING: u8 = 0x03;
pub const FC_READ_INPUT: u8 = 0x04;
pub const FC_WRITE_SINGLE: u8 = 0x06;
pub const FC_WRITE_MULTIPLE: u8 = 0x10;

/// Input-register blocks; a read must stay inside one block and is at most 13 registers.
pub const INPUT_BLOCKS: [(u16, u16); 6] = [
    (0x0000, 0x0005),
    (0x0100, 0x0108),
    (0x0110, 0x011C),
    (0x0200, 0x0207),
    (0x0300, 0x030C),
    (0x0400, 0x040C),
];
pub const STATUS_BLOCK: (u16, u16) = (0x0000, 6);
pub const IDENTITY_BLOCK: (u16, u16) = (0x0100, 9);
pub const NICKNAME_BLOCK: (u16, u16) = (0x0110, 13);
pub const COMM_BLOCK: (u16, u16) = (0x0200, 8);
pub const ADC_BLOCK: (u16, u16) = (0x0300, 13);
pub const FAST_BLOCK: (u16, u16) = (0x0400, 13);
pub const NICKNAME_MAX_BYTES: usize = 24;
pub const CAP_NICKNAME: u16 = 0x0020;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("UID must contain exactly 16 hexadecimal digits")]
    BadUid,
    #[error("invalid input-register range; reads cannot cross gaps or exceed 13 registers")]
    BadRange,
    #[error("nickname: {0}")]
    BadNickname(&'static str),
    #[error("malformed reply: {0}")]
    Malformed(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uid(pub [u8; 8]);

impl FromStr for Uid {
    type Err = ProtocolError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let compact: String = s
            .trim()
            .chars()
            .filter(|c| !matches!(c, '-' | ':' | ' '))
            .collect();
        if compact.len() != 16 || !compact.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ProtocolError::BadUid);
        }
        let mut out = [0u8; 8];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&compact[i * 2..i * 2 + 2], 16)
                .map_err(|_| ProtocolError::BadUid)?;
        }
        Ok(Uid(out))
    }
}
impl fmt::Display for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("-")?;
            }
            write!(f, "{b:02X}")?;
        }
        Ok(())
    }
}
impl fmt::Debug for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl Serialize for Uid {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in bytes {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

pub fn with_crc(mut frame: Vec<u8>) -> Vec<u8> {
    let crc = crc16(&frame);
    frame.extend_from_slice(&crc.to_le_bytes());
    frame
}

fn addressed(fc: u8, uid: Uid, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(12 + payload.len());
    frame.extend_from_slice(&[UNIT, fc]);
    frame.extend_from_slice(&uid.0);
    frame.extend_from_slice(payload);
    with_crc(frame)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Off = 0,
    On = 1,
    Discharge = 2,
}
impl Command {
    pub fn code(self) -> u16 {
        self as u16
    }
    pub fn name(self) -> &'static str {
        match self {
            Command::Off => "off",
            Command::On => "on",
            Command::Discharge => "discharge",
        }
    }
}

/// What a reply must look like to belong to a request. Replies carry no transaction id,
/// so only one request may be outstanding per link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    Registers { fc: u8, count: u16 },
    Echo { fc: u8, frame: Vec<u8> },
}
impl Expect {
    pub fn fc(&self) -> u8 {
        match self {
            Expect::Registers { fc, .. } | Expect::Echo { fc, .. } => *fc,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub uid: Uid,
    pub frame: Vec<u8>,
    pub expect: Expect,
    /// Name used in traces, e.g. "read 0x0000/6" or "control on".
    pub label: String,
}

pub fn read_input(uid: Uid, start: u16, count: u16) -> Result<Request, ProtocolError> {
    let end = start as u32 + count as u32;
    if count == 0
        || count > 13
        || !INPUT_BLOCKS
            .iter()
            .any(|&(lo, hi)| start >= lo && end - 1 <= hi as u32)
    {
        return Err(ProtocolError::BadRange);
    }
    let mut payload = start.to_be_bytes().to_vec();
    payload.extend_from_slice(&count.to_be_bytes());
    Ok(Request {
        uid,
        frame: addressed(FC_READ_INPUT, uid, &payload),
        expect: Expect::Registers {
            fc: FC_READ_INPUT,
            count,
        },
        label: format!("read 0x{start:04X}/{count}"),
    })
}

pub fn write_control(uid: Uid, command: Command) -> Request {
    let mut payload = 0u16.to_be_bytes().to_vec();
    payload.extend_from_slice(&command.code().to_be_bytes());
    let frame = addressed(FC_WRITE_SINGLE, uid, &payload);
    Request {
        uid,
        expect: Expect::Echo {
            fc: FC_WRITE_SINGLE,
            frame: frame.clone(),
        },
        frame,
        label: format!("control {}", command.name()),
    }
}

/// 26 bytes: big-endian length followed by UTF-8, zero padded.
pub fn nickname_data(nickname: &str) -> Result<[u8; 26], ProtocolError> {
    if nickname
        .chars()
        .any(|c| (c as u32) < 0x20 || c as u32 == 0x7F)
    {
        return Err(ProtocolError::BadNickname(
            "cannot contain ASCII control characters",
        ));
    }
    let bytes = nickname.as_bytes();
    if bytes.len() > NICKNAME_MAX_BYTES {
        return Err(ProtocolError::BadNickname("exceeds 24 UTF-8 bytes"));
    }
    let mut data = [0u8; 26];
    data[..2].copy_from_slice(&(bytes.len() as u16).to_be_bytes());
    data[2..2 + bytes.len()].copy_from_slice(bytes);
    Ok(data)
}

pub fn write_nickname(uid: Uid, nickname: &str) -> Result<Request, ProtocolError> {
    let data = nickname_data(nickname)?;
    let (start, count) = NICKNAME_BLOCK;
    let mut payload = start.to_be_bytes().to_vec();
    payload.extend_from_slice(&count.to_be_bytes());
    payload.push(26);
    payload.extend_from_slice(&data);
    let mut echo = start.to_be_bytes().to_vec();
    echo.extend_from_slice(&count.to_be_bytes());
    Ok(Request {
        uid,
        frame: addressed(FC_WRITE_MULTIPLE, uid, &payload),
        expect: Expect::Echo {
            fc: FC_WRITE_MULTIPLE,
            frame: addressed(FC_WRITE_MULTIPLE, uid, &echo),
        },
        label: "write nickname".into(),
    })
}

pub fn decode_nickname(registers: &[u16]) -> Result<String, ProtocolError> {
    if registers.len() != 13 {
        return Err(ProtocolError::Malformed(
            "nickname block must be 13 registers",
        ));
    }
    let data: Vec<u8> = registers.iter().flat_map(|r| r.to_be_bytes()).collect();
    let len = u16::from_be_bytes([data[0], data[1]]) as usize;
    if len > NICKNAME_MAX_BYTES || data[2 + len..].iter().any(|&b| b != 0) {
        return Err(ProtocolError::Malformed(
            "invalid nickname length or padding",
        ));
    }
    let name = std::str::from_utf8(&data[2..2 + len])
        .map_err(|_| ProtocolError::Malformed("nickname is not UTF-8"))?
        .to_string();
    nickname_data(&name)
        .map_err(|_| ProtocolError::Malformed("nickname has control characters"))?;
    Ok(name)
}

/// Device exception code from an `fc | 0x80` reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceException {
    pub fc: u8,
    pub code: u8,
}
impl DeviceException {
    pub fn name(&self) -> &'static str {
        match self.code {
            1 => "ILLEGAL_FUNCTION",
            2 => "ILLEGAL_DATA_ADDRESS",
            3 => "ILLEGAL_DATA_VALUE",
            4 => "DEVICE_FAILURE",
            6 => "DEVICE_BUSY",
            _ => "UNKNOWN",
        }
    }
}
impl fmt::Display for DeviceException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "device exception 0x{:02X} ({}) for FC 0x{:02X}",
            self.code,
            self.name(),
            self.fc
        )
    }
}

/// Splits a byte stream into CRC-valid frames. Bytes that cannot start a frame are skipped
/// and counted in `discarded`.
#[derive(Default, Debug)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    pub discarded: usize,
    pub crc_errors: usize,
}
impl FrameDecoder {
    pub fn pending(&self) -> usize {
        self.buffer.len()
    }
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        loop {
            if self.buffer.is_empty() {
                break;
            }
            if self.buffer[0] != UNIT {
                self.skip();
                continue;
            }
            if self.buffer.len() < 2 {
                break;
            }
            let length = match self.buffer[1] {
                0x83 | 0x84 | 0x86 | 0x90 => 13,
                FC_WRITE_SINGLE | FC_WRITE_MULTIPLE => 16,
                FC_READ_HOLDING | FC_READ_INPUT => {
                    if self.buffer.len() < 11 {
                        break;
                    }
                    let count = self.buffer[10] as usize;
                    if !(2..=26).contains(&count) || !count.is_multiple_of(2) {
                        self.skip();
                        continue;
                    }
                    13 + count
                }
                _ => {
                    self.skip();
                    continue;
                }
            };
            if self.buffer.len() < length {
                break;
            }
            let frame = &self.buffer[..length];
            let crc = u16::from_le_bytes([frame[length - 2], frame[length - 1]]);
            if crc16(&frame[..length - 2]) != crc {
                self.crc_errors += 1;
                self.skip();
                continue;
            }
            frames.push(frame.to_vec());
            self.buffer.drain(..length);
        }
        frames
    }
    fn skip(&mut self) {
        self.buffer.remove(0);
        self.discarded += 1;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Matched {
    Registers(Vec<u16>),
    Echo,
    Exception(DeviceException),
}

/// Returns `None` when the frame belongs to a different UID/function/shape (stale or foreign).
pub fn match_reply(uid: Uid, expect: &Expect, frame: &[u8]) -> Option<Matched> {
    if frame.len() < 13 || frame[0] != UNIT || frame[2..10] != uid.0 {
        return None;
    }
    let fc = expect.fc();
    if frame[1] == fc | 0x80 && frame.len() == 13 {
        return Some(Matched::Exception(DeviceException {
            fc,
            code: frame[10],
        }));
    }
    if frame[1] != fc {
        return None;
    }
    match expect {
        Expect::Registers { count, .. } => {
            let bytes = *count as usize * 2;
            if frame[10] as usize != bytes || frame.len() != 13 + bytes {
                return None;
            }
            Some(Matched::Registers(
                frame[11..11 + bytes]
                    .chunks(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect(),
            ))
        }
        Expect::Echo { frame: echo, .. } => (frame == echo.as_slice()).then_some(Matched::Echo),
    }
}

pub const STATE_NAMES: [&str; 4] = ["OFF", "ON", "TURNING_ON", "DISCHARGING"];
pub const RESULT_NAMES: [&str; 6] = [
    "NONE",
    "RUNNING",
    "SUCCESS",
    "DISCHARGE_TIMEOUT",
    "MEASUREMENT_INVALID",
    "INTERNAL_ERROR",
];
pub mod flags {
    pub const OUTPUT: u16 = 0x0001;
    pub const DISCHARGE: u16 = 0x0002;
    pub const BUSY: u16 = 0x0004;
    pub const VIN_VALID: u16 = 0x0008;
    pub const VOUT_VALID: u16 = 0x0010;
    pub const LOW_QUALIFIED: u16 = 0x0020;
    pub const DISCHARGE_TIMEOUT: u16 = 0x0040;
    pub const INTERNAL_ERROR: u16 = 0x0080;
}

fn name_of(table: &[&str], code: u16) -> String {
    table
        .get(code as usize)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("UNKNOWN_{code}"))
}

/// Status block 0x0000..0x0005. Invalid measurements are `None`, never 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub power_state: String,
    pub state_code: u16,
    pub status_flags: u16,
    pub output_enabled: bool,
    pub discharge_enabled: bool,
    pub busy: bool,
    pub vin_valid: bool,
    pub vout_valid: bool,
    pub vin_mv: Option<u16>,
    pub vout_mv: Option<u16>,
    pub low_qualified: bool,
    pub discharge_timeout: bool,
    pub internal_error: bool,
    pub last_command: u16,
    pub result_code: u16,
    pub last_result: String,
}
impl Status {
    pub fn decode(r: &[u16]) -> Result<Self, ProtocolError> {
        if r.len() != 6 {
            return Err(ProtocolError::Malformed("status block must be 6 registers"));
        }
        let f = r[1];
        Ok(Status {
            power_state: name_of(&STATE_NAMES, r[0]),
            state_code: r[0],
            status_flags: f,
            output_enabled: f & flags::OUTPUT != 0,
            discharge_enabled: f & flags::DISCHARGE != 0,
            busy: f & flags::BUSY != 0,
            vin_valid: f & flags::VIN_VALID != 0,
            vout_valid: f & flags::VOUT_VALID != 0,
            vin_mv: (f & flags::VIN_VALID != 0).then_some(r[2]),
            vout_mv: (f & flags::VOUT_VALID != 0).then_some(r[3]),
            low_qualified: f & flags::LOW_QUALIFIED != 0,
            discharge_timeout: f & flags::DISCHARGE_TIMEOUT != 0,
            internal_error: f & flags::INTERNAL_ERROR != 0,
            last_command: r[4],
            result_code: r[5],
            last_result: name_of(&RESULT_NAMES, r[5]),
        })
    }
}

pub fn version(value: u16) -> String {
    format!("{}.{}", value >> 8, value & 0xFF)
}
pub fn u32_of(high: u16, low: u16) -> u32 {
    (high as u32) << 16 | low as u32
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Identity {
    pub uid: Uid,
    pub protocol_version: String,
    pub device_type: u16,
    pub firmware_version: String,
    pub hardware_version: String,
    pub capabilities: u16,
}
impl Identity {
    pub fn decode(r: &[u16]) -> Result<Self, ProtocolError> {
        if r.len() != 9 {
            return Err(ProtocolError::Malformed(
                "identity block must be 9 registers",
            ));
        }
        let mut uid = [0u8; 8];
        for (i, reg) in r[4..8].iter().enumerate() {
            uid[i * 2..i * 2 + 2].copy_from_slice(&reg.to_be_bytes());
        }
        Ok(Identity {
            uid: Uid(uid),
            protocol_version: version(r[0]),
            device_type: r[1],
            firmware_version: version(r[2]),
            hardware_version: version(r[3]),
            capabilities: r[8],
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommDiagnostics {
    pub uptime_s: u32,
    pub valid_request_count: u32,
    pub crc_error_count: u16,
    pub uart_error_count: u16,
    pub exception_response_count: u16,
    pub reset_reason: u16,
}
impl CommDiagnostics {
    pub fn decode(r: &[u16]) -> Result<Self, ProtocolError> {
        if r.len() != 8 {
            return Err(ProtocolError::Malformed(
                "communication block must be 8 registers",
            ));
        }
        Ok(CommDiagnostics {
            uptime_s: u32_of(r[0], r[1]),
            valid_request_count: u32_of(r[2], r[3]),
            crc_error_count: r[4],
            uart_error_count: r[5],
            exception_response_count: r[6],
            reset_reason: r[7],
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AdcDiagnostics {
    pub diagnostic_version: u16,
    pub measurement_seq: u32,
    pub sample_count: u16,
    pub vin_raw_sum: u16,
    pub vout_raw_sum: u16,
    pub calibration_code: u16,
    pub power_state: u16,
    pub status_flags: u16,
    pub vin_mv: Option<u16>,
    pub vout_mv: Option<u16>,
    pub uptime_s: u32,
}
impl AdcDiagnostics {
    pub fn decode(r: &[u16]) -> Result<Self, ProtocolError> {
        if r.len() != 13 {
            return Err(ProtocolError::Malformed("ADC block must be 13 registers"));
        }
        Ok(AdcDiagnostics {
            diagnostic_version: r[0],
            measurement_seq: u32_of(r[1], r[2]),
            sample_count: r[3],
            vin_raw_sum: r[4],
            vout_raw_sum: r[5],
            calibration_code: r[6],
            power_state: r[7],
            status_flags: r[8],
            vin_mv: (r[8] & flags::VIN_VALID != 0).then_some(r[9]),
            vout_mv: (r[8] & flags::VOUT_VALID != 0).then_some(r[10]),
            uptime_s: u32_of(r[11], r[12]),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FastDiagnostics {
    pub diagnostic_version: u16,
    pub ready: bool,
    pub prepared: bool,
    pub last_token: u32,
    pub processing_us: u32,
    pub started_us: u32,
    pub gpio_written_us: u32,
    pub accepted_count: u16,
    pub result_code: u16,
    pub link_kbaud: u16,
}
impl FastDiagnostics {
    pub fn decode(r: &[u16]) -> Result<Self, ProtocolError> {
        if r.len() != 13 {
            return Err(ProtocolError::Malformed("fast block must be 13 registers"));
        }
        Ok(FastDiagnostics {
            diagnostic_version: r[0],
            ready: r[1] & 1 != 0,
            prepared: r[1] & 2 != 0,
            last_token: u32_of(r[2], r[3]),
            processing_us: u32_of(r[4], r[5]),
            started_us: u32_of(r[6], r[7]),
            gpio_written_us: u32_of(r[8], r[9]),
            accepted_count: r[10],
            result_code: r[11],
            link_kbaud: r[12],
        })
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
