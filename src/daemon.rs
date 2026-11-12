//! Background process: owns every Voloo serial port and serves the local HTTP API,
//! SSE events and MCP on 127.0.0.1. Started on demand by clients; exits after
//! `idle_exit` without requests or open event streams. Exiting never changes outputs.
use crate::{
    config::Config,
    device::{ErrorKind, OpError},
    firmware, home,
    manager::{Job, Manager},
    mcp,
    observe::{Level, Limits, Recorder, now, read_log},
    protocol::{Command, Uid},
};
use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post, put},
};
use futures_util::{Stream, StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio_stream::wrappers::BroadcastStream;

/// Written to `$VOLOO_HOME/daemon.json` (mode 0600) so clients can find and authenticate.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DaemonInfo {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub version: String,
    pub started_at: String,
}

pub fn info_path() -> Option<PathBuf> {
    home::dir().map(|d| d.join("daemon.json"))
}
pub fn read_info() -> Option<DaemonInfo> {
    serde_json::from_slice(&fs::read(info_path()?).ok()?).ok()
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&tmp)?, bytes)?;
    fs::rename(tmp, path)
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS random source");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone)]
struct App {
    manager: Arc<Manager>,
    token: Arc<String>,
    /// One-time web login codes (60 s) and the browser sessions they turned into.
    codes: Arc<std::sync::Mutex<std::collections::HashMap<String, Instant>>>,
    sessions: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    port: u16,
    last_activity: Arc<AtomicU64>,
    started: Instant,
    /// Set once to stop: ends event streams, then the HTTP server.
    stopping: tokio::sync::watch::Sender<bool>,
}

/// Runs the background process in the foreground of the calling thread until it exits.
pub fn run() -> Result<(), String> {
    let home = home::dir().ok_or("No home directory; set VOLOO_HOME")?;
    fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    let lock_path = home.join("daemon.lock");
    let lock = fs::File::create(&lock_path).map_err(|e| format!("{}: {e}", lock_path.display()))?;
    // A previous process may still be closing its ports after `daemon stop`; give it time
    // instead of failing the client that just started us.
    let deadline = Instant::now() + Duration::from_secs(5);
    while lock.try_lock().is_err() {
        if Instant::now() >= deadline {
            return Err("Another voloo background process is already running".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let config = Config::load()?;
    let level: Level = std::env::var("VOLOO_LOG_LEVEL")
        .ok()
        .and_then(|l| l.parse().ok())
        .unwrap_or(Level::Info);
    let recorder = Arc::new(
        Recorder::with_dir(level, home.join("logs"), Limits::default())
            .map_err(|e| format!("logs: {e}"))?,
    );
    let manager = Manager::new(recorder, config.clone());
    manager.start();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let result = runtime.block_on(serve(manager.clone(), config));
    manager.shutdown();
    if let (Some(path), Some(info)) = (info_path(), read_info())
        && info.pid == std::process::id()
    {
        let _ = fs::remove_file(path);
    }
    drop(lock);
    result
}

/// Waits until no background process holds the lock (it has fully exited and released its
/// ports). Returns false on timeout.
pub fn wait_exited(timeout: Duration) -> bool {
    let Some(path) = home::dir().map(|d| d.join("daemon.lock")) else {
        return true;
    };
    let deadline = Instant::now() + timeout;
    loop {
        match fs::File::open(&path) {
            Err(_) => return true,
            Ok(f) if f.try_lock().is_ok() => return true,
            Ok(_) if Instant::now() >= deadline => return false,
            Ok(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

async fn serve(manager: Arc<Manager>, config: Config) -> Result<(), String> {
    let port: u16 = std::env::var("VOLOO_DAEMON_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("bind 127.0.0.1:{port}: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let app = App {
        manager,
        token: Arc::new(random_token()),
        codes: Default::default(),
        sessions: Default::default(),
        port,
        last_activity: Arc::new(AtomicU64::new(unix_now())),
        started: Instant::now(),
        stopping: tokio::sync::watch::channel(false).0,
    };
    let info = DaemonInfo {
        pid: std::process::id(),
        port,
        token: app.token.to_string(),
        version: env!("CARGO_PKG_VERSION").into(),
        started_at: now(),
    };
    let path = info_path().ok_or("No home directory")?;
    write_private(&path, &serde_json::to_vec_pretty(&info).unwrap())
        .map_err(|e| format!("{}: {e}", path.display()))?;

    // Idle exit: no request and no open event stream for `idle_exit`.
    {
        let app = app.clone();
        let idle = config.idle_exit.as_secs();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let quiet = unix_now().saturating_sub(app.last_activity.load(Ordering::SeqCst));
                if app.manager.watchers.load(Ordering::SeqCst) == 0
                    && quiet >= idle
                    && !app.manager.has_updates()
                {
                    let _ = app.stopping.send(true);
                    break;
                }
            }
        });
    }
    // SIGINT/SIGTERM stop the process the same way as `voloo daemon stop`.
    {
        let stopping = app.stopping.clone();
        tokio::spawn(async move {
            #[cfg(unix)]
            {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
            let _ = stopping.send(true);
        });
    }
    let mut stopped = app.stopping.subscribe();
    let mut deadline = app.stopping.subscribe();
    let server = axum::serve(listener, router(app)).with_graceful_shutdown(async move {
        let _ = stopped.wait_for(|s| *s).await;
    });
    // Event streams end on the stop signal; anything else still open gets 3 s.
    tokio::select! {
        result = server => result.map_err(|e| e.to_string()),
        _ = async {
            let _ = deadline.wait_for(|s| *s).await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        } => Ok(()),
    }
}

fn router(app: App) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/devices", get(devices))
        .route("/v1/devices/{id}", get(device_status))
        .route("/v1/devices/{id}/info", get(device_info))
        .route("/v1/devices/{id}/diagnostics", get(device_diagnostics))
        .route("/v1/devices/{id}/power", post(device_power))
        .route("/v1/devices/{id}/cycle", post(device_cycle))
        .route("/v1/devices/{id}/nickname", put(device_nickname))
        .route(
            "/v1/devices/{id}/firmware",
            get(device_firmware).post(device_firmware_update),
        )
        .route(
            "/v1/devices/{id}/firmware/rollback",
            post(device_firmware_rollback),
        )
        .route("/v1/operations", get(operations))
        .route("/v1/faults", get(faults))
        .route("/v1/metrics", get(metrics))
        .route("/v1/trace", get(trace))
        .route("/v1/events", get(events))
        .route("/v1/shutdown", post(shutdown))
        .route("/mcp", post(mcp_post).get(mcp_other).delete(mcp_other))
        .route("/openapi.json", get(openapi))
        .route("/v1/devices/{id}/lock", post(device_lock))
        .route("/v1/ui/code", post(ui_code))
        .route("/", get(|| async { axum::response::Redirect::to("/ui/") }))
        .route("/ui/login", get(ui_login))
        .route("/ui/", get(ui_index))
        .route("/ui/app.js", get(ui_js))
        .route("/ui/i18n.js", get(ui_i18n))
        .route("/ui/app.css", get(ui_css))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

struct ApiError(OpError);
impl From<OpError> for ApiError {
    fn from(e: OpError) -> Self {
        ApiError(e)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(json!({"error": self.0}))).into_response()
    }
}
type ApiResult = Result<Json<Value>, ApiError>;

fn deny(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({"error": {"code": "forbidden", "message": message}})),
    )
        .into_response()
}

/// Local-only access: exact Host, no foreign browser Origin, and the bearer token.
async fn guard(State(app): State<App>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let allowed_hosts = [
        format!("127.0.0.1:{}", app.port),
        format!("localhost:{}", app.port),
    ];
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !allowed_hosts.iter().any(|h| h == host) {
        return deny(StatusCode::FORBIDDEN, "Host not allowed");
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok())
        && !allowed_hosts
            .iter()
            .any(|h| origin == format!("http://{h}"))
    {
        return deny(StatusCode::FORBIDDEN, "Origin not allowed");
    }
    // The page and its login link carry no secrets; everything else needs the token or a
    // browser session cookie obtained through a one-time code from `voloo ui`.
    let path = req.uri().path();
    let public = path == "/" || path.starts_with("/ui/");
    let expected = format!("Bearer {}", app.token);
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let session = cookie(headers, &session_cookie(app.port))
        .is_some_and(|c| app.sessions.lock().unwrap().contains(&c));
    if !public && !session && !constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
        return deny(
            StatusCode::UNAUTHORIZED,
            "Missing or wrong token; read it from $VOLOO_HOME/daemon.json or run `voloo api-info`",
        );
    }
    app.last_activity.store(unix_now(), Ordering::SeqCst);
    next.run(req).await
}
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Who is calling, for the operation journal.
fn source(headers: &HeaderMap) -> &'static str {
    match headers.get("x-voloo-source").and_then(|h| h.to_str().ok()) {
        Some("cli") => "cli",
        Some("ui") => "ui",
        Some("mcp") => "mcp",
        _ => "http",
    }
}
/// `-` names "the only connected device" (reads only).
fn target(id: &str) -> Option<&str> {
    (id != "-").then_some(id)
}

async fn health(State(app): State<App>) -> Json<Value> {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id(),
        "uptime_s": app.started.elapsed().as_secs(), "devices": app.manager.devices().len(),
        "watchers": app.manager.watchers.load(Ordering::SeqCst),
    }))
}
async fn devices(State(app): State<App>) -> Json<Value> {
    Json(json!(app.manager.devices()))
}
async fn device_status(State(app): State<App>, Path(id): Path<String>, h: HeaderMap) -> ApiResult {
    Ok(Json(
        app.manager
            .call(target(&id), Job::Status, source(&h))
            .await?,
    ))
}
async fn device_info(State(app): State<App>, Path(id): Path<String>, h: HeaderMap) -> ApiResult {
    Ok(Json(
        app.manager.call(target(&id), Job::Info, source(&h)).await?,
    ))
}
#[derive(Deserialize)]
struct DiagQuery {
    kind: Option<String>,
}
async fn device_diagnostics(
    State(app): State<App>,
    Path(id): Path<String>,
    Query(q): Query<DiagQuery>,
    h: HeaderMap,
) -> ApiResult {
    let kind = q.kind.unwrap_or_else(|| "communication".into());
    Ok(Json(
        app.manager
            .call(target(&id), Job::Diagnostics(kind), source(&h))
            .await?,
    ))
}
#[derive(Deserialize)]
struct FirmwareBody {
    data: Vec<u8>,
}
async fn device_firmware(
    State(app): State<App>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> ApiResult {
    Ok(Json(
        app.manager
            .call(target(&id), Job::FirmwareStatus, source(&h))
            .await?,
    ))
}
async fn device_firmware_update(
    State(app): State<App>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(body): Json<FirmwareBody>,
) -> ApiResult {
    let image = firmware::Image::parse(&body.data)?;
    Ok(Json(
        app.manager
            .call(target(&id), Job::Firmware(image), source(&h))
            .await?,
    ))
}
async fn device_firmware_rollback(
    State(app): State<App>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> ApiResult {
    Ok(Json(
        app.manager
            .call(target(&id), Job::FirmwareRollback, source(&h))
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PowerBody {
    action: String,
}
async fn device_power(
    State(app): State<App>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(body): Json<PowerBody>,
) -> ApiResult {
    let command = match body.action.as_str() {
        "on" => Command::On,
        "off" => Command::Off,
        "discharge" => Command::Discharge,
        _ => {
            return Err(OpError::new(
                ErrorKind::InvalidArgument,
                "action must be on, off or discharge",
            )
            .into());
        }
    };
    Ok(Json(
        app.manager
            .call(target(&id), Job::Power(command), source(&h))
            .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CycleBody {
    #[serde(default = "default_off_ms")]
    off_ms: u64,
    #[serde(default = "default_true")]
    discharge: bool,
}
fn default_off_ms() -> u64 {
    1000
}
fn default_true() -> bool {
    true
}
async fn device_cycle(
    State(app): State<App>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(body): Json<CycleBody>,
) -> ApiResult {
    Ok(Json(
        app.manager
            .call(
                target(&id),
                Job::Cycle {
                    off_ms: body.off_ms,
                    discharge: body.discharge,
                },
                source(&h),
            )
            .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NicknameBody {
    nickname: String,
}
async fn device_nickname(
    State(app): State<App>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(body): Json<NicknameBody>,
) -> ApiResult {
    Ok(Json(
        app.manager
            .call(target(&id), Job::Nickname(body.nickname), source(&h))
            .await?,
    ))
}

#[derive(Deserialize)]
struct LogQuery {
    device: Option<String>,
    limit: Option<usize>,
    trace_id: Option<String>,
    active: Option<bool>,
}
fn uid_filter(app: &App, device: Option<&str>) -> Result<Option<String>, ApiError> {
    device
        .map(|d| {
            d.parse::<Uid>()
                .map(|u| u.to_string())
                .or_else(|_| app.manager.resolve(Some(d)).map(|u| u.to_string()))
        })
        .transpose()
        .map_err(ApiError)
}
async fn operations(State(app): State<App>, Query(q): Query<LogQuery>) -> ApiResult {
    let uid = uid_filter(&app, q.device.as_deref())?;
    let dir =
        home::logs().ok_or_else(|| OpError::new(ErrorKind::NotFound, "No Voloo home directory"))?;
    Ok(Json(json!(read_log(
        &dir,
        "operations",
        uid.as_deref(),
        q.limit.unwrap_or(50).min(1000)
    ))))
}
async fn faults(State(app): State<App>, Query(q): Query<LogQuery>) -> ApiResult {
    let uid = uid_filter(&app, q.device.as_deref())?;
    let mut list = app
        .manager
        .recorder
        .faults(uid.as_deref(), q.active.unwrap_or(false));
    list.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
    list.truncate(q.limit.unwrap_or(200));
    Ok(Json(json!(list)))
}
async fn metrics(State(app): State<App>) -> Json<Value> {
    let devices: Vec<Value> = app
        .manager
        .devices()
        .into_iter()
        .map(|d| json!({"uid": d.uid, "port": d.port, "online": d.online, "metrics": d.metrics}))
        .collect();
    Json(
        json!({"uptime_s": app.started.elapsed().as_secs(), "watchers": app.manager.watchers.load(Ordering::SeqCst), "devices": devices}),
    )
}
async fn trace(State(app): State<App>, Query(q): Query<LogQuery>) -> ApiResult {
    let uid = uid_filter(&app, q.device.as_deref())?;
    let spans: Vec<Value> = app
        .manager
        .recorder
        .spans(q.trace_id.as_deref(), q.limit.unwrap_or(200).min(10_000))
        .into_iter()
        .filter(|s| uid.as_deref().is_none_or(|u| s.uid == u))
        .map(|s| json!(s))
        .collect();
    Ok(Json(json!(spans)))
}
async fn shutdown(State(app): State<App>) -> Json<Value> {
    // Answer first: stopping immediately could close this connection before the reply.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = app.stopping.send(true);
    });
    Json(json!({"stopping": true, "note": "Device outputs are left unchanged."}))
}

/// Keeps the fast polling rate while at least one event stream is open.
struct Watched<S> {
    inner: S,
    manager: Arc<Manager>,
}
impl<S> Drop for Watched<S> {
    fn drop(&mut self) {
        self.manager.watchers.fetch_sub(1, Ordering::SeqCst);
    }
}
impl<S: Stream + Unpin> Stream for Watched<S> {
    type Item = S::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}
async fn events(State(app): State<App>) -> impl IntoResponse {
    app.manager.watchers.fetch_add(1, Ordering::SeqCst);
    let hello = SseEvent::default()
        .event("hello")
        .json_data(json!({"at": now(), "devices": app.manager.devices()}))
        .unwrap();
    let live = BroadcastStream::new(app.manager.events.subscribe()).filter_map(|e| async move {
        match e {
            Ok(e) => SseEvent::default().event(e.kind.clone()).json_data(&e).ok(),
            // A slow reader missed events; tell it to refetch the device list.
            Err(_) => Some(SseEvent::default().event("lagged").data("{}")),
        }
    });
    let mut stopping = app.stopping.subscribe();
    let stream = stream::once(async move { hello })
        .chain(live)
        .take_until(async move {
            let _ = stopping.wait_for(|s| *s).await;
        })
        .map(Ok::<_, std::convert::Infallible>);
    Sse::new(Watched {
        inner: Box::pin(stream),
        manager: app.manager.clone(),
    })
    .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn mcp_post(State(app): State<App>, Json(message): Json<Value>) -> Response {
    match mcp::handle(&app.manager, message).await {
        Some(reply) => Json(reply).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}
async fn mcp_other() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        "This server does not open server-initiated streams",
    )
        .into_response()
}

async fn openapi() -> Json<Value> {
    Json(openapi_spec())
}

pub fn openapi_spec() -> Value {
    let id = json!({"name": "id", "in": "path", "required": true, "schema": {"type": "string"}, "description": "UID, unique nickname, or '-' for the only connected device (reads only)"});
    let ok = |d: &str| json!({"200": {"description": d, "content": {"application/json": {"schema": {"type": "object"}}}}, "default": {"$ref": "#/components/responses/Error"}});
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Voloo local API", "version": env!("CARGO_PKG_VERSION"),
            "description": "Local-only API of the voloo background process. Connection details are in $VOLOO_HOME/daemon.json (port, token) or `voloo api-info --json`. Every request needs `Authorization: Bearer <token>`. Voloo switches external power; OFF does not mean 0 V at the load. Power results are confirmed by reading the device back; an 'unknown' outcome must be followed by a status read, never a blind retry."},
        "servers": [{"url": "http://127.0.0.1:{port}", "variables": {"port": {"default": "0"}}}],
        "security": [{"bearer": []}],
        "paths": {
            "/v1/health": {"get": {"summary": "Process health", "responses": ok("Health")}},
            "/v1/devices": {"get": {"summary": "Known devices with the latest polled state (no device I/O)", "responses": ok("Device snapshots")}},
            "/v1/devices/{id}": {"get": {"summary": "Read status now", "parameters": [id], "responses": ok("Status report")}},
            "/v1/devices/{id}/info": {"get": {"summary": "Identity, versions, nickname", "parameters": [id], "responses": ok("Info")}},
            "/v1/devices/{id}/diagnostics": {"get": {"summary": "Diagnostic registers", "parameters": [id, {"name": "kind", "in": "query", "schema": {"enum": ["communication", "adc", "fast"]}}], "responses": ok("Diagnostics")}},
            "/v1/devices/{id}/power": {"post": {"summary": "Switch on, off or discharge and confirm", "parameters": [id],
                "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["action"], "properties": {"action": {"enum": ["on", "off", "discharge"]}}}}}},
                "responses": ok("Power result (outcome completed | already_satisfied)")}},
            "/v1/devices/{id}/cycle": {"post": {"summary": "Discharge (or OFF), wait, ON", "parameters": [id],
                "requestBody": {"content": {"application/json": {"schema": {"type": "object", "properties": {"off_ms": {"type": "integer", "minimum": 0, "maximum": 30000, "default": 1000}, "discharge": {"type": "boolean", "default": true}}}}}},
                "responses": ok("Cycle result")}},
            "/v1/devices/{id}/nickname": {"put": {"summary": "Set the persistent nickname (output must be OFF)", "parameters": [id],
                "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["nickname"], "properties": {"nickname": {"type": "string", "maxLength": 24}}}}}},
                "responses": ok("Verified nickname")}},
            "/v1/devices/{id}/firmware": {
                "get": {"summary": "Read USB firmware support and A/B boot state", "parameters": [id], "responses": ok("Firmware support and boot state")},
                "post": {"summary": "Upload firmware, reconnect, verify and confirm the trial", "parameters": [id], "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["data"], "properties": {"data": {"type": "array", "maxItems": 524288, "items": {"type": "integer", "minimum": 0, "maximum": 255}}}}}}}, "responses": ok("Verified firmware result; a failed trial reports verified rollback")}
            },
            "/v1/devices/{id}/firmware/rollback": {"post": {"summary": "Trial-boot and verify the previous A/B image, then confirm rollback", "parameters": [id], "responses": ok("Verified rollback result")}},
            "/v1/operations": {"get": {"summary": "Operation journal", "parameters": [{"name": "device", "in": "query", "schema": {"type": "string"}}, {"name": "limit", "in": "query", "schema": {"type": "integer"}}], "responses": ok("Records")}},
            "/v1/faults": {"get": {"summary": "Faults since the process started (and conditions still active)", "parameters": [{"name": "device", "in": "query", "schema": {"type": "string"}}, {"name": "active", "in": "query", "schema": {"type": "boolean"}}], "responses": ok("Faults")}},
            "/v1/metrics": {"get": {"summary": "Per-device polling and link statistics", "responses": ok("Metrics")}},
            "/v1/trace": {"get": {"summary": "Serial transaction spans", "parameters": [{"name": "trace_id", "in": "query", "schema": {"type": "string"}}, {"name": "device", "in": "query", "schema": {"type": "string"}}], "responses": ok("Spans")}},
            "/v1/events": {"get": {"summary": "Server-sent events: hello, device.added, device.removed, device.offline, device.online, device.locked, status, firmware.progress, operation.started, operation.finished, fault.raised, fault.occurred, fault.cleared, lagged", "responses": {"200": {"description": "text/event-stream"}}}},
            "/v1/devices/{id}/lock": {"post": {"summary": "Lock or unlock a device for every client (config.toml locks cannot be lifted)", "parameters": [id],
                "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["locked"], "properties": {"locked": {"type": "boolean"}}}}}},
                "responses": ok("Lock state")}},
            "/v1/shutdown": {"post": {"summary": "Stop the background process (outputs unchanged)", "responses": ok("Stopping")}},
            "/mcp": {"post": {"summary": "Model Context Protocol (JSON-RPC, Streamable HTTP with JSON responses)", "responses": ok("JSON-RPC response")}}
        },
        "components": {
            "securitySchemes": {"bearer": {"type": "http", "scheme": "bearer"}},
            "responses": {"Error": {"description": "Error. HTTP 400 invalid_argument, 404 not_found, 409 busy, 502 device_failed, 503 link_error, 504 unknown (read status before acting).",
                "content": {"application/json": {"schema": {"type": "object", "properties": {"error": {"type": "object", "required": ["code", "message"], "properties": {
                    "code": {"enum": ["invalid_argument", "not_found", "busy", "device_failed", "link_error", "unknown"]},
                    "message": {"type": "string"}, "trace_id": {"type": "string"}, "uid": {"type": "string"}, "details": {}, "faults": {"type": "array"}}}}}}}}}
        }
    })
}

fn session_cookie(port: u16) -> String {
    // Cookies are shared across ports on one host; the port keeps sessions apart.
    format!("voloo_session_{port}")
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| {
            kv.trim()
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('='))
                .map(str::to_string)
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LockBody {
    locked: bool,
}
async fn device_lock(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<LockBody>,
) -> ApiResult {
    Ok(Json(app.manager.set_lock(&id, body.locked)?))
}

/// Issues a one-time login code (valid 60 s) for opening the web page.
async fn ui_code(State(app): State<App>) -> Json<Value> {
    let code = random_token();
    let mut codes = app.codes.lock().unwrap();
    codes.retain(|_, at| at.elapsed() < Duration::from_secs(60));
    codes.insert(code.clone(), Instant::now());
    Json(
        json!({"url": format!("http://127.0.0.1:{}/ui/login?code={code}", app.port), "expires_s": 60}),
    )
}

#[derive(Deserialize)]
struct LoginQuery {
    code: Option<String>,
}
async fn ui_login(State(app): State<App>, Query(q): Query<LoginQuery>) -> Response {
    let valid = q
        .code
        .and_then(|c| app.codes.lock().unwrap().remove(&c))
        .is_some_and(|at| at.elapsed() < Duration::from_secs(60));
    if !valid {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "<!doctype html><meta charset=utf-8><title>Voloo</title><p style=\"font:16px system-ui;margin:3em\">This link has expired or was already used. Run <code>voloo ui</code> again.</p>",
        )
            .into_response();
    }
    let session = random_token();
    app.sessions.lock().unwrap().insert(session.clone());
    (
        StatusCode::SEE_OTHER,
        [
            (
                header::SET_COOKIE,
                format!(
                    "{}={session}; HttpOnly; SameSite=Strict; Path=/",
                    session_cookie(app.port)
                ),
            ),
            (header::LOCATION, "/ui/".to_string()),
        ],
    )
        .into_response()
}

const UI_HTML: &str = include_str!("../web/index.html");
const UI_JS: &str = include_str!("../web/app.js");
const UI_I18N: &str = include_str!("../web/i18n.js");
const UI_CSS: &str = include_str!("../web/app.css");
fn asset(body: &'static str, kind: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, kind),
            (header::CACHE_CONTROL, "no-store"),
            (header::HeaderName::from_static("x-content-type-options"), "nosniff"),
            (header::HeaderName::from_static("referrer-policy"), "no-referrer"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
            ),
        ],
        body,
    )
        .into_response()
}
async fn ui_index() -> Response {
    asset(UI_HTML, "text/html; charset=utf-8")
}
async fn ui_js() -> Response {
    asset(UI_JS, "text/javascript; charset=utf-8")
}
async fn ui_i18n() -> Response {
    asset(UI_I18N, "text/javascript; charset=utf-8")
}
async fn ui_css() -> Response {
    asset(UI_CSS, "text/css; charset=utf-8")
}
