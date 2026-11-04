//! Observability: fault catalog with de-duplication, transaction spans (打点),
//! operation journal, and bounded JSONL logs under `$VOLOO_HOME/logs`.
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// UTC RFC 3339 timestamp with millisecond precision.
pub fn rfc3339(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        d.subsec_millis()
    )
}
pub fn now() -> String {
    rfc3339(SystemTime::now())
}
fn today() -> String {
    now()[..10].replace('-', "")
}

static TRACE_COUNTER: AtomicU64 = AtomicU64::new(0);
/// Identifier tying one public operation to its transaction spans.
pub fn new_trace_id() -> String {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "t{:x}-{:x}-{:x}",
        t.as_millis(),
        std::process::id(),
        TRACE_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Device,
    Link,
    Host,
}

/// Fixed fault catalog. Codes are stable identifiers for users, agents and support.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FaultCode {
    DevDischargeTimeout,
    DevMeasurementInvalid,
    DevInternalError,
    DevVinInvalid,
    DevVoutInvalid,
    DevResetDetected,
    DevException,
    DevBusy,
    LinkResponseTimeout,
    LinkCrcError,
    LinkLateFrame,
    LinkPortBusy,
    LinkUnplugged,
    LinkNoResponse,
    LinkIdentityMismatch,
    LinkIo,
    HostLogWriteFailed,
}
impl FaultCode {
    pub const ALL: [FaultCode; 17] = [
        Self::DevDischargeTimeout,
        Self::DevMeasurementInvalid,
        Self::DevInternalError,
        Self::DevVinInvalid,
        Self::DevVoutInvalid,
        Self::DevResetDetected,
        Self::DevException,
        Self::DevBusy,
        Self::LinkResponseTimeout,
        Self::LinkCrcError,
        Self::LinkLateFrame,
        Self::LinkPortBusy,
        Self::LinkUnplugged,
        Self::LinkNoResponse,
        Self::LinkIdentityMismatch,
        Self::LinkIo,
        Self::HostLogWriteFailed,
    ];
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DevDischargeTimeout => "DEV_DISCHARGE_TIMEOUT",
            Self::DevMeasurementInvalid => "DEV_MEASUREMENT_INVALID",
            Self::DevInternalError => "DEV_INTERNAL_ERROR",
            Self::DevVinInvalid => "DEV_VIN_INVALID",
            Self::DevVoutInvalid => "DEV_VOUT_INVALID",
            Self::DevResetDetected => "DEV_RESET_DETECTED",
            Self::DevException => "DEV_EXCEPTION",
            Self::DevBusy => "DEV_BUSY",
            Self::LinkResponseTimeout => "LINK_RESPONSE_TIMEOUT",
            Self::LinkCrcError => "LINK_CRC_ERROR",
            Self::LinkLateFrame => "LINK_LATE_FRAME",
            Self::LinkPortBusy => "LINK_PORT_BUSY",
            Self::LinkUnplugged => "LINK_UNPLUGGED",
            Self::LinkNoResponse => "LINK_NO_RESPONSE",
            Self::LinkIdentityMismatch => "LINK_IDENTITY_MISMATCH",
            Self::LinkIo => "LINK_IO",
            Self::HostLogWriteFailed => "HOST_LOG_WRITE_FAILED",
        }
    }
    pub fn source(self) -> Source {
        let s = self.as_str();
        if s.starts_with("DEV_") {
            Source::Device
        } else if s.starts_with("LINK_") {
            Source::Link
        } else {
            Source::Host
        }
    }
    pub fn severity(self) -> Severity {
        match self {
            Self::DevDischargeTimeout
            | Self::DevMeasurementInvalid
            | Self::DevInternalError
            | Self::LinkUnplugged
            | Self::LinkNoResponse
            | Self::LinkIdentityMismatch
            | Self::LinkPortBusy
            | Self::LinkIo
            | Self::HostLogWriteFailed => Severity::Error,
            Self::DevResetDetected => Severity::Info,
            _ => Severity::Warning,
        }
    }
    /// Conditions stay active while observed and are cleared when they go away.
    /// Everything else is a one-shot occurrence (a timeout, an exception, a restart).
    pub fn is_condition(self) -> bool {
        matches!(
            self,
            Self::DevDischargeTimeout
                | Self::DevMeasurementInvalid
                | Self::DevInternalError
                | Self::DevVinInvalid
                | Self::DevVoutInvalid
                | Self::LinkUnplugged
                | Self::LinkNoResponse
                | Self::LinkPortBusy
                | Self::HostLogWriteFailed
        )
    }
    /// The next step shown verbatim to users and agents.
    pub fn suggestion(self) -> &'static str {
        match self {
            Self::DevDischargeTimeout => {
                "Discharge stopped after its 5 s limit with output OFF. Check the load for a source keeping VOUT above 1 V before retrying."
            }
            Self::DevMeasurementInvalid => {
                "Discharge stopped because VOUT was invalid. Check the board's ground connection and VOUT sensing; do not assume the load is discharged."
            }
            Self::DevInternalError => {
                "The device latched an internal error. Read status and diagnostics, then power-cycle the Voloo's USB connection if it persists."
            }
            Self::DevVinInvalid => {
                "VIN measurement is invalid; readings are not zero volts. Check SOURCE wiring and the PWR_GND/BUS_GND connection."
            }
            Self::DevVoutInvalid => {
                "VOUT measurement is invalid; readings are not zero volts. Check the load wiring and the PWR_GND/BUS_GND connection."
            }
            Self::DevResetDetected => {
                "The device restarted (outputs reset to OFF). Re-read status before any action; previous operations are not resumed."
            }
            Self::DevException => {
                "The device rejected the request. Check firmware version and arguments; do not blindly resend power commands."
            }
            Self::DevBusy => {
                "The device is busy with another action. Read status and wait for it to finish."
            }
            Self::LinkResponseTimeout => {
                "No reply in time. Read status before any further power action; the command may or may not have executed."
            }
            Self::LinkCrcError => {
                "Corrupted frames were seen. Try another USB cable or port; avoid unpowered hubs."
            }
            Self::LinkLateFrame => {
                "A reply arrived after its request timed out and was discarded. Read status to confirm the device state."
            }
            Self::LinkPortBusy => {
                "The serial port is in use by another program (for example a browser Web Serial page). Close it and retry."
            }
            Self::LinkUnplugged => {
                "The USB connection was lost. The output keeps its last state; reconnect and read status before acting."
            }
            Self::LinkNoResponse => {
                "The device stopped answering; its output state is unknown. Replug the USB cable and read status before acting."
            }
            Self::LinkIdentityMismatch => {
                "The UID read from the device does not match the USB serial number. Do not control this port; report it with `voloo report`."
            }
            Self::LinkIo => {
                "Serial I/O failed. Reconnect the device and read status before acting."
            }
            Self::HostLogWriteFailed => {
                "Logs could not be written. Check free space and permissions of the Voloo home directory."
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Fault {
    pub code: FaultCode,
    pub source: Source,
    pub severity: Severity,
    pub uid: Option<String>,
    pub port: Option<String>,
    pub message: String,
    pub suggestion: &'static str,
    /// true: a condition that is active until cleared; false: a one-shot event.
    pub condition: bool,
    pub first_seen: String,
    pub last_seen: String,
    pub count: u64,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub context: Value,
}

/// Wire form of a fault (suggestion and classification are re-derived from the code).
#[derive(serde::Deserialize)]
struct FaultRecord {
    code: String,
    uid: Option<String>,
    port: Option<String>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    first_seen: String,
    #[serde(default)]
    last_seen: String,
    #[serde(default)]
    count: u64,
    #[serde(default)]
    active: bool,
    trace_id: Option<String>,
    #[serde(default)]
    context: Value,
}
impl<'de> serde::Deserialize<'de> for Fault {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Fault::try_from(FaultRecord::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
impl TryFrom<FaultRecord> for Fault {
    type Error = String;
    fn try_from(r: FaultRecord) -> Result<Self, String> {
        let code =
            FaultCode::parse(&r.code).ok_or_else(|| format!("unknown fault code {}", r.code))?;
        Ok(Fault {
            code,
            source: code.source(),
            severity: code.severity(),
            uid: r.uid,
            port: r.port,
            message: r.message,
            suggestion: code.suggestion(),
            condition: code.is_condition(),
            first_seen: r.first_seen,
            last_seen: r.last_seen,
            count: r.count,
            active: r.active,
            trace_id: r.trace_id,
            context: r.context,
        })
    }
}

/// One serial transaction.
#[derive(Clone, Debug, Serialize)]
pub struct Span {
    pub trace_id: String,
    pub at: String,
    pub port: String,
    pub uid: String,
    pub label: String,
    pub fc: u8,
    pub tx_len: usize,
    pub rx_len: usize,
    pub elapsed_us: u64,
    pub result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub stale_bytes: usize,
    pub foreign_frames: usize,
    pub crc_errors: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_hex: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Debug,
    Trace,
}
impl std::str::FromStr for Level {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "info" => Ok(Level::Info),
            "debug" => Ok(Level::Debug),
            "trace" => Ok(Level::Trace),
            _ => Err("log level must be info, debug or trace".into()),
        }
    }
}

pub struct Limits {
    pub max_age: Duration,
    pub max_total_bytes: u64,
    pub ring: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_age: Duration::from_secs(14 * 86400),
            max_total_bytes: 200 << 20,
            ring: 10_000,
        }
    }
}

struct Inner {
    spans: VecDeque<Span>,
    faults: BTreeMap<(String, FaultCode), Fault>,
    pending_trace: Vec<String>,
    log_failed: bool,
}

/// Thread-safe sink shared by all devices in one process.
/// Receives every fault event ("fault.raised", "fault.occurred", "fault.cleared").
pub type Listener = Box<dyn Fn(&str, &Fault) + Send + Sync>;

pub struct Recorder {
    pub level: Level,
    dir: Option<PathBuf>,
    limits: Limits,
    inner: Mutex<Inner>,
    listener: Mutex<Option<Listener>>,
}

impl Recorder {
    /// In-memory only (tests, or when no home directory is available).
    pub fn memory(level: Level) -> Self {
        Self::build(level, None, Limits::default())
    }
    pub fn with_dir(level: Level, dir: PathBuf, limits: Limits) -> std::io::Result<Self> {
        fs::create_dir_all(&dir)?;
        let recorder = Self::build(level, Some(dir), limits);
        recorder.prune();
        recorder.restore_conditions();
        Ok(recorder)
    }
    /// Reloads conditions whose latest logged event is `fault.raised`, so a later run that
    /// sees the condition gone records `fault.cleared` instead of leaving it raised forever.
    fn restore_conditions(&self) {
        let Some(dir) = &self.dir else { return };
        let mut latest: BTreeMap<(String, FaultCode), Value> = BTreeMap::new();
        for e in read_log(dir, "events", None, 100_000) {
            let Some(code) = e["code"].as_str().and_then(FaultCode::parse) else {
                continue;
            };
            if !code.is_condition() {
                continue;
            }
            let key = e["uid"]
                .as_str()
                .or(e["port"].as_str())
                .unwrap_or("")
                .to_string();
            latest.insert((key, code), e);
        }
        let mut inner = self.inner.lock().unwrap();
        for ((key, code), e) in latest {
            if e["event"] != "fault.raised" || code == FaultCode::HostLogWriteFailed {
                continue;
            }
            let text = |k: &str| e[k].as_str().map(str::to_string);
            inner.faults.insert(
                (key, code),
                Fault {
                    code,
                    source: code.source(),
                    severity: code.severity(),
                    uid: text("uid"),
                    port: text("port"),
                    message: text("message").unwrap_or_default(),
                    suggestion: code.suggestion(),
                    condition: code.is_condition(),
                    first_seen: text("first_seen").unwrap_or_default(),
                    last_seen: text("last_seen").unwrap_or_default(),
                    count: e["count"].as_u64().unwrap_or(1),
                    active: true,
                    trace_id: text("trace_id"),
                    context: e["context"].clone(),
                },
            );
        }
    }
    fn build(level: Level, dir: Option<PathBuf>, limits: Limits) -> Self {
        Recorder {
            level,
            dir,
            inner: Mutex::new(Inner {
                spans: VecDeque::with_capacity(limits.ring.min(1024)),
                faults: BTreeMap::new(),
                pending_trace: Vec::new(),
                log_failed: false,
            }),
            limits,
            listener: Mutex::new(None),
        }
    }
    pub fn set_listener(&self, listener: Listener) {
        *self.listener.lock().unwrap() = Some(listener);
    }
    pub fn log_dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn span(&self, span: Span) {
        let line = serde_json::to_string(&span).expect("span serializes");
        let mut inner = self.inner.lock().unwrap();
        if inner.spans.len() == self.limits.ring {
            inner.spans.pop_front();
        }
        inner.spans.push_back(span);
        if self.dir.is_some() {
            inner.pending_trace.push(line);
            if inner.pending_trace.len() >= 256 {
                let lines = std::mem::take(&mut inner.pending_trace);
                drop(inner);
                self.append("trace", &lines, false);
            }
        }
    }
    pub fn spans(&self, trace_id: Option<&str>, limit: usize) -> Vec<Span> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<Span> = inner
            .spans
            .iter()
            .rev()
            .filter(|s| trace_id.is_none_or(|t| s.trace_id == t))
            .take(limit)
            .cloned()
            .collect();
        out.reverse();
        out
    }

    /// Raises (or refreshes) a fault. Returns the record; repeated raises only bump
    /// `count`/`last_seen` and are written to the event log once per activation.
    #[allow(clippy::too_many_arguments)]
    pub fn raise(
        &self,
        code: FaultCode,
        uid: Option<String>,
        port: Option<String>,
        message: impl Into<String>,
        trace_id: Option<&str>,
        context: Value,
    ) -> Fault {
        let at = now();
        let key = (uid.clone().or(port.clone()).unwrap_or_default(), code);
        let mut inner = self.inner.lock().unwrap();
        let message = message.into();
        let condition = code.is_condition();
        // Conditions de-duplicate while active; occurrences de-duplicate per process so a
        // polling loop cannot flood the log, but every occurrence is counted.
        let (fault, fresh) = match inner.faults.get_mut(&key) {
            Some(f) if f.active || !condition => {
                f.count += 1;
                f.last_seen = at;
                f.message = message;
                f.trace_id = trace_id.map(str::to_string);
                f.context = context;
                (f.clone(), false)
            }
            _ => {
                let f = Fault {
                    code,
                    source: code.source(),
                    severity: code.severity(),
                    uid,
                    port,
                    message,
                    suggestion: code.suggestion(),
                    condition: code.is_condition(),
                    first_seen: at.clone(),
                    last_seen: at,
                    count: 1,
                    active: condition,
                    trace_id: trace_id.map(str::to_string),
                    context,
                };
                inner.faults.insert(key, f.clone());
                (f, true)
            }
        };
        drop(inner);
        if fresh {
            self.event(
                if condition {
                    "fault.raised"
                } else {
                    "fault.occurred"
                },
                &fault,
            );
        }
        fault
    }
    /// Marks a fault inactive when its condition is no longer observed.
    pub fn clear(&self, code: FaultCode, key: &str) -> Option<Fault> {
        let mut inner = self.inner.lock().unwrap();
        let f = inner
            .faults
            .get_mut(&(key.to_string(), code))
            .filter(|f| f.active)?;
        f.active = false;
        f.last_seen = now();
        let f = f.clone();
        drop(inner);
        self.event("fault.cleared", &f);
        Some(f)
    }
    /// Active conditions plus occurrences seen within `window` (newest information first
    /// for agents deciding what to do next).
    pub fn relevant(&self, key: &str, window: Duration) -> Vec<Fault> {
        let cutoff = rfc3339(SystemTime::now() - window);
        let inner = self.inner.lock().unwrap();
        inner
            .faults
            .iter()
            .filter(|((k, _), f)| {
                k == key && (f.active || (!f.code.is_condition() && f.last_seen >= cutoff))
            })
            .map(|(_, f)| f.clone())
            .collect()
    }
    pub fn faults(&self, key: Option<&str>, active_only: bool) -> Vec<Fault> {
        let inner = self.inner.lock().unwrap();
        inner
            .faults
            .iter()
            .filter(|((k, _), f)| key.is_none_or(|key| k == key) && (!active_only || f.active))
            .map(|(_, f)| f.clone())
            .collect()
    }

    fn event(&self, kind: &str, fault: &Fault) {
        if let Some(listener) = &*self.listener.lock().unwrap() {
            listener(kind, fault);
        }
        let mut v = serde_json::to_value(fault).expect("fault serializes");
        v["event"] = Value::from(kind);
        self.append("events", &[v.to_string()], true);
    }
    /// Write-operation journal: fsynced per record.
    pub fn operation(&self, record: &Value) {
        self.append("operations", &[record.to_string()], true);
    }
    pub fn flush(&self) {
        let lines = std::mem::take(&mut self.inner.lock().unwrap().pending_trace);
        if !lines.is_empty() {
            self.append("trace", &lines, false);
        }
    }

    fn append(&self, stem: &str, lines: &[String], sync: bool) {
        let Some(dir) = &self.dir else { return };
        let path = dir.join(format!("{stem}-{}.jsonl", today()));
        let result = (|| -> std::io::Result<()> {
            let mut options = OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file: File = options.open(&path)?;
            let mut buf = String::new();
            for l in lines {
                buf.push_str(l);
                buf.push('\n');
            }
            file.write_all(buf.as_bytes())?;
            if sync {
                file.sync_data()?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut inner = self.inner.lock().unwrap();
            if !inner.log_failed {
                inner.log_failed = true;
                let at = now();
                inner.faults.insert(
                    (String::new(), FaultCode::HostLogWriteFailed),
                    Fault {
                        code: FaultCode::HostLogWriteFailed,
                        source: Source::Host,
                        severity: Severity::Error,
                        uid: None,
                        port: None,
                        message: format!("{}: {error}", path.display()),
                        suggestion: FaultCode::HostLogWriteFailed.suggestion(),
                        condition: true,
                        first_seen: at.clone(),
                        last_seen: at,
                        count: 1,
                        active: true,
                        trace_id: None,
                        context: Value::Null,
                    },
                );
            }
        }
    }

    /// Deletes logs older than `max_age`, then oldest files until under `max_total_bytes`.
    pub fn prune(&self) {
        let Some(dir) = &self.dir else { return };
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((m.modified().ok()?, m.len(), e.path()))
            })
            .collect();
        files.sort();
        let cutoff = SystemTime::now() - self.limits.max_age;
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        for (modified, len, path) in files {
            if (modified < cutoff || total > self.limits.max_total_bytes)
                && fs::remove_file(&path).is_ok()
            {
                total -= len;
            }
        }
    }
}

/// Reads JSONL log records (newest last), optionally filtered by a substring match on uid.
pub fn read_log(dir: &Path, stem: &str, uid: Option<&str>, limit: usize) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{stem}-")) && n.ends_with(".jsonl"))
        })
        .collect();
    paths.sort();
    let mut out = VecDeque::new();
    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if uid.is_some_and(|u| v.get("uid").and_then(Value::as_str) != Some(u)) {
                continue;
            }
            if out.len() == limit {
                out.pop_front();
            }
            out.push_back(v);
        }
    }
    out.into()
}
