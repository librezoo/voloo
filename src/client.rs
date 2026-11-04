//! Client side of the background process: finds it through `daemon.json`, starts it on
//! demand, and speaks just enough HTTP/1.1 to 127.0.0.1 (no TLS, no proxies).
use crate::{
    daemon::{DaemonInfo, read_info},
    device::{ErrorKind, OpError},
    home,
};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub enum SendError {
    /// Nothing was sent (connection refused): safe to start the process and retry.
    NotConnected(String),
    /// The request may have reached the process; the outcome is unknown.
    Broken(String),
}

pub struct Client {
    pub info: DaemonInfo,
    pub source: &'static str,
}

impl Client {
    /// Connects to a running background process without starting one.
    pub fn existing(source: &'static str) -> Option<Client> {
        let client = Client {
            info: read_info()?,
            source,
        };
        client.healthy().then_some(client)
    }

    /// Connects, starting the background process if needed.
    pub fn connect(source: &'static str) -> Result<Client, OpError> {
        if let Some(c) = Client::existing(source) {
            return Ok(c);
        }
        spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(c) = Client::existing(source) {
                return Ok(c);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let log = home::logs()
            .map(|d| d.join("daemon.log").display().to_string())
            .unwrap_or_default();
        Err(OpError::new(
            ErrorKind::LinkError,
            format!("The voloo background process did not start; see {log}"),
        ))
    }

    fn healthy(&self) -> bool {
        matches!(
            self.send("GET", "/v1/health", None, Duration::from_secs(2)),
            Ok((200, _))
        )
    }

    /// Raw request; returns (status, body).
    pub fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<(u16, Value), SendError> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.info.port));
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
            .map_err(|e| SendError::NotConnected(e.to_string()))?;
        stream.set_read_timeout(Some(timeout)).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nX-Voloo-Source: {source}\r\nAccept: application/json\r\nConnection: close\r\n{ctype}Content-Length: {len}\r\n\r\n",
            port = self.info.port,
            token = self.info.token,
            source = self.source,
            ctype = if body.is_some() {
                "Content-Type: application/json\r\n"
            } else {
                ""
            },
            len = payload.len()
        );
        stream
            .write_all(head.as_bytes())
            .and_then(|_| stream.write_all(payload.as_bytes()))
            .map_err(|e| SendError::Broken(e.to_string()))?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| SendError::Broken(e.to_string()))?;
        parse_response(&raw).map_err(SendError::Broken)
    }

    /// JSON API call; HTTP errors become the server's `OpError`.
    pub fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, OpError> {
        let writes = method != "GET";
        match self.send(method, path, body, Duration::from_secs(120)) {
            Ok((status, value)) if status < 300 => Ok(value),
            Ok((_, value)) => Err(
                serde_json::from_value(value["error"].clone()).unwrap_or_else(|_| {
                    OpError::new(
                        ErrorKind::LinkError,
                        format!("Unexpected reply from the background process: {value}"),
                    )
                }),
            ),
            Err(SendError::NotConnected(e)) => Err(OpError::new(
                ErrorKind::LinkError,
                format!("Background process unreachable: {e}"),
            )),
            Err(SendError::Broken(e)) if writes => Err(OpError::new(
                ErrorKind::Unknown,
                format!(
                    "Lost the connection to the background process during the request ({e}); read status before any further action"
                ),
            )),
            Err(SendError::Broken(e)) => Err(OpError::new(
                ErrorKind::LinkError,
                format!("Lost the connection to the background process: {e}"),
            )),
        }
    }
}

fn parse_response(raw: &[u8]) -> Result<(u16, Value), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("incomplete HTTP response")?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| "bad HTTP header")?;
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or("bad status line")?;
    let chunked = head.lines().any(|l| {
        l.to_ascii_lowercase().starts_with("transfer-encoding:")
            && l.to_ascii_lowercase().contains("chunked")
    });
    let mut body = raw[split + 4..].to_vec();
    if chunked {
        let mut out = Vec::new();
        let mut rest = &body[..];
        loop {
            let line_end = rest
                .windows(2)
                .position(|w| w == b"\r\n")
                .ok_or("bad chunk")?;
            let size = usize::from_str_radix(
                std::str::from_utf8(&rest[..line_end])
                    .map_err(|_| "bad chunk")?
                    .split(';')
                    .next()
                    .unwrap()
                    .trim(),
                16,
            )
            .map_err(|_| "bad chunk size")?;
            rest = &rest[line_end + 2..];
            if size == 0 {
                break;
            }
            out.extend_from_slice(rest.get(..size).ok_or("short chunk")?);
            rest = rest.get(size + 2..).ok_or("short chunk")?;
        }
        body = out;
    }
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into()))
    };
    Ok((status, value))
}

/// Starts `voloo daemon run` detached from this terminal, logging to `logs/daemon.log`.
fn spawn() -> Result<(), OpError> {
    let exe =
        std::env::current_exe().map_err(|e| OpError::new(ErrorKind::LinkError, e.to_string()))?;
    let logs = home::logs()
        .ok_or_else(|| OpError::new(ErrorKind::NotFound, "No home directory; set VOLOO_HOME"))?;
    std::fs::create_dir_all(&logs)
        .map_err(|e| OpError::new(ErrorKind::LinkError, e.to_string()))?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join("daemon.log"))
        .map_err(|e| OpError::new(ErrorKind::LinkError, e.to_string()))?;
    let mut cmd = Command::new(exe);
    cmd.args(["daemon", "run"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    cmd.spawn().map_err(|e| {
        OpError::new(
            ErrorKind::LinkError,
            format!("cannot start the background process: {e}"),
        )
    })?;
    Ok(())
}
