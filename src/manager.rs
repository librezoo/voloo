//! Device manager for the background process: one worker thread per Voloo owns its
//! serial port exclusively and runs jobs one at a time; a scanner thread follows USB
//! hot-plug; every change is broadcast as an event (for SSE, the web page and MCP).
use crate::{
    config::Config,
    device::{Device, Diagnostics, ErrorKind, OpError},
    discover, firmware, home,
    link::LinkStats,
    observe::{Fault, FaultCode, Recorder, new_trace_id, now},
    protocol::{Command, Identity, Status, Uid},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, oneshot};

#[derive(Clone, Debug)]
pub enum Job {
    Status,
    Info,
    Diagnostics(String),
    Power(Command),
    Cycle { off_ms: u64, discharge: bool },
    Nickname(String),
    Firmware(firmware::Image),
    FirmwareOnline,
    FirmwareRollback,
    FirmwareStatus,
}
impl Job {
    pub fn writes(&self) -> bool {
        matches!(
            self,
            Job::Power(_)
                | Job::Cycle { .. }
                | Job::Nickname(_)
                | Job::Firmware(_)
                | Job::FirmwareOnline
                | Job::FirmwareRollback
        )
    }
    fn maintenance(&self) -> bool {
        matches!(
            self,
            Job::Firmware(_) | Job::FirmwareOnline | Job::FirmwareRollback
        )
    }
    fn action(&self) -> String {
        match self {
            Job::Status => "status".into(),
            Job::Info => "info".into(),
            Job::Diagnostics(k) => format!("diagnostics:{k}"),
            Job::Power(c) => c.name().into(),
            Job::Cycle { .. } => "cycle".into(),
            Job::Nickname(_) => "nickname".into(),
            Job::Firmware(_) | Job::FirmwareOnline => "firmware_update".into(),
            Job::FirmwareRollback => "firmware_rollback".into(),
            Job::FirmwareStatus => "firmware_status".into(),
        }
    }
}

struct Request {
    job: Job,
    source: &'static str,
    reply: oneshot::Sender<Result<Value, OpError>>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Metrics {
    pub polls: u64,
    pub poll_errors: u64,
    pub status_samples: usize,
    pub status_p50_us: Option<u64>,
    pub status_p99_us: Option<u64>,
    pub link: LinkStats,
    pub device_uptime_s: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub uid: Uid,
    pub port: String,
    /// False when unplugged or the port cannot be opened: the output state is then unknown.
    pub online: bool,
    pub locked: bool,
    /// "config" (config.toml, cannot be undone from clients) or "runtime" (set via API/UI).
    pub lock_source: Option<String>,
    pub nickname: Option<String>,
    pub identity: Option<Identity>,
    pub status: Option<Status>,
    pub observed_at: Option<String>,
    pub error: Option<OpError>,
    pub last_operation: Option<Value>,
    pub faults: Vec<Fault>,
    pub metrics: Metrics,
    pub firmware: Option<firmware::Progress>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub kind: String,
    pub at: String,
    pub data: Value,
}

struct Entry {
    snapshot: Arc<Mutex<Snapshot>>,
    tx: Option<mpsc::Sender<Request>>,
    alive: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    updating: Arc<AtomicBool>,
}

pub struct Manager {
    pub recorder: Arc<Recorder>,
    pub config: Config,
    entries: Mutex<BTreeMap<Uid, Entry>>,
    pub events: broadcast::Sender<Event>,
    /// Open SSE streams; while non-zero, devices are polled at the fast rate.
    pub watchers: AtomicUsize,
    stop: AtomicBool,
    threads: Mutex<Vec<JoinHandle<()>>>,
    /// Locks set at runtime (web page or API), persisted in `$VOLOO_HOME/locks.json`.
    runtime_locks: Mutex<std::collections::BTreeSet<Uid>>,
}

impl Manager {
    pub fn new(recorder: Arc<Recorder>, config: Config) -> Arc<Self> {
        let (events, _) = broadcast::channel(1024);
        let manager = Arc::new(Manager {
            recorder: recorder.clone(),
            config,
            entries: Mutex::new(BTreeMap::new()),
            events,
            watchers: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            threads: Mutex::new(Vec::new()),
            runtime_locks: Mutex::new(load_locks()),
        });
        let tx = manager.events.clone();
        recorder.set_listener(Box::new(move |kind, fault| {
            let _ = tx.send(Event {
                kind: kind.into(),
                at: now(),
                data: json!(fault),
            });
        }));
        manager
    }

    pub fn publish(&self, kind: &str, data: Value) {
        let _ = self.events.send(Event {
            kind: kind.into(),
            at: now(),
            data,
        });
    }

    /// Starts USB scanning. Returns immediately.
    pub fn start(self: &Arc<Self>) {
        // First pass synchronously, so the first request already sees connected devices.
        self.scan_once();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline
            && self.entries.lock().unwrap().values().any(|e| {
                let s = e.snapshot.lock().unwrap();
                e.alive.load(Ordering::SeqCst) && !s.online && s.error.is_none()
            })
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        let this = self.clone();
        let handle = std::thread::Builder::new()
            .name("voloo-scan".into())
            .spawn(move || this.scan_loop())
            .expect("spawn scanner");
        self.threads.lock().unwrap().push(handle);
    }

    /// Stops scanning and all workers. Device outputs are left as they are.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        for e in self.entries.lock().unwrap().values_mut() {
            e.stop.store(true, Ordering::SeqCst);
            e.tx = None;
        }
        let threads = std::mem::take(&mut *self.threads.lock().unwrap());
        for t in threads {
            let _ = t.join();
        }
        self.recorder.flush();
    }

    fn candidates(&self) -> BTreeMap<Uid, String> {
        let mut out = BTreeMap::new();
        if !self.config.scan_usb {
            // Only configured ports.
        } else if let Ok(ports) = discover::list_ports() {
            for p in ports.into_iter().filter(|p| p.is_voloo) {
                if let Some(uid) = p.uid {
                    out.insert(uid, p.path);
                }
            }
        }
        for (path, uid) in &self.config.demo {
            out.insert(*uid, path.clone());
        }
        for (path, uid) in &self.config.static_ports {
            if std::path::Path::new(path).exists() {
                out.insert(*uid, path.clone());
            }
        }
        out
    }

    fn scan_loop(self: Arc<Self>) {
        while !self.stop.load(Ordering::SeqCst) {
            self.scan_once();
            // Trace spans are buffered; write them about once a second so a crash or a
            // `voloo report` taken while running loses at most the last second.
            self.recorder.flush();
            for _ in 0..10 {
                if self.stop.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    /// One hot-plug pass: start workers for new devices, mark vanished ones offline.
    pub fn scan_once(self: &Arc<Self>) {
        let present = self.candidates();
        let mut entries = self.entries.lock().unwrap();
        for (uid, path) in &present {
            if entries
                .get(uid)
                .is_some_and(|e| e.updating.load(Ordering::SeqCst))
            {
                continue;
            }
            let restart = match entries.get(uid) {
                Some(e) => !e.alive.load(Ordering::SeqCst),
                None => true,
            };
            if !restart {
                continue;
            }
            let entry = entries.entry(*uid).or_insert_with(|| Entry {
                snapshot: Arc::new(Mutex::new(Snapshot {
                    uid: *uid,
                    port: path.clone(),
                    online: false,
                    locked: false,
                    lock_source: None,
                    nickname: None,
                    identity: None,
                    status: None,
                    observed_at: None,
                    error: None,
                    last_operation: None,
                    faults: Vec::new(),
                    metrics: Metrics::default(),
                    firmware: None,
                })),
                tx: None,
                alive: Arc::new(AtomicBool::new(false)),
                stop: Arc::new(AtomicBool::new(false)),
                updating: Arc::new(AtomicBool::new(false)),
            });
            entry.snapshot.lock().unwrap().port = path.clone();
            let (tx, rx) = mpsc::channel();
            entry.tx = Some(tx);
            entry.alive.store(true, Ordering::SeqCst);
            entry.stop = Arc::new(AtomicBool::new(false));
            let worker = Worker {
                misses: Default::default(),
                uid: *uid,
                path: path.clone(),
                manager: self.clone(),
                snapshot: entry.snapshot.clone(),
                alive: entry.alive.clone(),
                stop: entry.stop.clone(),
                updating: entry.updating.clone(),
            };
            let handle = std::thread::Builder::new()
                .name(format!("voloo-{uid}"))
                .spawn(move || worker.run(rx))
                .expect("spawn worker");
            self.threads.lock().unwrap().push(handle);
        }
        for (uid, e) in entries.iter_mut() {
            if !present.contains_key(uid)
                && e.alive.load(Ordering::SeqCst)
                && !e.updating.load(Ordering::SeqCst)
            {
                e.stop.store(true, Ordering::SeqCst);
                e.tx = None;
                let port = e.snapshot.lock().unwrap().port.clone();
                // The worker reports device.removed when it stops; record why it went away.
                self.recorder.raise(
                    FaultCode::LinkUnplugged,
                    Some(uid.to_string()),
                    Some(port),
                    "USB port disappeared (cable unplugged or device reset)",
                    None,
                    Value::Null,
                );
            }
        }
        // Reap finished worker threads so a long-running process does not accumulate handles.
        let mut threads = self.threads.lock().unwrap();
        let (done, running): (Vec<_>, Vec<_>) = std::mem::take(&mut *threads)
            .into_iter()
            .partition(|t| t.is_finished());
        *threads = running;
        drop(threads);
        for t in done {
            let _ = t.join();
        }
    }

    /// Lock source if writes to this device are refused.
    pub fn lock_source(&self, uid: &Uid) -> Option<&'static str> {
        if self.config.locked.contains(uid) {
            Some("config")
        } else if self.runtime_locks.lock().unwrap().contains(uid) {
            Some("runtime")
        } else {
            None
        }
    }

    /// Locks or unlocks a device for every client. Config locks cannot be lifted here.
    pub fn set_lock(&self, id: &str, locked: bool) -> Result<Value, OpError> {
        let uid = match id.parse::<Uid>() {
            Ok(uid) => uid,
            Err(_) => {
                let named: Vec<Uid> = self
                    .devices()
                    .into_iter()
                    .filter(|d| d.nickname.as_deref() == Some(id))
                    .map(|d| d.uid)
                    .collect();
                match named.as_slice() {
                    [one] => *one,
                    [] => {
                        return Err(OpError::new(
                            ErrorKind::NotFound,
                            format!("No Voloo is named {id:?}"),
                        ));
                    }
                    _ => {
                        return Err(OpError::new(
                            ErrorKind::InvalidArgument,
                            format!("Several devices are named {id:?}; use the UID"),
                        ));
                    }
                }
            }
        };
        if !locked && self.config.locked.contains(&uid) {
            return Err(OpError::new(
                ErrorKind::InvalidArgument,
                format!("Voloo {uid} is locked in config.toml; edit [devices] there to unlock it"),
            ));
        }
        {
            let mut locks = self.runtime_locks.lock().unwrap();
            if locked {
                locks.insert(uid);
            } else {
                locks.remove(&uid);
            }
            save_locks(&locks);
        }
        let source = self.lock_source(&uid);
        self.publish(
            "device.locked",
            json!({"uid": uid, "locked": source.is_some(), "lock_source": source}),
        );
        Ok(json!({"uid": uid, "locked": source.is_some(), "lock_source": source}))
    }

    pub fn has_updates(&self) -> bool {
        self.entries
            .lock()
            .unwrap()
            .values()
            .any(|e| e.updating.load(Ordering::SeqCst))
    }

    pub fn devices(&self) -> Vec<Snapshot> {
        let entries = self.entries.lock().unwrap();
        entries
            .values()
            .map(|e| {
                let mut s = e.snapshot.lock().unwrap().clone();
                let source = self.lock_source(&s.uid);
                s.locked = source.is_some();
                s.lock_source = source.map(str::to_string);
                s.faults = self
                    .recorder
                    .relevant(&s.uid.to_string(), Duration::from_secs(600));
                s
            })
            .collect()
    }

    /// UID, nickname, or `None` for "the only connected device".
    pub fn resolve(&self, id: Option<&str>) -> Result<Uid, OpError> {
        let devices = self.devices();
        let online: Vec<&Snapshot> = devices.iter().filter(|d| d.online).collect();
        match id {
            None => match online.as_slice() {
                [only] => Ok(only.uid),
                [] => Err(OpError::new(
                    ErrorKind::NotFound,
                    "No Voloo connected. Check the USB data cable.",
                )),
                many => Err(OpError::new(
                    ErrorKind::InvalidArgument,
                    format!(
                        "{} Voloo devices are connected; name one by UID or nickname",
                        many.len()
                    ),
                )),
            },
            Some(id) => {
                if let Ok(uid) = id.parse::<Uid>() {
                    return match devices.iter().find(|d| d.uid == uid) {
                        Some(d)
                            if d.online
                                || d.firmware.as_ref().is_some_and(|f| {
                                    !matches!(
                                        f.stage.as_str(),
                                        "completed" | "failed" | "rolled_back"
                                    )
                                }) =>
                        {
                            Ok(uid)
                        }
                        Some(d) => Err(OpError::new(
                            ErrorKind::LinkError,
                            format!(
                                "Voloo {uid} is offline{}; its output state is unknown",
                                d.error
                                    .as_ref()
                                    .map(|e| format!(" ({})", e.message))
                                    .unwrap_or_default()
                            ),
                        )),
                        None => Err(OpError::new(
                            ErrorKind::NotFound,
                            format!("Voloo {uid} is not connected"),
                        )),
                    };
                }
                let named: Vec<&Snapshot> = devices
                    .iter()
                    .filter(|d| {
                        d.online
                            || d.firmware.as_ref().is_some_and(|f| {
                                !matches!(f.stage.as_str(), "completed" | "failed" | "rolled_back")
                            })
                    })
                    .filter(|d| d.nickname.as_deref() == Some(id))
                    .collect();
                match named.as_slice() {
                    [one] => Ok(one.uid),
                    [] => Err(OpError::new(
                        ErrorKind::NotFound,
                        format!("No connected Voloo is named {id:?}"),
                    )),
                    many => Err(OpError::new(
                        ErrorKind::InvalidArgument,
                        format!("{} devices are named {id:?}; use the UID", many.len()),
                    )),
                }
            }
        }
    }

    /// Runs a job on the device's worker and waits for the result.
    pub async fn call(
        &self,
        id: Option<&str>,
        job: Job,
        source: &'static str,
    ) -> Result<Value, OpError> {
        if job.writes() && id.is_none() {
            return Err(OpError::new(
                ErrorKind::InvalidArgument,
                "Name the device (UID or nickname) for any action that changes it",
            ));
        }
        let uid = self.resolve(id)?;
        let maintenance = job.maintenance();
        let (reply, rx) = oneshot::channel();
        {
            let entries = self.entries.lock().unwrap();
            let tx = entries
                .get(&uid)
                .and_then(|e| e.tx.clone())
                .ok_or_else(|| {
                    OpError::new(ErrorKind::LinkError, format!("Voloo {uid} is offline"))
                })?;
            let updating = &entries.get(&uid).unwrap().updating;
            if updating.load(Ordering::SeqCst)
                || (maintenance
                    && updating
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_err())
            {
                return Err(OpError::new(
                    ErrorKind::Busy,
                    "Firmware maintenance is in progress for this device; no other device operation was queued",
                ));
            }
            tx.send(Request { job, source, reply }).map_err(|_| {
                if maintenance {
                    updating.store(false, Ordering::SeqCst);
                }
                OpError::new(ErrorKind::LinkError, format!("Voloo {uid} went offline"))
            })?;
        }
        match tokio::time::timeout(Duration::from_secs(90), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(OpError::new(
                ErrorKind::Unknown,
                "The device worker stopped before replying; read status before any further action",
            )),
            Err(_) => Err(OpError::new(
                ErrorKind::Unknown,
                "No result within 90 s; read status before any further action",
            )),
        }
    }
}

/// Consecutive failed polls before a device is reported offline.
const NO_RESPONSE_POLLS: u32 = 3;

struct Worker {
    misses: std::sync::atomic::AtomicU32,
    uid: Uid,
    path: String,
    manager: Arc<Manager>,
    snapshot: Arc<Mutex<Snapshot>>,
    alive: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    updating: Arc<AtomicBool>,
}

impl Worker {
    fn run(self, rx: mpsc::Receiver<Request>) {
        let m = &self.manager;
        let mut device = match discover::open(&self.path, Some(self.uid), &m.recorder) {
            Ok(d) => d,
            Err(e) => {
                let changed = {
                    let mut s = self.snapshot.lock().unwrap();
                    let changed =
                        s.online || s.error.as_ref().map(|x| &x.message) != Some(&e.message);
                    s.online = false;
                    s.error = Some(e);
                    changed
                };
                if changed {
                    m.publish(
                        "device.offline",
                        json!(self.snapshot.lock().unwrap().clone()),
                    );
                }
                self.alive.store(false, Ordering::SeqCst);
                return;
            }
        };
        let trace = new_trace_id();
        let nickname = device.nickname(&trace).ok();
        let identity = device.identity(&trace).ok();
        {
            let mut s = self.snapshot.lock().unwrap();
            s.online = true;
            s.error = None;
            s.nickname = nickname;
            s.identity = identity;
        }
        let mut latencies: VecDeque<u64> = VecDeque::with_capacity(1000);
        let mut last_comm = home::load_comm(self.uid);
        let mut last_comm_check = Instant::now() - Duration::from_secs(3600);
        self.poll(&mut device, &mut latencies);
        m.publish("device.added", json!(self.snapshot.lock().unwrap().clone()));

        while !self.stop.load(Ordering::SeqCst) && device.transport.broken.is_none() {
            let interval = if m.watchers.load(Ordering::SeqCst) > 0 {
                m.config.poll
            } else {
                m.config.idle_poll
            };
            match rx.recv_timeout(interval) {
                Ok(req) => self.handle(&mut device, req, &mut latencies),
                Err(mpsc::RecvTimeoutError::Timeout) => self.poll(&mut device, &mut latencies),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if last_comm_check.elapsed() >= Duration::from_secs(30)
                && device.transport.broken.is_none()
            {
                last_comm_check = Instant::now();
                device.transport.silent = true;
                if let Ok(r) = device.diagnostics("communication")
                    && let Some(Diagnostics::Communication(c)) = &r.data
                {
                    device.check_restart(&r.trace_id, last_comm.as_ref(), c);
                    home::save_comm(self.uid, c);
                    self.snapshot.lock().unwrap().metrics.device_uptime_s = Some(c.uptime_s);
                    last_comm = Some(c.clone());
                }
                device.transport.silent = false;
            }
        }
        {
            let mut s = self.snapshot.lock().unwrap();
            s.online = false;
            s.error = Some(match &device.transport.broken {
                Some(e) => OpError::new(ErrorKind::LinkError, e.to_string()),
                None if self.stop.load(Ordering::SeqCst) && !m.stop.load(Ordering::SeqCst) => {
                    OpError::new(
                        ErrorKind::LinkError,
                        "USB port disappeared; output state unknown",
                    )
                }
                None => OpError::new(ErrorKind::LinkError, "background process stopped"),
            });
        }
        m.publish(
            "device.removed",
            json!(self.snapshot.lock().unwrap().clone()),
        );
        self.alive.store(false, Ordering::SeqCst);
    }

    fn poll(&self, device: &mut Device, latencies: &mut VecDeque<u64>) {
        let started = Instant::now();
        let result = device.poll();
        let elapsed = started.elapsed().as_micros() as u64;
        let m = &self.manager;
        let mut s = self.snapshot.lock().unwrap();
        s.metrics.polls += 1;
        s.metrics.link = device.transport.stats.clone();
        match result {
            Ok(report) => {
                if latencies.len() == 1000 {
                    latencies.pop_front();
                }
                latencies.push_back(elapsed);
                let mut sorted: Vec<u64> = latencies.iter().copied().collect();
                sorted.sort_unstable();
                s.metrics.status_samples = sorted.len();
                s.metrics.status_p50_us = sorted.get(sorted.len() / 2).copied();
                s.metrics.status_p99_us = sorted.get(sorted.len() * 99 / 100).copied();
                let changed = s.status.as_ref() != Some(&report.status);
                s.status = Some(report.status.clone());
                s.observed_at = Some(report.observed_at.clone());
                s.error = None;
                let recovered =
                    self.misses.swap(0, Ordering::SeqCst) >= NO_RESPONSE_POLLS || !s.online;
                s.online = true;
                drop(s);
                if recovered {
                    m.recorder
                        .clear(FaultCode::LinkNoResponse, &self.uid.to_string());
                    m.publish(
                        "device.online",
                        json!(self.snapshot.lock().unwrap().clone()),
                    );
                }
                if changed || m.watchers.load(Ordering::SeqCst) > 0 {
                    m.publish("status", json!({"uid": self.uid, "observed_at": report.observed_at, "status": report.status, "faults": report.faults}));
                }
            }
            Err(e) => {
                s.metrics.poll_errors += 1;
                let misses = self.misses.fetch_add(1, Ordering::SeqCst) + 1;
                // A device that keeps not answering is treated as offline: its state is unknown.
                let lost = misses == NO_RESPONSE_POLLS && s.online;
                if lost {
                    s.online = false;
                }
                s.error = Some(if misses >= NO_RESPONSE_POLLS {
                    OpError::new(
                        ErrorKind::LinkError,
                        format!("not responding ({misses} polls without a valid reply)"),
                    )
                } else {
                    e
                });
                drop(s);
                if lost {
                    m.recorder.raise(
                        FaultCode::LinkNoResponse,
                        Some(self.uid.to_string()),
                        Some(self.path.clone()),
                        format!("no valid reply to {NO_RESPONSE_POLLS} consecutive polls"),
                        None,
                        Value::Null,
                    );
                    m.publish(
                        "device.offline",
                        json!(self.snapshot.lock().unwrap().clone()),
                    );
                }
            }
        }
    }

    fn handle(&self, device: &mut Device, req: Request, latencies: &mut VecDeque<u64>) {
        let m = &self.manager;
        let maintenance = req.job.maintenance();
        struct UpdateGuard(Option<Arc<AtomicBool>>);
        impl Drop for UpdateGuard {
            fn drop(&mut self) {
                if let Some(flag) = self.0.take() {
                    flag.store(false, Ordering::SeqCst);
                }
            }
        }
        let guard = UpdateGuard(maintenance.then(|| self.updating.clone()));
        let action = req.job.action();
        let writes = req.job.writes();
        if writes && let Some(source) = m.lock_source(&self.uid) {
            let how = if source == "config" {
                "by configuration (config.toml [devices] locked = true)"
            } else {
                "from the web page or API (unlock it there)"
            };
            let _ = req.reply.send(Err(OpError::new(
                ErrorKind::Busy,
                format!("Voloo {} is locked {how}; no command was sent", self.uid),
            )));
            return;
        }
        device.source = req.source;
        if writes {
            m.publish(
                "operation.started",
                json!({"uid": self.uid, "action": action, "source": req.source}),
            );
        }
        let result = match req.job {
            Job::Status => device.status().map(|r| json!(r)),
            Job::Info => device.info().map(|r| json!(r)),
            Job::Diagnostics(kind) => device.diagnostics(&kind).map(|r| json!(r)),
            Job::Power(c) => device.set_power(c).map(|r| json!(r)),
            Job::Cycle { off_ms, discharge } => device.cycle(off_ms, discharge).map(|r| json!(r)),
            Job::Nickname(name) => device.set_nickname(&name).map(|r| json!(r)),
            Job::FirmwareStatus => firmware::status(device),
            job @ (Job::Firmware(_) | Job::FirmwareOnline | Job::FirmwareRollback) => {
                let online = matches!(job, Job::FirmwareOnline);
                let image = match job {
                    Job::Firmware(image) => Some(image),
                    _ => None,
                };
                let mut report = |progress: firmware::Progress| {
                    {
                        let mut s = self.snapshot.lock().unwrap();
                        if !progress.connected {
                            s.online = false;
                            s.status = None;
                        }
                        s.firmware = Some(progress.clone());
                    }
                    m.publish(
                        "firmware.progress",
                        json!({"uid":self.uid,"firmware":progress}),
                    );
                };
                let result = if online {
                    firmware::perform_online(device, m.config.scan_usb, &mut report)
                } else {
                    firmware::perform(device, image, m.config.scan_usb, &mut report)
                };
                if let Err(error) = &result {
                    let mut s = self.snapshot.lock().unwrap();
                    if !s
                        .firmware
                        .as_ref()
                        .is_some_and(|p| matches!(p.stage.as_str(), "failed" | "rolled_back"))
                    {
                        let progress = firmware::Progress {
                            stage: "failed".into(),
                            percent: 100,
                            connected: device.transport.broken.is_none(),
                            written: 0,
                            total: 0,
                        };
                        s.firmware = Some(progress.clone());
                        drop(s);
                        m.publish(
                            "firmware.progress",
                            json!({"uid":self.uid,"firmware":progress,"error":error}),
                        );
                    }
                }
                if device.transport.broken.is_none() {
                    if let Ok(info) = device.info() {
                        let mut s = self.snapshot.lock().unwrap();
                        s.identity = Some(info.identity);
                        s.nickname = info.nickname;
                        s.port = device.port();
                    }
                    self.poll(device, latencies);
                }
                result
            }
        };
        device.source = "daemon";
        let result = result.map_err(|mut e| {
            e.faults = device.relevant_faults();
            e
        });
        {
            let mut s = self.snapshot.lock().unwrap();
            // Client requests count too, not only the background polls.
            s.metrics.link = device.transport.stats.clone();
            if let Ok(v) = &result
                && let Some(name) = v.get("nickname").and_then(Value::as_str)
                && action != "status"
            {
                s.nickname = Some(name.to_string());
            }
        }
        drop(guard);
        if writes {
            let summary = match &result {
                Ok(v) => {
                    json!({"uid": self.uid, "action": action, "source": req.source, "ok": true, "outcome": v.get("outcome").cloned().unwrap_or(json!("completed")), "trace_id": v.get("trace_id")})
                }
                Err(e) => {
                    json!({"uid": self.uid, "action": action, "source": req.source, "ok": false, "error": e})
                }
            };
            self.snapshot.lock().unwrap().last_operation =
                Some(json!({"at": now(), "result": summary.clone()}));
            m.publish("operation.finished", summary);
        }
        let _ = req.reply.send(result);
        if writes && !maintenance && device.transport.broken.is_none() {
            self.poll(device, latencies);
        }
    }
}

fn locks_path() -> Option<std::path::PathBuf> {
    home::dir().map(|d| d.join("locks.json"))
}
fn load_locks() -> std::collections::BTreeSet<Uid> {
    locks_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .map(|v| v.iter().filter_map(|u| u.parse().ok()).collect())
        .unwrap_or_default()
}
fn save_locks(locks: &std::collections::BTreeSet<Uid>) {
    let Some(path) = locks_path() else { return };
    let list: Vec<String> = locks.iter().map(Uid::to_string).collect();
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(&list).unwrap()).is_ok() {
        let _ = std::fs::rename(tmp, path);
    }
}
