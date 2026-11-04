//! Simulated Voloo Switch (protocol 2.5, firmware 2.7 behavior) for tests and demos.
//! Not a hardware model: timings are configurable and voltages are idealized.
use crate::{
    link::Link,
    protocol::{self, UNIT, Uid, crc16, flags, with_crc},
};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct SimConfig {
    pub uid: Uid,
    pub vin_mv: u16,
    pub interlock: Duration,
    /// Time for VOUT to fall below 1 V once discharge starts (None: never falls).
    pub discharge_fall: Option<Duration>,
    pub discharge_limit: Duration,
    pub nickname: String,
}
impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            uid: "01-23-45-67-89-AB-CD-EF".parse().unwrap(),
            vin_mv: 7500,
            interlock: Duration::from_millis(10),
            discharge_fall: Some(Duration::from_millis(40)),
            discharge_limit: Duration::from_millis(5000),
            nickname: String::new(),
        }
    }
}

/// Faults to inject; each counter applies to the next N replies.
#[derive(Default, Debug, Clone)]
pub struct Inject {
    pub drop_replies: u32,
    /// Drop only the reply to the next control write (FC06).
    pub drop_control_ack: u32,
    pub corrupt_replies: u32,
    /// Deliver the next reply this late (simulates a reply arriving after timeout).
    pub delay_next: Option<Duration>,
    pub exception_next: Option<u8>,
    pub vin_invalid: bool,
    pub vout_invalid: bool,
    /// Treat control writes as received but never act on them (echo only).
    pub ignore_control: bool,
    pub unplugged: bool,
    /// Disconnect right after a control write is received (its reply is lost).
    pub unplug_after_control: bool,
    pub missing_adc_block: bool,
}

#[derive(Debug)]
pub struct SimState {
    pub cfg: SimConfig,
    pub inject: Inject,
    boot: Instant,
    pub power_state: u16,
    output: bool,
    discharge: bool,
    busy: bool,
    low_qualified: bool,
    discharge_timeout: bool,
    pub last_command: u16,
    pub result: u16,
    pending: Option<(u16, Instant)>,
    vout_mv: u16,
    nickname: String,
    pub writes: Vec<u16>,
    pub nickname_writes: u32,
    valid_requests: u32,
    crc_errors: u16,
    exceptions: u16,
    pub reset_reason: u16,
}

impl SimState {
    pub fn new(cfg: SimConfig) -> Self {
        SimState {
            nickname: cfg.nickname.clone(),
            cfg,
            inject: Inject::default(),
            boot: Instant::now(),
            power_state: 0,
            output: false,
            discharge: false,
            busy: false,
            low_qualified: false,
            discharge_timeout: false,
            last_command: 0,
            result: 0,
            pending: None,
            vout_mv: 0,
            writes: Vec::new(),
            nickname_writes: 0,
            valid_requests: 0,
            crc_errors: 0,
            exceptions: 0,
            reset_reason: 1,
        }
    }
    /// Simulates a restart: outputs OFF, counters and uptime reset.
    pub fn reset(&mut self, reason: u16) {
        let (cfg, inject) = (self.cfg.clone(), self.inject.clone());
        *self = SimState::new(cfg);
        self.inject = inject;
        self.reset_reason = reason;
    }
    pub fn uptime_s(&self) -> u32 {
        self.boot.elapsed().as_secs() as u32
    }
    /// Advances the command state machine to `now`.
    fn tick(&mut self) {
        let now = Instant::now();
        if self.output {
            self.vout_mv = self.cfg.vin_mv;
        }
        let Some((cmd, since)) = self.pending else {
            return;
        };
        match cmd {
            1 if now >= since + self.cfg.interlock => {
                self.output = true;
                self.power_state = 1;
                self.busy = false;
                self.result = 2;
                self.vout_mv = self.cfg.vin_mv;
                self.pending = None;
            }
            2 => {
                if now < since + self.cfg.interlock {
                    return;
                }
                self.discharge = true;
                self.power_state = 3;
                let elapsed = now - since - self.cfg.interlock;
                if self.inject.vout_invalid {
                    self.finish_discharge(4);
                } else if self.cfg.discharge_fall.is_some_and(|f| elapsed >= f) {
                    self.vout_mv = 300;
                    self.low_qualified = true;
                    self.finish_discharge(2);
                } else if elapsed >= self.cfg.discharge_limit {
                    self.discharge_timeout = true;
                    self.finish_discharge(3);
                }
            }
            _ => {}
        }
    }
    fn finish_discharge(&mut self, result: u16) {
        self.discharge = false;
        self.power_state = 0;
        self.busy = false;
        self.result = result;
        self.pending = None;
    }
    fn control(&mut self, cmd: u16) {
        self.writes.push(cmd);
        if self.inject.ignore_control {
            return;
        }
        self.tick();
        match cmd {
            1 if self.power_state == 1 || self.power_state == 2 => {}
            1 => {
                self.output = false;
                self.discharge = false;
                self.low_qualified = false;
                self.discharge_timeout = false;
                self.power_state = 2;
                self.busy = true;
                self.result = 1;
                self.last_command = 1;
                self.pending = Some((1, Instant::now()));
            }
            0 => {
                if !(self.power_state == 0 && !self.busy && self.last_command == 0) {
                    self.low_qualified = false;
                    self.discharge_timeout = false;
                }
                self.output = false;
                self.discharge = false;
                self.busy = false;
                self.power_state = 0;
                self.pending = None;
                self.last_command = 0;
                self.result = 2;
            }
            2 if self.pending.is_some_and(|(c, _)| c == 2) => {}
            2 => {
                self.output = false;
                self.discharge = false;
                self.low_qualified = false;
                self.discharge_timeout = false;
                self.power_state = 0;
                self.busy = true;
                self.result = 1;
                self.last_command = 2;
                self.pending = Some((2, Instant::now()));
            }
            _ => {}
        }
    }
    fn status_regs(&mut self) -> Vec<u16> {
        self.tick();
        let mut f = 0;
        if self.output {
            f |= flags::OUTPUT;
        }
        if self.discharge {
            f |= flags::DISCHARGE;
        }
        if self.busy {
            f |= flags::BUSY;
        }
        if !self.inject.vin_invalid {
            f |= flags::VIN_VALID;
        }
        if !self.inject.vout_invalid {
            f |= flags::VOUT_VALID;
        }
        if self.low_qualified {
            f |= flags::LOW_QUALIFIED;
        }
        if self.discharge_timeout {
            f |= flags::DISCHARGE_TIMEOUT;
        }
        let vin = if self.inject.vin_invalid {
            0
        } else {
            self.cfg.vin_mv
        };
        let vout = if self.inject.vout_invalid {
            0
        } else {
            self.vout_mv
        };
        vec![
            self.power_state,
            f,
            vin,
            vout,
            self.last_command,
            self.result,
        ]
    }
    fn read(&mut self, start: u16, count: u16) -> Option<Vec<u16>> {
        let block: Vec<u16> = match start {
            0x0000..=0x0005 => self.status_regs(),
            0x0100..=0x0108 => {
                let u = self.cfg.uid.0;
                vec![
                    0x0205,
                    1,
                    0x0207,
                    0x0300,
                    u16::from_be_bytes([u[0], u[1]]),
                    u16::from_be_bytes([u[2], u[3]]),
                    u16::from_be_bytes([u[4], u[5]]),
                    u16::from_be_bytes([u[6], u[7]]),
                    0x03FF,
                ]
            }
            0x0110..=0x011C => {
                let d = protocol::nickname_data(&self.nickname).unwrap();
                d.chunks(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect()
            }
            0x0200..=0x0207 => {
                let up = self.uptime_s();
                vec![
                    (up >> 16) as u16,
                    up as u16,
                    (self.valid_requests >> 16) as u16,
                    self.valid_requests as u16,
                    self.crc_errors,
                    0,
                    self.exceptions,
                    self.reset_reason,
                ]
            }
            0x0300..=0x030C if !self.inject.missing_adc_block => {
                let s = self.status_regs();
                let up = self.uptime_s();
                vec![
                    1,
                    0,
                    1,
                    16,
                    2325,
                    2325,
                    512,
                    s[0],
                    s[1],
                    s[2],
                    s[3],
                    (up >> 16) as u16,
                    up as u16,
                ]
            }
            0x0400..=0x040C => vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 500],
            _ => return None,
        };
        let base = match start {
            0x0000..=0x0005 => 0x0000,
            0x0100..=0x0108 => 0x0100,
            0x0110..=0x011C => 0x0110,
            0x0200..=0x0207 => 0x0200,
            0x0300..=0x030C => 0x0300,
            _ => 0x0400,
        };
        let from = (start - base) as usize;
        block.get(from..from + count as usize).map(<[u16]>::to_vec)
    }

    /// Handles one complete request frame; returns the reply (if any) before injection.
    pub fn handle(&mut self, frame: &[u8]) -> Option<Vec<u8>> {
        if frame.len() < 12 || frame[0] != UNIT {
            return None;
        }
        let n = frame.len();
        if crc16(&frame[..n - 2]) != u16::from_le_bytes([frame[n - 2], frame[n - 1]]) {
            self.crc_errors = self.crc_errors.saturating_add(1);
            return None;
        }
        if frame[2..10] != self.cfg.uid.0 {
            return None;
        }
        self.valid_requests += 1;
        let fc = frame[1];
        let mut head = vec![UNIT, fc];
        head.extend_from_slice(&self.cfg.uid.0);
        let exception = |this: &mut Self, code: u8| {
            this.exceptions = this.exceptions.saturating_add(1);
            let mut f = vec![UNIT, fc | 0x80];
            f.extend_from_slice(&this.cfg.uid.0);
            f.push(code);
            Some(with_crc(f))
        };
        if let Some(code) = self.inject.exception_next.take() {
            return exception(self, code);
        }
        let word = |i: usize| u16::from_be_bytes([frame[i], frame[i + 1]]);
        match fc {
            0x04 | 0x03 if n == 16 => {
                let (start, count) = (word(10), word(12));
                let regs = if fc == 0x03 {
                    (start == 0 && count == 1).then(|| vec![self.last_command])
                } else {
                    self.read(start, count)
                };
                match regs {
                    Some(r) => {
                        head.push((r.len() * 2) as u8);
                        for v in r {
                            head.extend_from_slice(&v.to_be_bytes());
                        }
                        Some(with_crc(head))
                    }
                    None => exception(self, 2),
                }
            }
            0x06 if n == 16 => {
                if word(10) != 0 || word(12) > 2 {
                    return exception(self, if word(10) != 0 { 2 } else { 3 });
                }
                self.control(word(12));
                Some(frame.to_vec())
            }
            0x10 if n == 43 => {
                if word(10) != 0x0110 || word(12) != 13 || frame[14] != 26 {
                    return exception(self, 2);
                }
                let len = word(15) as usize;
                let Ok(name) = std::str::from_utf8(&frame[17..17 + len.min(24)]) else {
                    return exception(self, 3);
                };
                self.nickname = name.to_string();
                self.nickname_writes += 1;
                head.extend_from_slice(&frame[10..14]);
                Some(with_crc(head))
            }
            _ => exception(self, 1),
        }
    }
}

pub type Shared = Arc<Mutex<SimState>>;

pub fn shared(cfg: SimConfig) -> Shared {
    Arc::new(Mutex::new(SimState::new(cfg)))
}

/// Request frame length from its header (requests only).
pub fn request_len(buf: &[u8]) -> Option<usize> {
    match buf.get(1)? {
        0x03 | 0x04 | 0x06 => Some(16),
        0x10 => Some(43),
        _ => Some(1),
    }
}

/// Splits incoming bytes into requests and produces delayed/corrupted/dropped replies.
pub struct Responder {
    pub state: Shared,
    rx: Vec<u8>,
    queue: VecDeque<(Instant, Vec<u8>)>,
}
impl Responder {
    pub fn new(state: Shared) -> Self {
        Responder {
            state,
            rx: Vec::new(),
            queue: VecDeque::new(),
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        self.rx.extend_from_slice(bytes);
        while !self.rx.is_empty() {
            if self.rx[0] != UNIT {
                self.rx.remove(0);
                continue;
            }
            let Some(len) = request_len(&self.rx) else {
                break;
            };
            if self.rx.len() < len {
                break;
            }
            let frame: Vec<u8> = self.rx.drain(..len).collect();
            let mut s = self.state.lock().unwrap();
            let Some(mut reply) = s.handle(&frame) else {
                continue;
            };
            if frame[1] == 0x06 && s.inject.unplug_after_control {
                s.inject.unplugged = true;
                continue;
            }
            if frame[1] == 0x06 && s.inject.drop_control_ack > 0 {
                s.inject.drop_control_ack -= 1;
                continue;
            }
            if s.inject.drop_replies > 0 {
                s.inject.drop_replies -= 1;
                continue;
            }
            if s.inject.corrupt_replies > 0 {
                s.inject.corrupt_replies -= 1;
                let last = reply.len() - 1;
                reply[last] ^= 0xFF;
            }
            let at = Instant::now() + s.inject.delay_next.take().unwrap_or_default();
            self.queue.push_back((at, reply));
        }
    }
    /// Replies that are due by now.
    pub fn due(&mut self) -> Vec<u8> {
        let now = Instant::now();
        let mut out = Vec::new();
        while self.queue.front().is_some_and(|(at, _)| *at <= now) {
            out.extend(self.queue.pop_front().unwrap().1);
        }
        out
    }
    pub fn next_due(&self) -> Option<Instant> {
        self.queue.front().map(|(at, _)| *at)
    }
}

/// In-process link to a simulated device (no OS serial port).
pub struct SimLink {
    responder: Responder,
    pending: VecDeque<u8>,
    path: String,
}
impl SimLink {
    pub fn new(state: Shared) -> Self {
        Self::at(state, "sim://0")
    }
    pub fn at(state: Shared, path: &str) -> Self {
        SimLink {
            responder: Responder::new(state),
            pending: VecDeque::new(),
            path: path.into(),
        }
    }
    fn unplugged(&self) -> bool {
        self.responder.state.lock().unwrap().inject.unplugged
    }
}
impl Link for SimLink {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.unplugged() {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        self.responder.feed(bytes);
        Ok(())
    }
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.unplugged() {
                return Err(io::Error::from(io::ErrorKind::BrokenPipe));
            }
            self.pending.extend(self.responder.due());
            if !self.pending.is_empty() {
                let n = buf.len().min(self.pending.len());
                for (i, b) in self.pending.drain(..n).enumerate() {
                    buf[i] = b;
                }
                return Ok(n);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(0);
            }
            let wake = self
                .responder
                .next_due()
                .map_or(deadline, |d| d.min(deadline));
            std::thread::sleep(
                wake.saturating_duration_since(now)
                    .min(Duration::from_millis(1)),
            );
        }
    }
    fn path(&self) -> &str {
        &self.path
    }
}

/// Demo devices living inside one process, addressed as `sim://1` … `sim://8`.
/// They keep their state while the process runs, like real devices across reconnects.
pub fn demo_devices(count: u8) -> Vec<(String, Uid)> {
    (1..=count.clamp(1, 8))
        .map(|i| (format!("sim://{i}"), demo_config(i).uid))
        .collect()
}
fn demo_config(i: u8) -> SimConfig {
    let names = [
        "bench-a", "camera", "router", "", "dut-5", "dut-6", "dut-7", "dut-8",
    ];
    let vin = [12000, 5000, 24000, 7500, 3300, 9000, 15000, 48000];
    let k = (i.clamp(1, 8) - 1) as usize;
    SimConfig {
        uid: Uid([0x5E, 0x11, 0xD0, 0, 0, 0, 0, i]),
        nickname: names[k].into(),
        vin_mv: vin[k],
        discharge_fall: Some(Duration::from_millis(400)),
        ..SimConfig::default()
    }
}
/// Opens a link to the demo device at `path` (`sim://N`), creating it on first use.
pub fn demo_link(path: &str) -> Option<SimLink> {
    use std::{collections::HashMap, sync::OnceLock};
    static DEVICES: OnceLock<Mutex<HashMap<String, Shared>>> = OnceLock::new();
    let i: u8 = path
        .strip_prefix("sim://")?
        .parse()
        .ok()
        .filter(|i| (1..=8).contains(i))?;
    let state = DEVICES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(path.to_string())
        .or_insert_with(|| shared(demo_config(i)))
        .clone();
    Some(SimLink::at(state, path))
}

/// Serves a simulated device over a byte stream (e.g. a PTY) until `stop` is set.
#[cfg(unix)]
pub fn serve_tty(
    mut port: serialport::TTYPort,
    state: Shared,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    use serialport::SerialPort;
    use std::io::{Read, Write};
    let mut responder = Responder::new(state);
    port.set_timeout(Duration::from_millis(1)).ok();
    let mut buf = [0u8; 256];
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        if let Ok(n) = port.read(&mut buf) {
            responder.feed(&buf[..n]);
        }
        let out = responder.due();
        if !out.is_empty() {
            let _ = port.write_all(&out);
            let _ = port.flush();
        }
    }
}
