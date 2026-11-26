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
pub const METHODS: [&str; 6] = [
    "engine.status",
    "engine.capabilities",
    "stream.start",
    "stream.stop",
    "scene.validate",
    "scene.apply",
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
/// stream controller in the `t3box` binary and by fakes in tests.
pub trait EngineControl: Send + Sync {
    fn status(&self) -> EngineStatus;
    fn start(&self) -> Result<(), String>;
    fn stop(&self);
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

fn status_result(s: &EngineStatus) -> Value {
    json!({
        "state": s.state(),
        "streaming_requested": s.streaming_requested,
        "native_worker_running": s.native_worker_running,
        "youtube_ingest_confirmed": false,
        "restart_attempts": s.restart_attempts,
        "last_error": s.last_error,
    })
}

fn capabilities_result() -> Value {
    json!({
        "ipc_version": IPC_VERSION,
        "methods": METHODS,
        "scene_versions": [1],
        "program_apply": false,
        "compositor": false,
        "phone_camera": false,
        "max_frame_bytes": MAX_FRAME_BYTES,
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
        "engine.status" => { no_params(params)?; Ok(status_result(&engine.status())) }
        "engine.capabilities" => { no_params(params)?; Ok(capabilities_result()) }
        "stream.start" => {
            no_params(params)?;
            engine.start().map_err(|e| ipc_err(code::ENGINE_START_FAILED, e))?;
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
            crate::scene::validate_draft_v1(scene)
                .map_err(|e| ipc_err(code::INVALID_SCENE, e.to_string()))?;
            Ok(json!({"valid": true, "version": 1}))
        }
        "scene.apply" => {
            let scene = scene_param(params)?;
            crate::scene::validate_draft_v1(scene)
                .map_err(|e| ipc_err(code::INVALID_SCENE, e.to_string()))?;
            // Fail closed: never report a Program change that did not happen.
            Err(ipc_err(code::NOT_IMPLEMENTED, "compositor is not implemented; Program unchanged"))
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
pub fn serve(listener: UnixListener, engine: Arc<dyn EngineControl>) {
    let active = Arc::new(AtomicUsize::new(0));
    for connection in listener.incoming() {
        let mut stream: UnixStream = match connection {
            Ok(s) => s,
            Err(e) => { eprintln!("engine ipc accept error: {e}"); continue; }
        };
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        let Some(slot) = try_acquire(&active) else {
            let body = serde_json::to_vec(&error_response(None, &ipc_err(code::BUSY, "too many connections"))).unwrap_or_default();
            let _ = write_frame(&mut stream, &body);
            continue;
        };
        let engine = Arc::clone(&engine);
        let spawned = std::thread::Builder::new().name("t3box-ipc".into()).spawn(move || {
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
        fail_start: bool,
    }
    impl EngineControl for Fake {
        fn status(&self) -> EngineStatus {
            self.calls.lock().unwrap().push("status");
            EngineStatus { streaming_requested: false, native_worker_running: false, restart_attempts: 3, last_error: "x\"y".into() }
        }
        fn start(&self) -> Result<(), String> {
            self.calls.lock().unwrap().push("start");
            if self.fail_start { Err("T3BOX_OUTPUT is not configured".into()) } else { Ok(()) }
        }
        fn stop(&self) { self.calls.lock().unwrap().push("stop"); }
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
    fn rejects_malformed_requests_without_touching_engine() {
        let f = Fake::default();
        for (req, expected) in [
            (json!({"version":2,"request_id":"a","method":"engine.status"}), code::UNSUPPORTED_VERSION),
            (json!({"version":1,"request_id":"bad id","method":"engine.status"}), code::INVALID_REQUEST),
            (json!({"version":1,"request_id":"a"}), code::INVALID_REQUEST),
            (json!({"version":1,"request_id":"a","method":"x.y"}), code::UNKNOWN_METHOD),
            (json!({"version":1,"request_id":"a","method":"stream.start","params":{"force":true}}), code::INVALID_PARAMS),
            (json!({"version":1,"request_id":"a","method":"stream.start","params":[]}), code::INVALID_PARAMS),
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
        let f = Fake::default();
        let scene = json!({"version":1,"name":"LT","layers":[{"id":"h","source":"hdmi","x":0,"y":0,"width":1920,"height":1080,"visible":true}]});
        let v = call(&f, json!({"version":1,"request_id":"v","method":"scene.validate","params":{"scene":scene.clone()}}));
        assert_eq!(v["result"]["valid"], true);
        let a = call(&f, json!({"version":1,"request_id":"p","method":"scene.apply","params":{"scene":scene}}));
        assert_eq!(a["ok"], false);
        assert_eq!(a["error"]["code"], code::NOT_IMPLEMENTED);
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
        let dir = std::env::temp_dir().join(format!("t3box-ipc-test-{}", std::process::id()));
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
        std::thread::spawn(move || serve(listener, engine));

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
            assert_eq!(request(&mut s, &format!("c{i}b"))["result"]["program_apply"], false);
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
}
