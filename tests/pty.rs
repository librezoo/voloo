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

#[test]
fn simulator_exits_with_an_unread_reply_after_client_close() {
    use serialport::SerialPort;
    use std::{io::Write, sync::mpsc, time::Duration};
    let (master, guard) = serialport::TTYPort::pair().unwrap();
    let mut client = guard.try_clone().unwrap();
    let cfg = SimConfig::default();
    let uid = cfg.uid;
    let state = shared(cfg);
    state.lock().unwrap().inject.delay_next = Some(Duration::from_millis(100));
    let stop = Arc::new(AtomicBool::new(false));
    let (done, exited) = mpsc::channel();
    let server = {
        let (state, stop) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            serve_tty(master, state, stop);
            let _ = done.send(());
        })
    };
    client
        .write_all(&voloo::protocol::read_input(uid, 0, 6).unwrap().frame)
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while state.lock().unwrap().inject.delay_next.is_some() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(client); // The original slave remains alive but nobody consumes the reply.
    std::thread::sleep(Duration::from_millis(150));
    stop.store(true, Ordering::Relaxed);
    let stopped = exited.recv_timeout(Duration::from_secs(1)).is_ok();
    drop(guard);
    assert!(stopped, "unread PTY response blocked simulator shutdown");
    server.join().unwrap();
}
