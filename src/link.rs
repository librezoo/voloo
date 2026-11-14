//! Serial link and request/reply transport. One outstanding request per port:
//! replies carry no transaction id, so stale bytes are drained before each send
//! and the line is allowed to go quiet after a timeout.
use crate::{
    observe::{Level, Recorder, Span, now},
    protocol::{FrameDecoder, Matched, Request, hex, match_reply},
};
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};

pub trait Link: Send {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Reads available bytes, waiting at most `timeout`. Returns 0 on timeout.
    /// A zero timeout only returns what is already buffered and never waits.
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize>;
    fn path(&self) -> &str;
}

pub struct SerialLink {
    port: Box<dyn serialport::SerialPort>,
    path: String,
}
impl SerialLink {
    /// Opens exclusively. The X035 CDC firmware ignores line coding; a standard rate is used.
    pub fn open(path: &str) -> Result<Box<dyn Link>, LinkError> {
        match serialport::new(path, 115_200)
            .timeout(Duration::from_millis(10))
            .open()
        {
            Ok(port) => Ok(Box::new(SerialLink {
                port,
                path: path.to_string(),
            })),
            Err(e) => {
                let error = LinkError::from_open(path, e);
                // On macOS serialport sets every rate with IOSSIOSPEED, which pseudo-terminals
                // and some virtual ports reject; fall back to plain raw termios there.
                #[cfg(unix)]
                if matches!(error, LinkError::Io(_)) {
                    match RawTty::open(path) {
                        Ok(raw) => return Ok(Box::new(raw)),
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            return Err(LinkError::Busy(path.into()));
                        }
                        Err(_) => {}
                    }
                }
                Err(error)
            }
        }
    }
    #[cfg(unix)]
    pub fn from_tty(port: serialport::TTYPort, path: &str) -> Self {
        SerialLink {
            port: Box::new(port),
            path: path.to_string(),
        }
    }
}
impl Link for SerialLink {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.port.write_all(bytes)?;
        self.port.flush()
    }
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        if timeout.is_zero() && self.port.bytes_to_read()? == 0 {
            return Ok(0);
        }
        self.port
            .set_timeout(timeout.max(Duration::from_millis(1)))?;
        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(e)
                if e.kind() == io::ErrorKind::TimedOut || e.kind() == io::ErrorKind::WouldBlock =>
            {
                Ok(0)
            }
            Err(e) => Err(e),
        }
    }
    fn path(&self) -> &str {
        &self.path
    }
}

/// Minimal raw-mode TTY (exclusive, 8N1, no echo) for character devices that serialport
/// cannot configure, such as pseudo-terminals.
#[cfg(unix)]
pub struct RawTty {
    fd: std::os::fd::OwnedFd,
    path: String,
}
#[cfg(unix)]
impl RawTty {
    pub fn open(path: &str) -> io::Result<Self> {
        use std::os::fd::{FromRawFd, OwnedFd};
        let c = std::ffi::CString::new(path)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let fd =
            unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let raw = std::os::fd::AsRawFd::as_raw_fd(&fd);
        unsafe {
            // Same exclusion as serialport: TIOCEXCL (which pseudo-terminals on macOS ignore)
            // plus an exclusive advisory lock, so two voloo processes never share a port.
            if libc::flock(raw, libc::LOCK_EX | libc::LOCK_NB) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(raw, libc::TIOCEXCL as _) != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(raw, &mut t) != 0 {
                return Err(io::Error::last_os_error());
            }
            libc::cfmakeraw(&mut t);
            if libc::tcsetattr(raw, libc::TCSANOW, &t) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(RawTty {
            fd,
            path: path.to_string(),
        })
    }
}
#[cfg(unix)]
impl Link for RawTty {
    fn write_all(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        let deadline = Instant::now() + Duration::from_secs(1);
        while !bytes.is_empty() {
            let n = unsafe { libc::write(self.fd.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                return Err(e);
            }
            bytes = &bytes[n as usize..];
        }
        Ok(())
    }
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        use std::os::fd::AsRawFd;
        let mut pfd = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
        if ready < 0 {
            return Err(io::Error::last_os_error());
        }
        if ready == 0 {
            return Ok(0);
        }
        if pfd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
            && pfd.revents & libc::POLLIN == 0
        {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::WouldBlock {
                Ok(0)
            } else {
                Err(e)
            };
        }
        Ok(n as usize)
    }
    fn path(&self) -> &str {
        &self.path
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum LinkError {
    #[error("no reply within {0} ms")]
    Timeout(u64),
    #[error("serial port {0} is in use by another program")]
    Busy(String),
    #[error("serial port {0} not found or disconnected")]
    Gone(String),
    #[error("serial I/O error: {0}")]
    Io(String),
}
impl LinkError {
    fn from_open(path: &str, e: serialport::Error) -> Self {
        let text = e.to_string();
        match e.kind() {
            // serialport reports EBUSY (TIOCEXCL held by another program) as NoDevice;
            // a port whose path still exists is busy, not unplugged.
            serialport::ErrorKind::NoDevice if std::path::Path::new(path).exists() => {
                LinkError::Busy(path.into())
            }
            serialport::ErrorKind::NoDevice => LinkError::Gone(path.into()),
            serialport::ErrorKind::Io(io::ErrorKind::PermissionDenied) => LinkError::Io(format!(
                "{path}: permission denied (on Linux, join the dialout group)"
            )),
            serialport::ErrorKind::Io(io::ErrorKind::NotFound) => LinkError::Gone(path.into()),
            _ if text.contains("busy")
                || text.contains("Busy")
                || text.contains("exclusive")
                || text.contains("Access is denied") =>
            {
                LinkError::Busy(path.into())
            }
            _ => LinkError::Io(format!("{path}: {text}")),
        }
    }
    fn from_io(path: &str, e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::BrokenPipe | io::ErrorKind::NotConnected => {
                LinkError::Gone(path.into())
            }
            _ if e.raw_os_error() == Some(6) /* ENXIO */ || e.raw_os_error() == Some(5) /* EIO */ => {
                LinkError::Gone(path.into())
            }
            _ => LinkError::Io(format!("{path}: {e}")),
        }
    }
}

pub struct Timing {
    /// USB adds scheduling latency on top of the protocol's 20 ms reply budget.
    pub response: Duration,
    /// After a timeout, wait until the line has been quiet this long...
    pub quiet: Duration,
    /// ...but no longer than this in total.
    pub recovery_max: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Timing {
            response: Duration::from_millis(150),
            quiet: Duration::from_millis(30),
            recovery_max: Duration::from_millis(300),
        }
    }
}

pub struct Transport {
    link: Box<dyn Link>,
    recorder: Arc<Recorder>,
    pub timing: Timing,
    /// Set after an I/O failure; the transport is unusable afterwards.
    pub broken: Option<LinkError>,
    pub stats: LinkStats,
    /// Background polling: no span unless the log level is `trace`.
    pub silent: bool,
}

#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct LinkStats {
    pub requests: u64,
    pub timeouts: u64,
    pub crc_errors: u64,
    pub late_or_foreign_frames: u64,
    pub stale_bytes: u64,
}

impl Transport {
    pub fn new(link: Box<dyn Link>, recorder: Arc<Recorder>) -> Self {
        Transport {
            link,
            recorder,
            timing: Timing::default(),
            broken: None,
            stats: LinkStats::default(),
            silent: false,
        }
    }
    pub fn path(&self) -> &str {
        self.link.path()
    }

    /// Firmware updates deliberately leave CDC. Drop the exclusive OS handle
    /// before ROM ISP or reconnection; no second process may race the worker.
    pub fn release_for_update(&mut self) {
        struct Released(String);
        impl Link for Released {
            fn write_all(&mut self, _: &[u8]) -> io::Result<()> {
                Err(io::ErrorKind::NotConnected.into())
            }
            fn read(&mut self, _: &mut [u8], _: Duration) -> io::Result<usize> {
                Err(io::ErrorKind::NotConnected.into())
            }
            fn path(&self) -> &str {
                &self.0
            }
        }
        let path = self.path().to_string();
        self.link = Box::new(Released(path.clone()));
        self.broken = Some(LinkError::Gone(path));
    }

    /// Discards whatever is already buffered (late replies from earlier requests) without
    /// waiting: an empty buffer must not add latency to every request.
    fn drain(&mut self, budget: Duration) -> Result<usize, LinkError> {
        let mut buf = [0u8; 256];
        let mut total = 0;
        let start = Instant::now();
        loop {
            let n = self
                .link
                .read(&mut buf, Duration::ZERO)
                .map_err(|e| LinkError::from_io(self.link.path(), e))?;
            total += n;
            if n == 0 || start.elapsed() >= budget {
                return Ok(total);
            }
        }
    }
    fn recover(&mut self) -> Result<usize, LinkError> {
        let mut buf = [0u8; 256];
        let mut total = 0;
        let start = Instant::now();
        let mut last = Instant::now();
        while start.elapsed() < self.timing.recovery_max && last.elapsed() < self.timing.quiet {
            let n = self
                .link
                .read(&mut buf, Duration::from_millis(5))
                .map_err(|e| LinkError::from_io(self.link.path(), e))?;
            if n > 0 {
                total += n;
                last = Instant::now();
            }
        }
        Ok(total)
    }

    pub fn request(
        &mut self,
        trace_id: &str,
        req: &Request,
        timeout: Option<Duration>,
    ) -> Result<Matched, LinkError> {
        if let Some(e) = &self.broken {
            return Err(e.clone());
        }
        let timeout = timeout.unwrap_or(self.timing.response);
        let started = Instant::now();
        let at = now();
        let mut decoder = FrameDecoder::default();
        let mut rx_frame: Option<Vec<u8>> = None;
        let mut foreign = 0usize;
        let mut stale = 0usize;
        let result = (|| {
            stale = self.drain(Duration::from_millis(20))?;
            self.link
                .write_all(&req.frame)
                .map_err(|e| LinkError::from_io(self.link.path(), e))?;
            let deadline = Instant::now() + timeout;
            let mut buf = [0u8; 256];
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(LinkError::Timeout(timeout.as_millis() as u64));
                }
                let n = self
                    .link
                    .read(&mut buf, left)
                    .map_err(|e| LinkError::from_io(self.link.path(), e))?;
                for frame in decoder.push(&buf[..n]) {
                    match match_reply(req.uid, &req.expect, &frame) {
                        Some(m) => {
                            rx_frame = Some(frame);
                            return Ok(m);
                        }
                        None => foreign += 1,
                    }
                }
            }
        })();
        let mut result = result;
        if matches!(result, Err(LinkError::Timeout(_))) {
            match self.recover() {
                Ok(n) => stale += n,
                Err(e) => result = Err(e),
            }
        }
        if let Err(e @ (LinkError::Gone(_) | LinkError::Io(_))) = &result {
            self.broken = Some(e.clone());
        }
        self.stats.requests += 1;
        self.stats.crc_errors += decoder.crc_errors as u64;
        self.stats.late_or_foreign_frames += foreign as u64;
        self.stats.stale_bytes += stale as u64;
        if matches!(result, Err(LinkError::Timeout(_))) {
            self.stats.timeouts += 1;
        }
        if self.silent && self.recorder.level < Level::Trace {
            return result;
        }
        let debug = self.recorder.level >= Level::Debug;
        self.recorder.span(Span {
            trace_id: trace_id.to_string(),
            at,
            port: self.link.path().to_string(),
            uid: req.uid.to_string(),
            label: req.label.clone(),
            fc: req.expect.fc(),
            tx_len: req.frame.len(),
            rx_len: rx_frame.as_ref().map_or(0, Vec::len),
            elapsed_us: started.elapsed().as_micros() as u64,
            result: match &result {
                Ok(Matched::Exception(_)) => "exception",
                Ok(_) => "ok",
                Err(LinkError::Timeout(_)) => "timeout",
                Err(_) => "io_error",
            },
            detail: match &result {
                Ok(Matched::Exception(e)) => Some(e.to_string()),
                Err(e) => Some(e.to_string()),
                _ => None,
            },
            stale_bytes: stale,
            foreign_frames: foreign,
            crc_errors: decoder.crc_errors,
            tx_hex: debug.then(|| hex(&req.frame)),
            rx_hex: if debug {
                rx_frame.as_deref().map(hex)
            } else {
                None
            },
        });
        result
    }
}
