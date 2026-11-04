//! Device operations with the protocol's completion rules (master protocol §7):
//! read before writing, confirm by status, never resend a power command after a lost ACK,
//! report `unknown` when evidence is insufficient, and never cancel a device action on timeout.
use crate::{
    link::{LinkError, LinkStats, Transport},
    observe::{Fault, FaultCode, Recorder, new_trace_id, now},
    protocol::{
        self, ADC_BLOCK, AdcDiagnostics, CAP_NICKNAME, COMM_BLOCK, CommDiagnostics, Command,
        DeviceException, FAST_BLOCK, FastDiagnostics, IDENTITY_BLOCK, Identity, Matched,
        NICKNAME_BLOCK, Request, STATUS_BLOCK, Status, Uid, flags,
    },
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    thread::sleep,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidArgument,
    NotFound,
    /// The device reported failure (result code, latched flag or exception).
    DeviceFailed,
    /// Completion could not be established; read status before any further action.
    Unknown,
    /// The port could not be used or the reply never came (read-only requests).
    LinkError,
    Busy,
}
impl ErrorKind {
    pub fn http_status(self) -> u16 {
        match self {
            ErrorKind::InvalidArgument => 400,
            ErrorKind::NotFound => 404,
            ErrorKind::Busy => 409,
            ErrorKind::DeviceFailed => 502,
            ErrorKind::LinkError => 503,
            ErrorKind::Unknown => 504,
        }
    }
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::DeviceFailed => 2,
            ErrorKind::Unknown | ErrorKind::LinkError => 3,
            ErrorKind::InvalidArgument | ErrorKind::NotFound => 4,
            ErrorKind::Busy => 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct OpError {
    pub code: ErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub uid: Option<String>,
    #[serde(skip_serializing_if = "Value::is_null", default)]
    pub details: Value,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub faults: Vec<Fault>,
}
impl OpError {
    pub fn new(code: ErrorKind, message: impl Into<String>) -> Self {
        OpError {
            code,
            message: message.into(),
            trace_id: None,
            uid: None,
            details: Value::Null,
            faults: Vec::new(),
        }
    }
    fn at(mut self, trace_id: &str, uid: Uid) -> Self {
        self.trace_id = Some(trace_id.into());
        self.uid = Some(uid.to_string());
        self
    }
    fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
}
impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    AlreadySatisfied,
}

#[derive(Debug, Clone, Serialize)]
pub struct PowerResult {
    pub trace_id: String,
    pub uid: Uid,
    pub action: Command,
    pub outcome: Outcome,
    /// `None`: no command was sent; `Some(false)`: sent but the ACK was lost.
    pub acknowledged: Option<bool>,
    pub status: Status,
    pub elapsed_ms: u64,
    pub faults: Vec<Fault>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CycleResult {
    pub trace_id: String,
    pub uid: Uid,
    pub off: PowerResult,
    pub off_ms: u64,
    pub on: PowerResult,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub trace_id: String,
    pub uid: Uid,
    pub port: String,
    pub observed_at: String,
    #[serde(flatten)]
    pub status: Status,
    pub faults: Vec<Fault>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InfoReport {
    pub trace_id: String,
    pub port: String,
    pub observed_at: String,
    #[serde(flatten)]
    pub identity: Identity,
    pub nickname: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Diagnostics {
    Communication(CommDiagnostics),
    Adc(AdcDiagnostics),
    Fast(FastDiagnostics),
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsReport {
    pub trace_id: String,
    pub uid: Uid,
    pub observed_at: String,
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Diagnostics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NicknameResult {
    pub trace_id: String,
    pub uid: Uid,
    pub nickname: String,
    pub acknowledged: bool,
    pub verified: bool,
}

pub struct Budgets {
    pub on_off: Duration,
    pub discharge: Duration,
    pub poll: Duration,
    pub nickname_reply: Duration,
}
impl Default for Budgets {
    fn default() -> Self {
        Budgets {
            on_off: Duration::from_millis(900),
            discharge: Duration::from_millis(5500),
            poll: Duration::from_millis(25),
            nickname_reply: Duration::from_millis(350),
        }
    }
}

/// True when the device has stably reached the commanded state.
pub fn stable(s: &Status, command: Command) -> bool {
    if s.busy || s.internal_error || s.discharge_timeout {
        return false;
    }
    match command {
        Command::On => s.state_code == 1 && s.output_enabled && !s.discharge_enabled,
        Command::Off => s.state_code == 0 && !s.output_enabled && !s.discharge_enabled,
        Command::Discharge => {
            s.state_code == 0
                && !s.output_enabled
                && !s.discharge_enabled
                && s.low_qualified
                && s.vout_mv.is_some_and(|v| v <= 1000)
        }
    }
}

pub struct Device {
    pub transport: Transport,
    pub uid: Uid,
    pub recorder: Arc<Recorder>,
    pub budgets: Budgets,
    /// Who asked: cli, mcp, http, ui. Written to the operation journal.
    pub source: &'static str,
}

impl Device {
    pub fn new(transport: Transport, uid: Uid, recorder: Arc<Recorder>) -> Self {
        Device {
            transport,
            uid,
            recorder,
            budgets: Budgets::default(),
            source: "cli",
        }
    }
    pub fn port(&self) -> String {
        self.transport.path().to_string()
    }
    fn key(&self) -> String {
        self.uid.to_string()
    }

    fn raise(
        &self,
        code: FaultCode,
        message: impl Into<String>,
        trace_id: &str,
        context: Value,
    ) -> Fault {
        self.recorder.raise(
            code,
            Some(self.key()),
            Some(self.port()),
            message,
            Some(trace_id),
            context,
        )
    }

    /// One transaction plus link-level fault bookkeeping.
    fn exchange(
        &mut self,
        trace_id: &str,
        req: &Request,
        timeout: Option<Duration>,
    ) -> Result<Matched, LinkError> {
        self.exchange_probe(trace_id, req, timeout, false)
    }
    /// `optional`: an ILLEGAL_DATA_ADDRESS reply means "block not supported", not a fault.
    fn exchange_probe(
        &mut self,
        trace_id: &str,
        req: &Request,
        timeout: Option<Duration>,
        optional: bool,
    ) -> Result<Matched, LinkError> {
        let before: LinkStats = self.transport.stats.clone();
        let result = self.transport.request(trace_id, req, timeout);
        let after = &self.transport.stats;
        if after.crc_errors > before.crc_errors {
            self.raise(
                FaultCode::LinkCrcError,
                format!(
                    "{} corrupted frame(s) during {}",
                    after.crc_errors - before.crc_errors,
                    req.label
                ),
                trace_id,
                Value::Null,
            );
        }
        if after.late_or_foreign_frames > before.late_or_foreign_frames
            || after.stale_bytes > before.stale_bytes
        {
            self.raise(
                FaultCode::LinkLateFrame,
                format!("discarded late data around {}", req.label),
                trace_id,
                json!({"stale_bytes": after.stale_bytes - before.stale_bytes, "frames": after.late_or_foreign_frames - before.late_or_foreign_frames}),
            );
        }
        match &result {
            Err(LinkError::Timeout(ms)) => {
                self.raise(
                    FaultCode::LinkResponseTimeout,
                    format!("{} got no reply in {ms} ms", req.label),
                    trace_id,
                    Value::Null,
                );
            }
            Err(e @ LinkError::Gone(_)) => {
                self.raise(
                    FaultCode::LinkUnplugged,
                    e.to_string(),
                    trace_id,
                    Value::Null,
                );
            }
            Err(e @ LinkError::Busy(_)) => {
                self.raise(
                    FaultCode::LinkPortBusy,
                    e.to_string(),
                    trace_id,
                    Value::Null,
                );
            }
            Err(e @ LinkError::Io(_)) => {
                self.raise(FaultCode::LinkIo, e.to_string(), trace_id, Value::Null);
            }
            Ok(Matched::Exception(e)) if optional && e.code == 2 => {}
            Ok(Matched::Exception(e)) => {
                let code = if e.code == 6 {
                    FaultCode::DevBusy
                } else {
                    FaultCode::DevException
                };
                self.raise(
                    code,
                    format!("{} during {}", e, req.label),
                    trace_id,
                    Value::Null,
                );
            }
            Ok(_) => {
                // The device answered: link conditions from earlier attempts are over.
                self.recorder.clear(FaultCode::LinkUnplugged, &self.key());
                self.recorder.clear(FaultCode::LinkPortBusy, &self.key());
                self.recorder.clear(FaultCode::LinkNoResponse, &self.key());
            }
        }
        result
    }

    fn read_block(&mut self, trace_id: &str, block: (u16, u16)) -> Result<Vec<u16>, OpError> {
        self.read_block_probe(trace_id, block, false)
    }
    fn read_block_probe(
        &mut self,
        trace_id: &str,
        (start, count): (u16, u16),
        optional: bool,
    ) -> Result<Vec<u16>, OpError> {
        let req = protocol::read_input(self.uid, start, count).expect("valid fixed block");
        match self.exchange_probe(trace_id, &req, None, optional) {
            Ok(Matched::Registers(r)) => Ok(r),
            Ok(Matched::Exception(e)) => Err(exception_error(e)),
            Ok(Matched::Echo) => unreachable!("reads never match an echo"),
            Err(e) => Err(link_error(e)),
        }
        .map_err(|e| e.at(trace_id, self.uid))
    }

    fn read_status(&mut self, trace_id: &str) -> Result<Status, OpError> {
        let r = self.read_block(trace_id, STATUS_BLOCK)?;
        let status =
            Status::decode(&r).map_err(|e| OpError::new(ErrorKind::LinkError, e.to_string()))?;
        self.check_status(&status, trace_id);
        Ok(status)
    }

    /// Raises/clears device faults implied by a fresh status block.
    fn check_status(&self, s: &Status, trace_id: &str) {
        let key = self.key();
        let snapshot = || serde_json::to_value(s).unwrap_or(Value::Null);
        let set = |code: FaultCode, on: bool, message: &str| {
            if on {
                self.raise(code, message, trace_id, snapshot());
            } else {
                self.recorder.clear(code, &key);
            }
        };
        set(
            FaultCode::DevDischargeTimeout,
            s.discharge_timeout || s.result_code == 3,
            "discharge timed out (flag latched)",
        );
        set(
            FaultCode::DevMeasurementInvalid,
            s.result_code == 4,
            "last discharge ended with VOUT invalid",
        );
        set(
            FaultCode::DevInternalError,
            s.internal_error || s.result_code == 5,
            "device reports an internal error",
        );
        set(
            FaultCode::DevVinInvalid,
            !s.vin_valid,
            "VIN measurement invalid",
        );
        set(
            FaultCode::DevVoutInvalid,
            !s.vout_valid,
            "VOUT measurement invalid",
        );
    }

    /// Active conditions and recent occurrences for this device.
    pub fn relevant_faults(&self) -> Vec<Fault> {
        self.recorder
            .relevant(&self.key(), Duration::from_secs(600))
    }

    /// Background poll: same checks as `status`, without a span unless logging at `trace`.
    pub fn poll(&mut self) -> Result<StatusReport, OpError> {
        self.transport.silent = true;
        let result = self.status();
        self.transport.silent = false;
        result
    }

    pub fn status(&mut self) -> Result<StatusReport, OpError> {
        let trace_id = new_trace_id();
        let status = self.read_status(&trace_id)?;
        Ok(StatusReport {
            uid: self.uid,
            port: self.port(),
            observed_at: now(),
            status,
            faults: self.relevant_faults(),
            trace_id,
        })
    }

    pub fn identity(&mut self, trace_id: &str) -> Result<Identity, OpError> {
        let r = self.read_block(trace_id, IDENTITY_BLOCK)?;
        let identity =
            Identity::decode(&r).map_err(|e| OpError::new(ErrorKind::LinkError, e.to_string()))?;
        if identity.uid != self.uid {
            self.raise(
                FaultCode::LinkIdentityMismatch,
                format!(
                    "identity register reports {} but {} was expected",
                    identity.uid, self.uid
                ),
                trace_id,
                Value::Null,
            );
            return Err(OpError::new(
                ErrorKind::InvalidArgument,
                "Identity-register UID does not match the requested UID",
            )
            .at(trace_id, self.uid)
            .details(json!({"reported_uid": identity.uid.to_string()})));
        }
        Ok(identity)
    }

    pub fn info(&mut self) -> Result<InfoReport, OpError> {
        let trace_id = new_trace_id();
        let identity = self.identity(&trace_id)?;
        let nickname = if identity.capabilities & CAP_NICKNAME != 0 {
            Some(self.nickname(&trace_id)?)
        } else {
            None
        };
        Ok(InfoReport {
            trace_id,
            port: self.port(),
            observed_at: now(),
            identity,
            nickname,
        })
    }

    pub fn nickname(&mut self, trace_id: &str) -> Result<String, OpError> {
        let r = self.read_block(trace_id, NICKNAME_BLOCK)?;
        protocol::decode_nickname(&r)
            .map_err(|e| OpError::new(ErrorKind::LinkError, e.to_string()).at(trace_id, self.uid))
    }

    pub fn diagnostics(&mut self, kind: &str) -> Result<DiagnosticsReport, OpError> {
        let trace_id = new_trace_id();
        let block = match kind {
            "communication" => COMM_BLOCK,
            "adc" => ADC_BLOCK,
            "fast" => FAST_BLOCK,
            _ => {
                return Err(OpError::new(
                    ErrorKind::InvalidArgument,
                    "kind must be communication, adc or fast",
                ));
            }
        };
        let r = match self.read_block_probe(&trace_id, block, kind != "communication") {
            Ok(r) => r,
            // Optional blocks may be absent on older firmware; that does not mean offline.
            Err(e)
                if kind != "communication"
                    && e.details.get("exception_code") == Some(&json!(2)) =>
            {
                let status = self.read_status(&trace_id)?;
                return Ok(DiagnosticsReport {
                    trace_id,
                    uid: self.uid,
                    observed_at: now(),
                    supported: false,
                    data: None,
                    status: Some(status),
                });
            }
            Err(e) => return Err(e),
        };
        let bad = |e: protocol::ProtocolError| OpError::new(ErrorKind::LinkError, e.to_string());
        let data = match kind {
            "communication" => {
                Diagnostics::Communication(CommDiagnostics::decode(&r).map_err(bad)?)
            }
            "adc" => Diagnostics::Adc(AdcDiagnostics::decode(&r).map_err(bad)?),
            _ => Diagnostics::Fast(FastDiagnostics::decode(&r).map_err(bad)?),
        };
        Ok(DiagnosticsReport {
            trace_id,
            uid: self.uid,
            observed_at: now(),
            supported: true,
            data: Some(data),
            status: None,
        })
    }

    /// Compares communication counters with a previous snapshot of the same UID:
    /// uptime going backwards means a restart; CRC/UART counters growing means a noisy link.
    pub fn check_restart(
        &self,
        trace_id: &str,
        previous: Option<&CommDiagnostics>,
        current: &CommDiagnostics,
    ) {
        let Some(prev) = previous else { return };
        // The ms tick wraps after ~49.7 days; a drop from near the wrap point is ambiguous.
        const WRAP_S: u32 = 4_294_967;
        if current.uptime_s < prev.uptime_s && prev.uptime_s < WRAP_S - 3600 {
            let reason = [
                "UNKNOWN",
                "POWER_ON",
                "EXTERNAL_PIN",
                "SOFTWARE",
                "WATCHDOG",
                "BROWNOUT",
            ]
            .get(current.reset_reason as usize)
            .copied()
            .unwrap_or("UNKNOWN");
            self.raise(
                FaultCode::DevResetDetected,
                format!("device restarted (reset reason {reason}); uptime {} s -> {} s", prev.uptime_s, current.uptime_s),
                trace_id,
                json!({"reset_reason": reason, "previous_uptime_s": prev.uptime_s, "uptime_s": current.uptime_s}),
            );
        }
        if current.crc_error_count > prev.crc_error_count
            || current.uart_error_count > prev.uart_error_count
        {
            self.raise(
                FaultCode::LinkCrcError,
                "device counted new CRC/UART receive errors",
                trace_id,
                json!({"crc_errors": current.crc_error_count.saturating_sub(prev.crc_error_count), "uart_errors": current.uart_error_count.saturating_sub(prev.uart_error_count)}),
            );
        }
    }

    fn journal(
        &self,
        trace_id: &str,
        action: &str,
        args: Value,
        before: Option<&Status>,
        result: &Result<Value, OpError>,
    ) {
        let (outcome, error, after) = match result {
            Ok(v) => (
                v.get("outcome").cloned().unwrap_or(json!("completed")),
                Value::Null,
                v.get("status").cloned().unwrap_or(Value::Null),
            ),
            Err(e) => (
                json!(e.code),
                json!(e.message),
                e.details
                    .get("last_observed_status")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
        };
        self.recorder.operation(&json!({
            "at": now(), "trace_id": trace_id, "source": self.source, "uid": self.uid.to_string(), "port": self.port(),
            "action": action, "args": args, "outcome": outcome, "error": error,
            "before": before, "after": after,
            "faults": self.relevant_faults().iter().map(|f| f.code.as_str()).collect::<Vec<_>>(),
        }));
    }

    pub fn set_power(&mut self, command: Command) -> Result<PowerResult, OpError> {
        let trace_id = new_trace_id();
        let started = Instant::now();
        let mut before = None;
        let result = self.power_inner(&trace_id, command, started, &mut before);
        let journal = result
            .as_ref()
            .map(|r| serde_json::to_value(r).unwrap())
            .map_err(Clone::clone);
        self.journal(
            &trace_id,
            command.name(),
            json!({}),
            before.as_ref(),
            &journal,
        );
        result
    }

    fn power_inner(
        &mut self,
        trace_id: &str,
        command: Command,
        started: Instant,
        before_out: &mut Option<Status>,
    ) -> Result<PowerResult, OpError> {
        let before = self.read_status(trace_id)?;
        *before_out = Some(before.clone());
        let done = |this: &Self, outcome, acknowledged, status| PowerResult {
            trace_id: trace_id.to_string(),
            uid: this.uid,
            action: command,
            outcome,
            acknowledged,
            status,
            elapsed_ms: started.elapsed().as_millis() as u64,
            faults: this.relevant_faults(),
        };
        if stable(&before, command) {
            return Ok(done(self, Outcome::AlreadySatisfied, None, before));
        }
        let budget = if command == Command::Discharge {
            self.budgets.discharge
        } else {
            self.budgets.on_off
        };
        let deadline = Instant::now() + budget;
        let in_progress = before.last_command == command.code() && before.busy;
        let mut acknowledged = None;
        if !in_progress {
            let req = protocol::write_control(self.uid, command);
            match self.exchange(trace_id, &req, None) {
                Ok(Matched::Echo) => acknowledged = Some(true),
                Ok(Matched::Exception(e)) => return Err(exception_error(e).at(trace_id, self.uid)),
                Ok(Matched::Registers(_)) => unreachable!(),
                // Never resend after a lost ACK: confirm through status instead.
                Err(LinkError::Timeout(_)) => acknowledged = Some(false),
                Err(e) => {
                    return Err(OpError::new(
                        ErrorKind::Unknown,
                        "Power command outcome unknown; read status before taking another action",
                    )
                    .at(trace_id, self.uid)
                    .details(json!({"cause": e.to_string(), "action": command.name()})));
                }
            }
        }
        // Without an ACK, a failure latched by an earlier identical command is not
        // evidence about this one: the new command may never have arrived.
        let mut observed_this = acknowledged == Some(true) || in_progress;
        let mut status = before.clone();
        let mut last_read_error = None;
        while Instant::now() < deadline {
            match self.read_status(trace_id) {
                Ok(s) => {
                    status = s;
                    if status.last_command == command.code()
                        && (status.result_code == 1 || status.busy)
                    {
                        observed_this = true;
                    }
                    let stale_failure = !observed_this
                        && before.last_command == command.code()
                        && before.result_code == status.result_code
                        && before.status_flags == status.status_flags;
                    if status.last_command == command.code()
                        && !stale_failure
                        && (status.result_code >= 3
                            || status.internal_error
                            || status.discharge_timeout)
                    {
                        return Err(OpError::new(
                            ErrorKind::DeviceFailed,
                            format!("Device failed {}: {}", command.name(), status.last_result),
                        )
                        .at(trace_id, self.uid)
                        .details(
                            json!({"acknowledged": acknowledged, "last_observed_status": status}),
                        ));
                    }
                    if status.last_command == command.code()
                        && status.result_code == 2
                        && stable(&status, command)
                    {
                        return Ok(done(self, Outcome::Completed, acknowledged, status));
                    }
                }
                Err(e) if e.code == ErrorKind::LinkError && self.transport.broken.is_none() => {
                    last_read_error = Some(e.message)
                }
                Err(e) => {
                    return Err(OpError::new(ErrorKind::Unknown, "Power command completion unknown; the connection failed while confirming")
                        .at(trace_id, self.uid)
                        .details(json!({"acknowledged": acknowledged, "cause": e.message, "last_observed_status": status})));
                }
            }
            sleep(self.budgets.poll);
        }
        Err(OpError::new(ErrorKind::Unknown, "Power command completion unknown; a timeout does not cancel the device action. Read status next.")
            .at(trace_id, self.uid)
            .details(json!({"acknowledged": acknowledged, "action": command.name(), "last_observed_status": status, "last_read_error": last_read_error})))
    }

    /// OFF (default: discharge to a confirmed low VOUT), hold, then ON. Any failure stops the
    /// sequence; nothing turns ON after an error.
    pub fn cycle(&mut self, off_ms: u64, discharge: bool) -> Result<CycleResult, OpError> {
        if off_ms > 30_000 {
            return Err(OpError::new(
                ErrorKind::InvalidArgument,
                "off_ms must be 0..30000",
            ));
        }
        let trace_id = new_trace_id();
        let off = self.set_power(if discharge {
            Command::Discharge
        } else {
            Command::Off
        })?;
        sleep(Duration::from_millis(off_ms));
        let on = self.set_power(Command::On)?;
        let result = CycleResult {
            trace_id: trace_id.clone(),
            uid: self.uid,
            off,
            off_ms,
            on,
        };
        self.journal(
            &trace_id,
            "cycle",
            json!({"off_ms": off_ms, "discharge": discharge}),
            None,
            &Ok(json!({"outcome": "completed", "status": result.on.status})),
        );
        Ok(result)
    }

    pub fn set_nickname(&mut self, nickname: &str) -> Result<NicknameResult, OpError> {
        let req = protocol::write_nickname(self.uid, nickname)
            .map_err(|e| OpError::new(ErrorKind::InvalidArgument, e.to_string()))?;
        let trace_id = new_trace_id();
        let mut before = None;
        let result = (|| {
            let status = self.read_status(&trace_id)?;
            before = Some(status.clone());
            if status.state_code != 0
                || status.status_flags & (flags::OUTPUT | flags::DISCHARGE | flags::BUSY) != 0
            {
                return Err(OpError::new(
                    ErrorKind::Busy,
                    "Nickname changes require the output fully OFF and idle",
                )
                .at(&trace_id, self.uid));
            }
            let acknowledged =
                match self.exchange(&trace_id, &req, Some(self.budgets.nickname_reply)) {
                    Ok(Matched::Echo) => true,
                    Ok(Matched::Exception(e)) => {
                        return Err(exception_error(e).at(&trace_id, self.uid));
                    }
                    Ok(Matched::Registers(_)) => unreachable!(),
                    Err(LinkError::Timeout(_)) => false,
                    Err(e) => return Err(link_error(e).at(&trace_id, self.uid)),
                };
            let actual = self.nickname(&trace_id)?;
            if actual != nickname {
                let code = if acknowledged {
                    ErrorKind::DeviceFailed
                } else {
                    ErrorKind::Unknown
                };
                return Err(OpError::new(code, "Nickname readback does not match the request")
                    .at(&trace_id, self.uid)
                    .details(json!({"requested": nickname, "actual": actual, "acknowledged": acknowledged})));
            }
            Ok(NicknameResult {
                trace_id: trace_id.clone(),
                uid: self.uid,
                nickname: actual,
                acknowledged,
                verified: true,
            })
        })();
        let journal = result
            .as_ref()
            .map(|r| json!({"outcome": "completed", "nickname": r.nickname}))
            .map_err(Clone::clone);
        self.journal(
            &trace_id,
            "nickname",
            json!({"nickname": nickname}),
            before.as_ref(),
            &journal,
        );
        result
    }
}

fn exception_error(e: DeviceException) -> OpError {
    let kind = if e.code == 6 {
        ErrorKind::Busy
    } else {
        ErrorKind::DeviceFailed
    };
    OpError::new(kind, e.to_string())
        .details(json!({"exception_code": e.code, "exception": e.name()}))
}
fn link_error(e: LinkError) -> OpError {
    match e {
        LinkError::Busy(_) => OpError::new(ErrorKind::Busy, e.to_string()),
        _ => OpError::new(ErrorKind::LinkError, e.to_string()),
    }
}
