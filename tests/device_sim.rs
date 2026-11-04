//! Device semantics and fault reporting against the in-process simulator.
use std::{sync::Arc, time::Duration};
use voloo::{
    device::{Device, ErrorKind, Outcome},
    link::Transport,
    observe::{FaultCode, Level, Limits, Recorder, read_log},
    protocol::{CommDiagnostics, Command},
    sim::{Shared, SimConfig, SimLink, shared},
};

fn setup(cfg: SimConfig) -> (Device, Shared, Arc<Recorder>) {
    setup_with(cfg, Arc::new(Recorder::memory(Level::Debug)))
}
fn setup_with(cfg: SimConfig, recorder: Arc<Recorder>) -> (Device, Shared, Arc<Recorder>) {
    let uid = cfg.uid;
    let state = shared(cfg);
    let mut transport = Transport::new(Box::new(SimLink::new(state.clone())), recorder.clone());
    transport.timing.response = Duration::from_millis(60);
    let device = Device::new(transport, uid, recorder.clone());
    (device, state, recorder)
}
fn active(recorder: &Recorder, code: FaultCode) -> bool {
    recorder.faults(None, true).iter().any(|f| f.code == code)
}
fn ever(recorder: &Recorder, code: FaultCode) -> bool {
    recorder.faults(None, false).iter().any(|f| f.code == code)
}

#[test]
fn on_off_and_already_satisfied() {
    let (mut d, state, _) = setup(SimConfig::default());
    let on = d.set_power(Command::On).unwrap();
    assert_eq!(
        (on.outcome, on.acknowledged),
        (Outcome::Completed, Some(true))
    );
    assert!(on.status.output_enabled && on.status.state_code == 1);
    let again = d.set_power(Command::On).unwrap();
    assert_eq!(
        (again.outcome, again.acknowledged),
        (Outcome::AlreadySatisfied, None)
    );
    let off = d.set_power(Command::Off).unwrap();
    assert_eq!(off.outcome, Outcome::Completed);
    assert_eq!(state.lock().unwrap().writes, vec![1, 0], "no extra writes");
}

#[test]
fn lost_ack_is_confirmed_by_status_without_resending() {
    let (mut d, state, rec) = setup(SimConfig::default());
    state.lock().unwrap().inject.drop_control_ack = 1;
    let r = d.set_power(Command::On).unwrap();
    assert_eq!(
        (r.outcome, r.acknowledged),
        (Outcome::Completed, Some(false))
    );
    assert_eq!(
        state.lock().unwrap().writes,
        vec![1],
        "a lost ACK must not cause a resend"
    );
    assert!(ever(&rec, FaultCode::LinkResponseTimeout));
    assert!(
        !active(&rec, FaultCode::LinkResponseTimeout),
        "cleared by later successful reads"
    );
}

#[test]
fn echo_without_effect_is_unknown_not_success() {
    let (mut d, state, _) = setup(SimConfig::default());
    d.budgets.on_off = Duration::from_millis(200);
    state.lock().unwrap().inject.ignore_control = true;
    let e = d.set_power(Command::On).unwrap_err();
    assert_eq!(e.code, ErrorKind::Unknown);
    assert!(e.trace_id.is_some());
    assert_eq!(state.lock().unwrap().writes, vec![1]);
}

#[test]
fn discharge_confirms_low_vout() {
    let (mut d, _, _) = setup(SimConfig::default());
    d.set_power(Command::On).unwrap();
    let r = d.set_power(Command::Discharge).unwrap();
    assert_eq!(r.outcome, Outcome::Completed);
    assert!(r.status.low_qualified && r.status.vout_mv.unwrap() <= 1000);
}

#[test]
fn discharge_timeout_fails_and_cycle_never_turns_on() {
    let cfg = SimConfig {
        discharge_fall: None,
        discharge_limit: Duration::from_millis(150),
        ..SimConfig::default()
    };
    let (mut d, state, rec) = setup(cfg);
    d.set_power(Command::On).unwrap();
    let e = d.cycle(0, true).unwrap_err();
    assert_eq!(e.code, ErrorKind::DeviceFailed);
    assert_eq!(
        state.lock().unwrap().writes,
        vec![1, 2],
        "no ON after a failed discharge"
    );
    assert!(active(&rec, FaultCode::DevDischargeTimeout));
    let fault = rec
        .faults(None, true)
        .into_iter()
        .find(|f| f.code == FaultCode::DevDischargeTimeout)
        .unwrap();
    assert!(fault.suggestion.contains("5 s"));
    assert_eq!(e.code.exit_code(), 2);
}

#[test]
fn invalid_vout_during_discharge_is_reported() {
    let (mut d, state, rec) = setup(SimConfig::default());
    state.lock().unwrap().inject.vout_invalid = true;
    let e = d.set_power(Command::Discharge).unwrap_err();
    assert_eq!(e.code, ErrorKind::DeviceFailed);
    assert!(active(&rec, FaultCode::DevMeasurementInvalid));
    assert!(active(&rec, FaultCode::DevVoutInvalid));
    let s = d.status().unwrap();
    assert_eq!(s.status.vout_mv, None, "invalid VOUT is null, not 0 V");
}

#[test]
fn measurement_faults_clear_when_valid_again() {
    let (mut d, state, rec) = setup(SimConfig::default());
    state.lock().unwrap().inject.vin_invalid = true;
    let s = d.status().unwrap();
    assert_eq!(s.status.vin_mv, None);
    assert!(s.faults.iter().any(|f| f.code == FaultCode::DevVinInvalid));
    d.status().unwrap();
    assert_eq!(
        rec.faults(None, true)
            .iter()
            .find(|f| f.code == FaultCode::DevVinInvalid)
            .unwrap()
            .count,
        2,
        "de-duplicated"
    );
    state.lock().unwrap().inject.vin_invalid = false;
    assert!(d.status().unwrap().faults.is_empty());
    assert!(ever(&rec, FaultCode::DevVinInvalid));
}

#[test]
fn device_exceptions_map_to_failed_and_busy() {
    let (mut d, state, rec) = setup(SimConfig::default());
    state.lock().unwrap().inject.exception_next = Some(4);
    assert_eq!(d.status().unwrap_err().code, ErrorKind::DeviceFailed);
    assert!(ever(&rec, FaultCode::DevException));
    state.lock().unwrap().inject.exception_next = Some(6);
    assert_eq!(d.status().unwrap_err().code, ErrorKind::Busy);
    assert!(ever(&rec, FaultCode::DevBusy));
    let s = d.status().unwrap();
    assert!(
        s.faults.iter().any(|f| f.code == FaultCode::DevException),
        "recent occurrences are reported"
    );
    assert!(!active(&rec, FaultCode::DevException));
}

#[test]
fn corrupted_reply_times_out_with_crc_fault() {
    let (mut d, state, rec) = setup(SimConfig::default());
    state.lock().unwrap().inject.corrupt_replies = 1;
    assert_eq!(d.status().unwrap_err().code, ErrorKind::LinkError);
    assert!(ever(&rec, FaultCode::LinkCrcError));
    assert!(d.status().is_ok(), "link recovers");
}

#[test]
fn late_reply_is_discarded_and_reported() {
    let (mut d, state, rec) = setup(SimConfig::default());
    state.lock().unwrap().inject.delay_next = Some(Duration::from_millis(150));
    // Identity reply arrives after its timeout and recovery; the next status read must not take it.
    assert!(d.info().is_err());
    std::thread::sleep(Duration::from_millis(120));
    let s = d.status().unwrap();
    assert_eq!(s.status.state_code, 0);
    assert!(ever(&rec, FaultCode::LinkLateFrame));
}

#[test]
fn unplug_is_reported_and_power_outcome_unknown() {
    let (mut d, state, rec) = setup(SimConfig::default());
    d.status().unwrap();
    state.lock().unwrap().inject.unplugged = true;
    let e = d.set_power(Command::On).unwrap_err();
    assert_eq!(
        e.code,
        ErrorKind::LinkError,
        "unplugged before any write: nothing was sent"
    );
    assert!(
        active(&rec, FaultCode::LinkUnplugged),
        "unplugged is a condition"
    );
    assert!(state.lock().unwrap().writes.is_empty());
    assert!(d.transport.broken.is_some());
}

#[test]
fn unplug_after_command_is_unknown_and_not_resent() {
    let (mut d, state, rec) = setup(SimConfig::default());
    state.lock().unwrap().inject.unplug_after_control = true;
    let e = d.set_power(Command::On).unwrap_err();
    assert_eq!(e.code, ErrorKind::Unknown, "the command may have executed");
    assert_eq!(e.code.exit_code(), 3);
    assert!(e.message.contains("status"));
    assert_eq!(state.lock().unwrap().writes, vec![1]);
    assert!(active(&rec, FaultCode::LinkUnplugged));
}

#[test]
fn nickname_requires_off_and_verifies_readback() {
    let (mut d, state, _) = setup(SimConfig::default());
    let r = d.set_nickname("电源🔌").unwrap();
    assert!(r.verified && r.acknowledged);
    assert_eq!(d.info().unwrap().nickname.as_deref(), Some("电源🔌"));
    d.set_power(Command::On).unwrap();
    assert_eq!(d.set_nickname("x").unwrap_err().code, ErrorKind::Busy);
    assert_eq!(state.lock().unwrap().nickname_writes, 1);
    assert_eq!(
        d.set_nickname(&"电".repeat(9)).unwrap_err().code,
        ErrorKind::InvalidArgument
    );
}

#[test]
fn optional_adc_block_falls_back_without_fault() {
    let (mut d, state, rec) = setup(SimConfig::default());
    state.lock().unwrap().inject.missing_adc_block = true;
    let r = d.diagnostics("adc").unwrap();
    assert!(!r.supported && r.status.is_some());
    assert!(
        !ever(&rec, FaultCode::DevException),
        "an absent optional block is not a fault"
    );
    assert!(d.diagnostics("communication").unwrap().supported);
    assert_eq!(
        d.diagnostics("bogus").unwrap_err().code,
        ErrorKind::InvalidArgument
    );
}

#[test]
fn restart_detection_from_uptime() {
    let (d, _, rec) = setup(SimConfig::default());
    let prev = CommDiagnostics {
        uptime_s: 500,
        valid_request_count: 10,
        crc_error_count: 0,
        uart_error_count: 0,
        exception_response_count: 0,
        reset_reason: 1,
    };
    let cur = CommDiagnostics {
        uptime_s: 3,
        reset_reason: 4,
        ..prev.clone()
    };
    d.check_restart("t", Some(&prev), &cur);
    let f = rec
        .faults(None, false)
        .into_iter()
        .find(|f| f.code == FaultCode::DevResetDetected)
        .unwrap();
    assert!(f.message.contains("WATCHDOG"));
    let noisy = CommDiagnostics {
        uptime_s: 600,
        crc_error_count: 2,
        ..prev.clone()
    };
    d.check_restart("t", Some(&prev), &noisy);
    assert!(ever(&rec, FaultCode::LinkCrcError));
}

#[test]
fn spans_logs_and_journal() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Arc::new(
        Recorder::with_dir(Level::Debug, dir.path().to_path_buf(), Limits::default()).unwrap(),
    );
    let (mut d, state, rec) = setup_with(SimConfig::default(), rec);
    let r = d.set_power(Command::On).unwrap();
    let spans = rec.spans(Some(&r.trace_id), 100);
    assert!(spans.len() >= 3, "status, control, confirm");
    assert!(
        spans
            .iter()
            .any(|s| s.label == "control on" && s.tx_hex.is_some())
    );
    state.lock().unwrap().inject.vin_invalid = true;
    d.status().unwrap();
    rec.flush();
    let ops = read_log(dir.path(), "operations", Some(&r.uid.to_string()), 10);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["action"], "on");
    assert_eq!(ops[0]["outcome"], "completed");
    assert_eq!(ops[0]["source"], "cli");
    let events = read_log(dir.path(), "events", None, 10);
    assert!(
        events
            .iter()
            .any(|e| e["event"] == "fault.raised" && e["code"] == "DEV_VIN_INVALID")
    );
    assert!(!read_log(dir.path(), "trace", None, 10).is_empty());
}

#[test]
fn info_level_omits_raw_frames() {
    let (mut d, _, _) = setup_with(
        SimConfig::default(),
        Arc::new(Recorder::memory(Level::Info)),
    );
    let r = d.status().unwrap();
    assert!(
        d.recorder
            .spans(Some(&r.trace_id), 10)
            .iter()
            .all(|s| s.tx_hex.is_none())
    );
}

#[test]
fn log_pruning_respects_size_cap() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..5 {
        std::fs::write(
            dir.path().join(format!("trace-2026010{i}.jsonl")),
            vec![b'x'; 1000],
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    let limits = Limits {
        max_total_bytes: 2500,
        ..Limits::default()
    };
    Recorder::with_dir(Level::Info, dir.path().to_path_buf(), limits).unwrap();
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
    assert_eq!(left.len(), 2, "oldest files removed until under the cap");
}

#[test]
fn conditions_survive_restarts_until_observed_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let open = || {
        Arc::new(
            Recorder::with_dir(Level::Info, dir.path().to_path_buf(), Limits::default()).unwrap(),
        )
    };
    let cfg = SimConfig::default();
    let state = shared(cfg.clone());
    let device = |rec: Arc<Recorder>| {
        Device::new(
            Transport::new(Box::new(SimLink::new(state.clone())), rec.clone()),
            cfg.uid,
            rec,
        )
    };
    state.lock().unwrap().inject.vin_invalid = true;
    device(open()).status().unwrap();
    // A new process (fresh recorder) still knows the condition is active...
    let rec = open();
    assert!(
        rec.faults(None, true)
            .iter()
            .any(|f| f.code == FaultCode::DevVinInvalid)
    );
    // ...and records the clear when it observes VIN valid again.
    state.lock().unwrap().inject.vin_invalid = false;
    device(rec).status().unwrap();
    let events = read_log(dir.path(), "events", None, 10);
    assert_eq!(events.last().unwrap()["event"], "fault.cleared");
    assert!(open().faults(None, true).is_empty());
}
