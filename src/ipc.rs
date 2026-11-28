//! Versioned engine control IPC over a Unix domain socket.
//!
//! Contract: `contracts/ipc.schema.json` and `docs/IPC.md`.
//!
//! Framing: every message is a 4-byte big-endian unsigned length followed by
//! exactly that many bytes of UTF-8 JSON. Frames larger than
//! [`MAX_FRAME_BYTES`] are rejected and the connection is closed. A client may
//! send several requests sequentially on one connection; each request receives
//! exactly one response carrying the same `request_id`.
//!
//! This is the control plane only. No media payload ever crosses this socket,
//! and a slow or misbehaving client cannot block the media threads: every
//! connection runs on its own bounded thread with read/write timeouts, and the
//! number of concurrent connections is capped.

use serde_json::{json, Map, Value};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

pub const IPC_VERSION: u64 = 1;
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const MAX_CONNECTIONS: usize = 8;
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_REQUEST_ID_BYTES: usize = 64;

/// Methods understood by IPC version 1. `scene.apply` is part of the contract
/// but fails closed until a real compositor acknowledges frame-boundary
/// application (docs/STUDIO-ARCHITECTURE.md safety gates).
pub const METHODS: [&str; 8] = [
    "engine.status",
    "engine.capabilities",
    "stream.start",
    "stream.stop",
    "scene.validate",
    "scene.apply",
    "audio.status",
    "audio.configure",
];

/// Error codes. Kept small and stable; callers map them to HTTP statuses.
pub mod code {
    pub const INVALID_FRAME: &str = "invalid_frame";
    pub const FRAME_TOO_LARGE: &str = "frame_too_large";
    pub const INVALID_REQUEST: &str = "invalid_request";
    pub const UNSUPPORTED_VERSION: &str = "unsupported_version";
    pub const UNKNOWN_METHOD: &str = "unknown_method";
    pub const INVALID_PARAMS: &str = "invalid_params";
    pub const INVALID_SCENE: &str = "invalid_scene";
    pub const NOT_IMPLEMENTED: &str = "not_implemented";
    pub const ENGINE_START_FAILED: &str = "engine_start_failed";
    pub const BUSY: &str = "busy";
    /// No composing media loop is running to acknowledge a Program.
    pub const PROGRAM_INACTIVE: &str = "program_inactive";
    pub const PROGRAM_TIMEOUT: &str = "program_timeout";
    pub const FORBIDDEN: &str = "forbidden";
}

/// Snapshot of engine state reported through `engine.status`.
///
/// `native_worker_running` is worker activity only; it is never evidence of
/// YouTube ingest, so `youtube_ingest_confirmed` is always reported as false
/// until an independent confirmation path exists.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineStatus {
    pub streaming_requested: bool,
    pub native_worker_running: bool,
    pub restart_attempts: u64,
    pub last_error: String,
    /// Unix time (ms) the running native worker started, for elapsed
    /// on-air time in the apps; None while no worker runs.
    pub native_worker_started_unix_ms: Option<u64>,
}

/// Identity of this engine process. The management plane compares
/// `instance_id` across calls to detect engine restarts and reconcile its view
/// (it must never assume state survived a restart).
#[derive(Debug, Clone, PartialEq)]
pub struct EngineInstance {
    pub instance_id: String,
    pub started: std::time::Instant,
}

impl EngineInstance {
    pub fn new() -> Self {
        let mut bytes = [0u8; 8];
        // /dev/urandom is always present on Linux; fall back to time+pid.
        let random = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
        if random.is_err() {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64).unwrap_or_default();
            bytes = (nanos ^ ((std::process::id() as u64) << 32)).to_be_bytes();
        }
        let instance_id = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Self { instance_id, started: std::time::Instant::now() }
    }
}

impl Default for EngineInstance {
    fn default() -> Self { Self::new() }
}

impl EngineStatus {
    pub fn state(&self) -> &'static str {
        match (self.native_worker_running, self.streaming_requested) {
            (true, true) => "native_worker_active",
            (true, false) => "stopping",
            (false, true) => "starting_or_retrying",
            (false, false) => "idle",
        }
    }
}

/// The engine operations the IPC layer may invoke. Implemented by the real
/// stream controller in the `stephania` binary and by fakes in tests.
pub trait EngineControl: Send + Sync {
    fn instance(&self) -> &EngineInstance;
    fn status(&self) -> EngineStatus;
    /// Start streaming. `output` is the RTMPS destination (URL with the
    /// stream key) sent by control; None falls back to the engine's own
    /// configuration (`STEPHANIA_OUTPUT`, development). Implementations
    /// must never log or report the output.
    fn start(&self, output: Option<&str>) -> Result<(), String>;
    fn stop(&self);
    /// Apply a validated scene to Program and return only after the media
    /// thread acknowledged frame-boundary application. Default: fail closed.
    fn apply_scene(&self, _scene_id: &str, _revision: u64, _scene: &Value, _transition: crate::compositor::Transition) -> Result<Value, IpcError> {
        Err(ipc_err(code::NOT_IMPLEMENTED, "compositor is not enabled; Program unchanged"))
    }
    /// Program on air (scene_id, revision); revision 0 = built-in HDMI only.
    fn program(&self) -> (String, u64) { (String::new(), 0) }
    fn compositor_enabled(&self) -> bool { false }
    /// Live non-HDMI sources (crate::composition::SourceRegistry::describe).
    fn sources(&self) -> Vec<Value> { Vec::new() }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IpcError {
    pub code: &'static str,
    pub message: String,
}

fn ipc_err(code: &'static str, message: impl Into<String>) -> IpcError {
    IpcError { code, message: message.into() }
}

/// Read one frame. `Ok(None)` means the peer closed cleanly between frames.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, IpcError> {
    let mut header = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match reader.read(&mut header[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(ipc_err(code::INVALID_FRAME, "truncated length prefix")),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(ipc_err(code::INVALID_FRAME, format!("read failed: {}", e.kind()))),
        }
    }
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 {
        return Err(ipc_err(code::INVALID_FRAME, "empty frame"));
    }
    if len > MAX_FRAME_BYTES {
        return Err(ipc_err(code::FRAME_TOO_LARGE, format!("frame exceeds {MAX_FRAME_BYTES} bytes")));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)
        .map_err(|e| ipc_err(code::INVALID_FRAME, format!("truncated frame: {}", e.kind())))?;
    Ok(Some(body))
}

pub fn write_frame<W: Write>(writer: &mut W, body: &[u8]) -> io::Result<()> {
    if body.is_empty() || body.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame size out of bounds"));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
    writer.write_all(&out)?;
    writer.flush()
}

fn valid_request_id(id: &str) -> bool {
    (1..=MAX_REQUEST_ID_BYTES).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

pub fn ok_response(request_id: &str, result: Value) -> Value {
    json!({"version": IPC_VERSION, "request_id": request_id, "ok": true, "result": result})
}

pub fn error_response(request_id: Option<&str>, error: &IpcError) -> Value {
    json!({
        "version": IPC_VERSION,
        "request_id": request_id,
        "ok": false,
        "error": {"code": error.code, "message": error.message},
    })
}

fn status_result(s: &EngineStatus, instance: &EngineInstance) -> Value {
    json!({
        "instance_id": instance.instance_id,
        "uptime_ms": instance.started.elapsed().as_millis() as u64,
        "state": s.state(),
        "streaming_requested": s.streaming_requested,
        "native_worker_running": s.native_worker_running,
        "youtube_ingest_confirmed": false,
        "restart_attempts": s.restart_attempts,
        "last_error": s.last_error,
        "native_worker_started_unix_ms": s.native_worker_started_unix_ms,
    })
}

/// `program` is the stored scene on air, or null when none was applied in
/// this engine process (HDMI passthrough default).
fn with_program(mut status: Value, program: (String, u64)) -> Value {
    status["program"] = if program.0.is_empty() { Value::Null } else { json!({"scene_id": program.0, "revision": program.1}) };
    status
}

fn capabilities_result(instance: &EngineInstance) -> Value {
    json!({
        "ipc_version": IPC_VERSION,
        "methods": METHODS,
        "scene_versions": [1],
        "program_apply": false,
        "compositor": false,
        "phone_camera": false,
        "max_frame_bytes": MAX_FRAME_BYTES,
        "instance_id": instance.instance_id,
    })
}

fn params_object(params: Option<&Value>) -> Result<Option<&Map<String, Value>>, IpcError> {
    match params {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(m)) => Ok(Some(m)),
        Some(_) => Err(ipc_err(code::INVALID_PARAMS, "params must be an object")),
    }
}

fn no_params(params: Option<&Value>) -> Result<(), IpcError> {
    match params_object(params)? {
        Some(m) if !m.is_empty() => Err(ipc_err(code::INVALID_PARAMS, "method takes no params")),
        _ => Ok(()),
    }
}

/// `{"type":"cut"}` (default) or `{"type":"fade","duration_ms":1..=5000}`.
pub fn parse_transition(v: Option<&Value>) -> Result<crate::compositor::Transition, IpcError> {
    use crate::compositor::{Transition, MAX_FADE_MS};
    let bad = || ipc_err(code::INVALID_PARAMS, "transition must be {type: cut} or {type: fade, duration_ms: 1..=5000}");
    let Some(v) = v else { return Ok(Transition::Cut) };
    let m = v.as_object().ok_or_else(bad)?;
    match m.get("type").and_then(Value::as_str) {
        Some("cut") if m.len() == 1 => Ok(Transition::Cut),
        Some("fade") if m.len() == 2 => {
            let ms = m.get("duration_ms").and_then(Value::as_u64).filter(|d| (1..=MAX_FADE_MS).contains(d)).ok_or_else(bad)?;
            Ok(Transition::Fade { duration_us: ms * 1000 })
        }
        _ => Err(bad()),
    }
}

/// `stream.start` params: none, `{}` or `{output}`. `output` is an RTMPS
/// URL `rtmps://host[:port]/app/stream_key` (crate::rtmps_tls::Target). The
/// error never echoes the value: it carries the stream key.
fn stream_start_output(params: Option<&Value>) -> Result<Option<&str>, IpcError> {
    let bad = || ipc_err(code::INVALID_PARAMS, "params.output must be an RTMPS URL rtmps://host[:port]/app/stream_key");
    let Some(m) = params_object(params)? else { return Ok(None) };
    if m.keys().any(|k| k != "output") {
        return Err(ipc_err(code::INVALID_PARAMS, "only params.output is accepted"));
    }
    let Some(output) = m.get("output") else { return Ok(None) };
    let output = output.as_str().ok_or_else(bad)?;
    crate::rtmps_tls::Target::parse(output).map_err(|_| bad())?;
    Ok(Some(output))
}

fn scene_param(params: Option<&Value>) -> Result<&Value, IpcError> {
    let m = params_object(params)?.ok_or_else(|| ipc_err(code::INVALID_PARAMS, "params.scene required"))?;
    if m.len() != 1 {
        return Err(ipc_err(code::INVALID_PARAMS, "only params.scene is accepted"));
    }
    m.get("scene").ok_or_else(|| ipc_err(code::INVALID_PARAMS, "params.scene required"))
}

/// Dispatch one parsed request. Pure with respect to I/O so it can be
/// exercised exhaustively in unit tests.
pub fn dispatch(engine: &dyn EngineControl, method: &str, params: Option<&Value>) -> Result<Value, IpcError> {
    match method {
        "engine.status" => {
            no_params(params)?;
            let mut status = with_program(status_result(&engine.status(), engine.instance()), engine.program());
            status["sources"] = Value::Array(engine.sources());
            Ok(status)
        }
        "engine.capabilities" => {
            no_params(params)?;
            let mut caps = capabilities_result(engine.instance());
            caps["program_apply"] = json!(engine.compositor_enabled());
            caps["compositor"] = json!(engine.compositor_enabled());
            Ok(caps)
        }
        "stream.start" => {
            let output = stream_start_output(params)?;
            engine.start(output).map_err(|e| ipc_err(code::ENGINE_START_FAILED, e))?;
            // Accepted means the request was taken, not that ingest works.
            Ok(json!({"accepted": true, "streaming_requested": true, "youtube_ingest_confirmed": false}))
        }
        "stream.stop" => {
            no_params(params)?;
            engine.stop();
            Ok(json!({"accepted": true, "streaming_requested": false}))
        }
        "scene.validate" => {
            let scene = scene_param(params)?;
            crate::scene::validate(scene)
                .map_err(|e| ipc_err(code::INVALID_SCENE, e.to_string()))?;
            Ok(json!({"valid": true, "version": scene["version"].clone()}))
        }
        "scene.apply" => {
            let m = params_object(params)?.ok_or_else(|| ipc_err(code::INVALID_PARAMS, "params required"))?;
            if !(3..=4).contains(&m.len()) || (m.len() == 4 && !m.contains_key("transition")) {
                return Err(ipc_err(code::INVALID_PARAMS, "params must be {scene, scene_id, revision, transition?}"));
            }
            let transition = parse_transition(m.get("transition"))?;
            let scene = m.get("scene").ok_or_else(|| ipc_err(code::INVALID_PARAMS, "params.scene required"))?;
            let scene_id = m.get("scene_id").and_then(Value::as_str)
                .filter(|s| (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
                .ok_or_else(|| ipc_err(code::INVALID_PARAMS, "params.scene_id invalid"))?;
            let revision = m.get("revision").and_then(Value::as_u64).filter(|r| *r >= 1)
                .ok_or_else(|| ipc_err(code::INVALID_PARAMS, "params.revision must be >= 1"))?;
            crate::scene::validate(scene)
                .map_err(|e| ipc_err(code::INVALID_SCENE, e.to_string()))?;
            // Fail closed: success only after a frame-boundary acknowledgement.
            engine.apply_scene(scene_id, revision, scene, transition)
        }
        // Audio inputs (docs/AUDIO.md): configuration applies live; status
        // carries per-input health and levels while streaming.
        "audio.status" => {
            no_params(params)?;
            Ok(crate::audio::control().status())
        }
        "audio.configure" => {
            let p = params.ok_or_else(|| ipc_err(code::INVALID_PARAMS, "params required"))?;
            if p.as_object().is_none_or(|m| m.len() != 1 || !m.contains_key("inputs")) {
                return Err(ipc_err(code::INVALID_PARAMS, "params must be {inputs}"));
            }
            crate::audio::control().configure(p).map_err(|e| ipc_err(code::INVALID_PARAMS, e))
        }
        _ => Err(ipc_err(code::UNKNOWN_METHOD, "unknown method")),
    }
}

/// Parse and execute one request frame, always producing a response value.
pub fn handle_frame(engine: &dyn EngineControl, frame: &[u8]) -> Value {
    let parsed: Value = match serde_json::from_slice(frame) {
        Ok(v) => v,
        Err(_) => return error_response(None, &ipc_err(code::INVALID_REQUEST, "body is not JSON")),
    };
    let Value::Object(req) = parsed else {
        return error_response(None, &ipc_err(code::INVALID_REQUEST, "request must be an object"));
    };
    let request_id = match req.get("request_id") {
        Some(Value::String(id)) if valid_request_id(id) => id.as_str(),
        _ => return error_response(None, &ipc_err(code::INVALID_REQUEST, "invalid request_id")),
    };
    if req.keys().any(|k| !matches!(k.as_str(), "version" | "request_id" | "method" | "params")) {
        return error_response(Some(request_id), &ipc_err(code::INVALID_REQUEST, "unexpected property"));
    }
    match req.get("version") {
        Some(Value::Number(n)) if n.as_u64() == Some(IPC_VERSION) => {}
        _ => return error_response(Some(request_id), &ipc_err(code::UNSUPPORTED_VERSION, "version must be 1")),
    }
    let Some(Value::String(method)) = req.get("method") else {
        return error_response(Some(request_id), &ipc_err(code::INVALID_REQUEST, "method required"));
    };
    match dispatch(engine, method, req.get("params")) {
        Ok(result) => ok_response(request_id, result),
        Err(e) => error_response(Some(request_id), &e),
    }
}

/// Serve one connection until the peer closes, a timeout fires or a framing
/// error occurs. Framing errors are answered once and then the connection is
/// closed, because the stream position can no longer be trusted.
pub fn serve_connection<S: Read + Write>(engine: &dyn EngineControl, stream: &mut S) {
    loop {
        match read_frame(stream) {
            Ok(None) => return,
            Ok(Some(frame)) => {
                let response = handle_frame(engine, &frame);
                let body = serde_json::to_vec(&response).unwrap_or_default();
                if write_frame(stream, &body).is_err() {
                    return;
                }
            }
            Err(e) => {
                let body = serde_json::to_vec(&error_response(None, &e)).unwrap_or_default();
                let _ = write_frame(stream, &body);
                return;
            }
        }
    }
}

/// Which local users may use the control socket. Root and the engine's own
/// user are always allowed; additional uids (the control service user) come
/// from `STEPHANIA_IPC_ALLOWED_UIDS` (comma separated). Filesystem permissions
/// still apply first; this is defence in depth against a misconfigured mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerPolicy {
    pub allowed_uids: Vec<u32>,
}

impl PeerPolicy {
    pub fn from_env() -> Result<Self, String> {
        let mut allowed_uids = Vec::new();
        if let Ok(list) = std::env::var("STEPHANIA_IPC_ALLOWED_UIDS") {
            for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                allowed_uids.push(part.parse::<u32>().map_err(|_| format!("invalid uid in STEPHANIA_IPC_ALLOWED_UIDS: {part}"))?);
            }
        }
        Ok(Self { allowed_uids })
    }

    pub fn allows(&self, peer_uid: u32, own_uid: u32) -> bool {
        peer_uid == 0 || peer_uid == own_uid || self.allowed_uids.contains(&peer_uid)
    }
}

/// Peer uid via SO_PEERCRED. `None` when the platform cannot tell.
#[cfg(target_os = "linux")]
pub fn peer_uid(stream: &UnixStream) -> Option<u32> {
    use std::os::unix::io::AsRawFd;
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: valid fd owned by `stream`; `cred`/`len` point to properly sized locals.
    let rc = unsafe {
        libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(), &mut len)
    };
    (rc == 0 && len as usize == std::mem::size_of::<libc::ucred>()).then_some(cred.uid)
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn peer_uid(_stream: &UnixStream) -> Option<u32> { None }

#[cfg(unix)]
fn own_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(unix)]
struct ConnectionSlot(Arc<AtomicUsize>);

#[cfg(unix)]
impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(unix)]
fn try_acquire(active: &Arc<AtomicUsize>) -> Option<ConnectionSlot> {
    let mut current = active.load(Ordering::Acquire);
    loop {
        if current >= MAX_CONNECTIONS {
            return None;
        }
        match active.compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Some(ConnectionSlot(Arc::clone(active))),
            Err(actual) => current = actual,
        }
    }
}

/// Bind the engine socket at `path`.
///
/// A pre-existing *socket* at the path is treated as stale and removed; any
/// other file type is left untouched and binding fails, so a misconfigured
/// path can never delete unrelated data. The socket is chmod 0660 so access
/// is controlled by the owning user/group (systemd `RuntimeDirectory=` with
/// mode 0750 restricts the directory further).
#[cfg(unix)]
pub fn bind(path: &std::path::Path) -> io::Result<UnixListener> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_socket() {
            std::fs::remove_file(path)?;
        } else {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "engine socket path exists and is not a socket"));
        }
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
    Ok(listener)
}

/// Accept connections forever. Each accepted connection gets its own thread;
/// connections beyond [`MAX_CONNECTIONS`] receive a `busy` error and are closed.
#[cfg(unix)]
pub fn serve(listener: UnixListener, engine: Arc<dyn EngineControl>, policy: PeerPolicy) {
    let active = Arc::new(AtomicUsize::new(0));
    let own = own_uid();
    for connection in listener.incoming() {
        let mut stream: UnixStream = match connection {
            Ok(s) => s,
            Err(e) => { eprintln!("engine ipc accept error: {e}"); continue; }
        };
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        // Fail closed on Linux if credentials cannot be read.
        let permitted = match peer_uid(&stream) {
            Some(uid) => policy.allows(uid, own),
            None => !cfg!(target_os = "linux"),
        };
        if !permitted {
            let body = serde_json::to_vec(&error_response(None, &ipc_err(code::FORBIDDEN, "peer not permitted"))).unwrap_or_default();
            let _ = write_frame(&mut stream, &body);
            continue;
        }
        let Some(slot) = try_acquire(&active) else {
            let body = serde_json::to_vec(&error_response(None, &ipc_err(code::BUSY, "too many connections"))).unwrap_or_default();
            let _ = write_frame(&mut stream, &body);
            continue;
        };
        let engine = Arc::clone(&engine);
        let spawned = std::thread::Builder::new().name("stephania-ipc".into()).spawn(move || {
            let _slot = slot;
            serve_connection(engine.as_ref(), &mut stream);
        });
        if let Err(e) = spawned {
            eprintln!("engine ipc thread spawn failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<&'static str>>,
        outputs: Mutex<Vec<Option<String>>>,
        fail_start: bool,
        /// Models STEPHANIA_COMPOSITOR=0; the default fake is the default engine.
        compositor_off: bool,
        instance: EngineInstance,
    }
    impl EngineControl for Fake {
        fn instance(&self) -> &EngineInstance { &self.instance }
        fn status(&self) -> EngineStatus {
            self.calls.lock().unwrap().push("status");
            EngineStatus { streaming_requested: false, native_worker_running: false, restart_attempts: 3, last_error: "x\"y".into(), native_worker_started_unix_ms: None }
        }
        fn start(&self, output: Option<&str>) -> Result<(), String> {
            self.calls.lock().unwrap().push("start");
            self.outputs.lock().unwrap().push(output.map(str::to_owned));
            if self.fail_start { Err("STEPHANIA_OUTPUT is not configured".into()) } else { Ok(()) }
        }
        fn stop(&self) { self.calls.lock().unwrap().push("stop"); }
        fn compositor_enabled(&self) -> bool { !self.compositor_off }
        fn apply_scene(&self, _: &str, _: u64, _: &Value, _: crate::compositor::Transition) -> Result<Value, IpcError> {
            if self.compositor_off {
                return Err(ipc_err(code::NOT_IMPLEMENTED, "compositor disabled; Program unchanged"));
            }
            // Never streaming in the fake: same answer as the idle real engine.
            Err(ipc_err(code::PROGRAM_INACTIVE, "no stream is running; Program unchanged"))
        }
    }

    fn call(engine: &Fake, req: Value) -> Value {
        handle_frame(engine, &serde_json::to_vec(&req).unwrap())
    }

    #[test]
    fn status_round_trip_preserves_request_id_and_never_claims_ingest() {
        let f = Fake::default();
        let r = call(&f, json!({"version":1,"request_id":"req-001","method":"engine.status"}));
        assert_eq!(r["ok"], true);
        assert_eq!(r["request_id"], "req-001");
        assert_eq!(r["result"]["state"], "idle");
        assert_eq!(r["result"]["youtube_ingest_confirmed"], false);
        assert_eq!(r["result"]["last_error"], "x\"y");
        assert_eq!(r["result"]["instance_id"].as_str().unwrap().len(), 16);
        assert!(r["result"]["uptime_ms"].is_u64());
        assert!(r["result"]["program"].is_null(), "no scene applied: program must be null");
        assert_eq!(r["result"]["sources"], json!([]));
    }

    #[test]
    fn instances_are_distinct() {
        assert_ne!(EngineInstance::new().instance_id, EngineInstance::new().instance_id);
    }

    #[test]
    fn peer_policy() {
        let p = PeerPolicy { allowed_uids: vec![1001] };
        assert!(p.allows(0, 999));
        assert!(p.allows(999, 999));
        assert!(p.allows(1001, 999));
        assert!(!p.allows(1002, 999));
    }

    #[test]
    fn start_failure_is_reported_not_swallowed() {
        let f = Fake { fail_start: true, ..Default::default() };
        let r = call(&f, json!({"version":1,"request_id":"a","method":"stream.start","params":{}}));
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"]["code"], code::ENGINE_START_FAILED);
        assert_eq!(*f.calls.lock().unwrap(), vec!["start"]);
    }

    #[test]
    fn stop_and_start_reach_engine() {
        let f = Fake::default();
        assert_eq!(call(&f, json!({"version":1,"request_id":"a","method":"stream.start"}))["result"]["accepted"], true);
        assert_eq!(call(&f, json!({"version":1,"request_id":"b","method":"stream.stop"}))["ok"], true);
        assert_eq!(*f.calls.lock().unwrap(), vec!["start", "stop"]);
    }

    #[test]
    fn start_passes_output_to_engine_and_never_echoes_a_bad_one() {
        let f = Fake::default();
        let url = "rtmps://a.rtmps.youtube.com/live2/abcd-efgh-ijkl-mnop";
        let r = call(&f, json!({"version":1,"request_id":"a","method":"stream.start","params":{"output":url}}));
        assert_eq!(r["ok"], true, "{r}");
        call(&f, json!({"version":1,"request_id":"b","method":"stream.start"}));
        assert_eq!(*f.outputs.lock().unwrap(), vec![Some(url.to_string()), None]);
        let secret = "rtmps://a.rtmps.youtube.com/live2/SECRET?x=1";
        let r = call(&f, json!({"version":1,"request_id":"c","method":"stream.start","params":{"output":secret}}));
        assert_eq!(r["error"]["code"], code::INVALID_PARAMS);
        assert!(!r.to_string().contains("SECRET"));
    }

    #[test]
    fn rejects_malformed_requests_without_touching_engine() {
        let f = Fake::default();
        for (req, expected) in [
            (json!({"version":2,"request_id":"a","method":"engine.status"}), code::UNSUPPORTED_VERSION),
            (json!({"version":1,"request_id":"bad id","method":"engine.status"}), code::INVALID_REQUEST),
            (json!({"version":1,"request_id":"a"}), code::INVALID_REQUEST),
            (json!({"version":1,"request_id":"a","method":"x.y"}), code::UNKNOWN_METHOD),
            (json!({"version":1,"request_id":"a","method":"stream.start","params":{"force":true}}), code::INVALID_PARAMS),
            (json!({"version":1,"request_id":"a","method":"stream.start","params":[]}), code::INVALID_PARAMS),
            (json!({"version":1,"request_id":"a","method":"stream.start","params":{"output":1}}), code::INVALID_PARAMS),
            (json!({"version":1,"request_id":"a","method":"stream.start","params":{"output":"rtmp://a.rtmp.youtube.com/live2/k"}}), code::INVALID_PARAMS),
            (json!({"version":1,"request_id":"a","method":"stream.start","params":{"output":"rtmps://a.rtmps.youtube.com/live2/k","force":true}}), code::INVALID_PARAMS),
            (json!({"version":1,"request_id":"a","method":"engine.status","extra":1}), code::INVALID_REQUEST),
            (json!([1]), code::INVALID_REQUEST),
        ] {
            let r = call(&f, req.clone());
            assert_eq!(r["ok"], false, "{req}");
            assert_eq!(r["error"]["code"], expected, "{req}");
        }
        assert!(f.calls.lock().unwrap().is_empty());
        let r = handle_frame(&f, b"not json");
        assert_eq!(r["error"]["code"], code::INVALID_REQUEST);
        assert_eq!(r["request_id"], Value::Null);
    }

    #[test]
    fn scene_apply_fails_closed_even_for_valid_scene() {
        let scene = json!({"version":1,"name":"LT","layers":[{"id":"h","source":"hdmi","x":0,"y":0,"width":1920,"height":1080,"visible":true}]});
        let apply = json!({"version":1,"request_id":"p","method":"scene.apply","params":{"scene":scene.clone(),"scene_id":"lt","revision":1}});
        let f = Fake::default();
        let v = call(&f, json!({"version":1,"request_id":"v","method":"scene.validate","params":{"scene":scene}}));
        assert_eq!(v["result"]["valid"], true);
        let caps = call(&f, json!({"version":1,"request_id":"c","method":"engine.capabilities"}));
        assert_eq!(caps["result"]["program_apply"], true);
        // Default engine, nothing streaming: refused, Program unchanged.
        let a = call(&f, apply.clone());
        assert_eq!(a["ok"], false);
        assert_eq!(a["error"]["code"], code::PROGRAM_INACTIVE);
        // Compositor switched off: capabilities and apply both say so.
        let off = Fake { compositor_off: true, ..Default::default() };
        assert_eq!(call(&off, json!({"version":1,"request_id":"c","method":"engine.capabilities"}))["result"]["program_apply"], false);
        assert_eq!(call(&off, apply)["error"]["code"], code::NOT_IMPLEMENTED);
        let bad = call(&f, json!({"version":1,"request_id":"q","method":"scene.validate","params":{"scene":{"version":1}}}));
        assert_eq!(bad["error"]["code"], code::INVALID_SCENE);
    }


    #[test]
    fn framing_bounds() {
        let mut oversized = Vec::new();
        oversized.extend_from_slice(&((MAX_FRAME_BYTES as u32) + 1).to_be_bytes());
        assert_eq!(read_frame(&mut oversized.as_slice()).unwrap_err().code, code::FRAME_TOO_LARGE);
        assert_eq!(read_frame(&mut [0u8, 0, 0, 0].as_slice()).unwrap_err().code, code::INVALID_FRAME);
        assert_eq!(read_frame(&mut [0u8, 0].as_slice()).unwrap_err().code, code::INVALID_FRAME);
        assert_eq!(read_frame(&mut [0u8, 0, 0, 5, b'{'].as_slice()).unwrap_err().code, code::INVALID_FRAME);
        assert_eq!(read_frame(&mut [].as_slice()).unwrap(), None);
        assert!(write_frame(&mut Vec::new(), &vec![b'x'; MAX_FRAME_BYTES + 1]).is_err());
    }

    #[test]
    fn shared_ipc_fixtures_match() {
        // contracts/fixtures/ipc/*.json: {"request": <frame JSON>, "expect": {"ok":bool, "code"?:str}}
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/fixtures/ipc");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let fixture: Value = serde_json::from_str(&std::fs::read_to_string(entry.path()).unwrap()).unwrap();
            let f = Fake::default();
            let r = call(&f, fixture["request"].clone());
            let name = entry.file_name();
            assert_eq!(r["ok"], fixture["expect"]["ok"], "{name:?}");
            if let Some(c) = fixture["expect"].get("code") {
                assert_eq!(&r["error"]["code"], c, "{name:?}");
            }
            count += 1;
        }
        assert!(count >= 5);
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_end_to_end_with_connection_cap_and_stale_socket() {
        let dir = std::env::temp_dir().join(format!("stephania-ipc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("engine.sock");
        // A regular file at the path must not be deleted.
        std::fs::write(&path, b"keep").unwrap();
        assert!(bind(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        std::fs::remove_file(&path).unwrap();
        // A stale socket is replaced.
        drop(UnixListener::bind(&path).unwrap());
        let listener = bind(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o660);
        let engine: Arc<dyn EngineControl> = Arc::new(Fake::default());
        #[cfg(target_os = "linux")]
        {
            let probe = UnixListener::bind(dir.join("probe.sock")).unwrap();
            let c = UnixStream::connect(dir.join("probe.sock")).unwrap();
            let (server_side, _) = probe.accept().unwrap();
            assert_eq!(peer_uid(&server_side), Some(own_uid()));
            drop(c);
        }
        std::thread::spawn(move || serve(listener, engine, PeerPolicy { allowed_uids: vec![] }));

        let request = |s: &mut UnixStream, id: &str| {
            let body = serde_json::to_vec(&json!({"version":1,"request_id":id,"method":"engine.capabilities"})).unwrap();
            write_frame(s, &body).unwrap();
            serde_json::from_slice::<Value>(&read_frame(s).unwrap().unwrap()).unwrap()
        };
        let mut held: Vec<UnixStream> = Vec::new();
        for i in 0..MAX_CONNECTIONS {
            let mut s = UnixStream::connect(&path).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            // Two sequential requests on one connection.
            assert_eq!(request(&mut s, &format!("c{i}a"))["request_id"], format!("c{i}a"));
            assert_eq!(request(&mut s, &format!("c{i}b"))["result"]["program_apply"], true);
            held.push(s);
        }
        let mut extra = UnixStream::connect(&path).unwrap();
        extra.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let busy: Value = serde_json::from_slice(&read_frame(&mut extra).unwrap().unwrap()).unwrap();
        assert_eq!(busy["error"]["code"], code::BUSY);
        drop(held);
        // Slots are released after clients disconnect.
        let mut ok = None;
        for _ in 0..50 {
            let mut s = UnixStream::connect(&path).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let body = serde_json::to_vec(&json!({"version":1,"request_id":"z","method":"engine.status"})).unwrap();
            if write_frame(&mut s, &body).is_ok() {
                if let Ok(Some(frame)) = read_frame(&mut s) {
                    let v: Value = serde_json::from_slice(&frame).unwrap();
                    if v["ok"] == true { ok = Some(v); break; }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(ok.expect("slot released")["result"]["state"], "idle");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audio_status_and_configure_over_ipc() {
        let f = Fake::default();
        let r = call(&f, json!({"version":1,"request_id":"a1","method":"audio.status"}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["inputs"][0]["id"], "hdmi");
        let r = call(&f, json!({"version":1,"request_id":"a2","method":"audio.configure",
            "params":{"inputs":[{"id":"hdmi","gain_db":-3.0}]}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["inputs"][0]["gain_db"], -3.0);
        let r = call(&f, json!({"version":1,"request_id":"a3","method":"audio.configure",
            "params":{"inputs":[{"id":"hdmi","gain_db":99}]}}));
        assert_eq!(r["error"]["code"], "invalid_params");
        let r = call(&f, json!({"version":1,"request_id":"a4","method":"audio.configure","params":{"inputs":[],"x":1}}));
        assert_eq!(r["error"]["code"], "invalid_params");
        // Restore the shared default for other tests.
        call(&f, json!({"version":1,"request_id":"a5","method":"audio.configure","params":{"inputs":[{"id":"hdmi","gain_db":0.0}]}}));
    }

}
