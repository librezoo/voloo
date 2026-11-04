//! Exercises the real serial link (termios, timeouts, partial reads) over an OS pseudo-terminal.
#![cfg(unix)]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use voloo::{
    device::Outcome,
    discover,
    link::{SerialLink, Transport},
    observe::{Level, Recorder},
    protocol::Command,
    sim::{SimConfig, serve_tty, shared},
};

#[test]
fn serial_link_over_pty() {
    let (master, slave) = serialport::TTYPort::pair().expect("create PTY pair");
    let cfg = SimConfig {
        nickname: "bench-a".into(),
        ..SimConfig::default()
    };
    let uid = cfg.uid;
    let state = shared(cfg);
    let stop = Arc::new(AtomicBool::new(false));
    let server = {
        let (state, stop) = (state.clone(), stop.clone());
        std::thread::spawn(move || serve_tty(master, state, stop))
    };
    let recorder = Arc::new(Recorder::memory(Level::Info));
    let transport = Transport::new(
        Box::new(SerialLink::from_tty(slave, "pty")),
        recorder.clone(),
    );
    let mut device = discover::attach(transport, Some(uid), &recorder).expect("identity over PTY");
    assert_eq!(device.info().unwrap().nickname.as_deref(), Some("bench-a"));
    let on = device.set_power(Command::On).unwrap();
    assert_eq!(on.outcome, Outcome::Completed);
    let off = device.cycle(10, true).unwrap();
    assert!(off.on.status.output_enabled);
    assert_eq!(state.lock().unwrap().writes, vec![1, 2, 1]);
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
}
