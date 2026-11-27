//! T3Box Pi 5 stream control, diagnostics and loopback-only configuration UI.
mod stream;
mod wifi;
#[allow(dead_code)] // Pipeline primitives are tested before being wired to a native media backend.
mod policy;
mod html;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use stream::{Config, Controller};
use t3box::ipc::{EngineControl, EngineInstance, EngineStatus, PeerPolicy};

/// Default location of the versioned control socket (docs/IPC.md).
const DEFAULT_ENGINE_SOCKET: &str = "/run/t3box/engine.sock";

struct IpcEngine(Arc<Controller>, EngineInstance);

impl EngineControl for IpcEngine {
    fn instance(&self) -> &EngineInstance { &self.1 }
    fn status(&self) -> EngineStatus {
        let (desired, _process_running, restarts, last_error) = self.0.status();
        EngineStatus {
            streaming_requested: desired,
            native_worker_running: self.0.native_running(),
            restart_attempts: restarts,
            last_error,
        }
    }
    fn start(&self) -> Result<(), String> { self.0.start() }
    fn stop(&self) { self.0.stop() }

    fn apply_scene(&self, scene_id: &str, revision: u64, scene: &serde_json::Value) -> Result<serde_json::Value, t3box::ipc::IpcError> {
        use t3box::composition::{apply, ApplyError, APPLY_TIMEOUT};
        use t3box::ipc::{code, IpcError};
        if !t3box::composition::enabled() {
            return Err(IpcError { code: code::NOT_IMPLEMENTED, message: "T3BOX_COMPOSITOR is not enabled; Program unchanged".into() });
        }
        let program = t3box::compositor::Program::from_scene(scene_id, revision, scene)
            .map_err(|e| IpcError { code: code::INVALID_SCENE, message: e.to_string() })?;
        match apply(program, APPLY_TIMEOUT) {
            Ok(ack) => Ok(serde_json::json!({
                "applied": true, "scene_id": ack.scene_id, "revision": ack.revision,
                "first_frame_index": ack.first_frame_index,
            })),
            Err(ApplyError::Inactive) => Err(IpcError { code: code::PROGRAM_INACTIVE, message: "no stream is running; Program unchanged".into() }),
            Err(ApplyError::Timeout) => Err(IpcError { code: code::PROGRAM_TIMEOUT, message: "no frame boundary within timeout; Program unchanged".into() }),
        }
    }

    fn program(&self) -> (String, u64) { t3box::composition::current_program() }

    fn compositor_enabled(&self) -> bool { t3box::composition::enabled() }

    fn sources(&self) -> Vec<serde_json::Value> { t3box::composition::sources().describe(std::time::Instant::now()) }
}

/// Resolve the IPC socket path. `T3BOX_ENGINE_SOCKET=off` disables it.
fn engine_socket_path() -> Option<std::path::PathBuf> {
    match std::env::var("T3BOX_ENGINE_SOCKET") {
        Ok(v) if v == "off" => None,
        Ok(v) if !v.is_empty() => Some(v.into()),
        _ => Some(DEFAULT_ENGINE_SOCKET.into()),
    }
}

/// Start the control socket on its own thread. Failure to bind never stops
/// the engine: the media plane must keep running if management is broken.
fn start_engine_ipc(controller: &Arc<Controller>) {
    let Some(path) = engine_socket_path() else { return };
    let policy = match PeerPolicy::from_env() {
        Ok(p) => p,
        Err(e) => { eprintln!("T3Box engine IPC disabled: {e}"); return; }
    };
    match t3box::ipc::bind(&path) {
        Ok(listener) => {
            let engine: Arc<dyn EngineControl> = Arc::new(IpcEngine(Arc::clone(controller), EngineInstance::new()));
            let spawned = std::thread::Builder::new().name("t3box-ipc-accept".into())
                .spawn(move || t3box::ipc::serve(listener, engine, policy));
            match spawned {
                Ok(_) => println!("T3Box engine IPC: {}", path.display()),
                Err(e) => eprintln!("T3Box engine IPC thread failed: {e}"),
            }
        }
        Err(e) => eprintln!("T3Box engine IPC unavailable at {}: {e}", path.display()),
    }
}

/// Phone media ingest socket (crate::phone_ingest, docs/PHONE.md).
/// `T3BOX_MEDIA_SOCKET=off` disables it; peers are limited like the IPC
/// socket, by `T3BOX_MEDIA_ALLOWED_UIDS` (the t3box-phone service).
#[cfg(all(unix, feature = "native-aac"))]
fn start_phone_ingest() {
    let path = match std::env::var("T3BOX_MEDIA_SOCKET") {
        Ok(v) if v == "off" => return,
        Ok(v) if !v.is_empty() => std::path::PathBuf::from(v),
        _ => std::path::PathBuf::from("/run/t3box/media.sock"),
    };
    let mut allowed = Vec::new();
    for part in std::env::var("T3BOX_MEDIA_ALLOWED_UIDS").unwrap_or_default().split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.parse() { Ok(uid) => allowed.push(uid), Err(_) => { eprintln!("T3Box phone ingest disabled: invalid uid {part}"); return; } }
    }
    let policy = PeerPolicy { allowed_uids: allowed };
    match t3box::ipc::bind(&path) {
        Ok(listener) => {
            match std::thread::Builder::new().name("t3box-phone-ingest".into()).spawn(move || t3box::phone_ingest::serve(listener, policy)) {
                Ok(_) => println!("T3Box phone ingest: {}", path.display()),
                Err(e) => eprintln!("T3Box phone ingest thread failed: {e}"),
            }
        }
        Err(e) => eprintln!("T3Box phone ingest unavailable at {}: {e}", path.display()),
    }
}

#[cfg(not(all(unix, feature = "native-aac")))]
fn start_phone_ingest() {}

fn video_devices() -> Vec<String> {
    let Ok(entries) = fs::read_dir("/dev") else { return Vec::new() };
    let mut result: Vec<String> = entries.flatten().filter_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        let suffix = name.strip_prefix("video")?;
        if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
            Some(format!("/dev/{name}"))
        } else { None }
    }).collect();
    result.sort();
    result
}

fn audio_capture_available() -> bool {
    fs::read_to_string("/proc/asound/pcm")
        .map(|s| s.lines().any(|line| line.contains("capture")))
        .unwrap_or(false)
}

fn wifi_interfaces() -> Vec<(String, bool)> {
    let Ok(entries) = fs::read_dir("/sys/class/net") else { return Vec::new() };
    let mut result: Vec<(String, bool)> = entries.flatten().filter_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.path().join("wireless").exists() && !name.starts_with("wl") {
            return None;
        }
        let up = fs::read_to_string(entry.path().join("operstate"))
            .map(|s| s.trim() == "up").unwrap_or(false);
        Some((name, up))
    }).collect();
    result.sort();
    result
}

fn cpu_temperature_c() -> Option<f64> {
    let input = fs::read_to_string("/sys/class/thermal/thermal_zone0/temp").ok()?;
    Some(input.trim().parse::<f64>().ok()? / 1000.0)
}

fn escape_json(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => result.push_str(&format!("\\u{:04x}", c as u32)),
            c => result.push(c),
        }
    }
    result
}

fn status_json(controller: &Controller) -> String {
    let videos = video_devices().iter()
        .map(|s| format!("\"{}\"", escape_json(s)))
        .collect::<Vec<_>>().join(",");
    let wifi = wifi_interfaces().iter()
        .map(|(name, up)| format!("{{\"name\":\"{}\",\"up\":{}}}", escape_json(name), up))
        .collect::<Vec<_>>().join(",");
    let temp = cpu_temperature_c()
        .map(|n| format!("{n:.1}")).unwrap_or_else(|| "null".into());
    let now = SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0);
    let (desired, process_running, restarts, last_error) = controller.status();
    let native_running = controller.native_running();
    format!(
        "{{\"state\":\"{}\",\"streaming_requested\":{},\"ffmpeg_process_running\":{},\"native_worker_running\":{native_running},\"youtube_ingest_confirmed\":false,\"video_devices\":[{videos}],\"audio_capture_available\":{},\"wifi_interfaces\":[{wifi}],\"cpu_temperature_c\":{temp},\"restart_attempts\":{},\"last_error\":\"{}\",\"timestamp_unix\":{now}}}",
        if native_running { if desired { "native_worker_active" } else { "stopping" } } else if process_running { "transmitting_unverified" } else { "idle" },
        desired, process_running, audio_capture_available(), restarts, escape_json(&last_error)
    )
}

fn url_decode(value: &str) -> Result<String, String> {
    let input = value.as_bytes();
    let mut data = Vec::with_capacity(input.len());
    let mut i=0;
    while i<input.len() {
        match input[i] {
            b'+' => data.push(b' '),
            b'%' if i+2<input.len() => {
                let v1=(input[i+1] as char).to_digit(16).ok_or("invalid url escape")?;
                let v2=(input[i+2] as char).to_digit(16).ok_or("invalid url escape")?;
                data.push(((v1<<4)|v2) as u8);i+=2;
            },
            b'%' => return Err("incomplete url escape".into()),
            x => data.push(x),
        }
        i+=1;
    }
    String::from_utf8(data).map_err(|_| "invalid UTF-8".into())
}

fn form_field(form: &str, name: &str) -> Result<Option<String>, String> {
    for pair in form.split('&') {
        if let Some((key,value))=pair.split_once('=') {
            if url_decode(key)?==name {return Ok(Some(url_decode(value)?))}
        }
    }
    Ok(None)
}

fn write_response(mut stream: TcpStream, status: &str, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; style-src 'unsafe-inline' 'self'; script-src 'unsafe-inline' 'self'; base-uri 'none'; frame-ancestors 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn handle(mut stream: TcpStream, controller: Arc<Controller>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(4)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(4)));
    // Max 4 KiB headers+body; intended for short Wi-Fi credentials only.
    let mut buf=Vec::new();
    let mut tmp=[0u8; 1024];
    let mut header_end=None;
    let mut needed=0usize;
    loop {
        let Ok(n)=stream.read(&mut tmp) else {return};
        if n==0 { break; }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len()>4096 { write_response(stream,"413 Payload Too Large","application/json","{\"error\":\"request_too_large\"}");return }
        if header_end.is_none() {
            if let Some(end)=buf.windows(4).position(|x|x==b"\r\n\r\n") {
                header_end=Some(end+4);
                let header=String::from_utf8_lossy(&buf[..end]);
                needed=header.lines().find_map(|s| {
                    let (key,val)=s.split_once(':')?;
                    if key.eq_ignore_ascii_case("content-length"){val.trim().parse::<usize>().ok()} else {None}
                }).unwrap_or(0);
                if needed>2048 {write_response(stream,"413 Payload Too Large","application/json","{\"error\":\"request_too_large\"}");return}
            }
        }
        if let Some(end)=header_end {
            if buf.len()>=end+needed {break}
        }
    }
    let Some(end)=header_end else {return};
    let header=String::from_utf8_lossy(&buf[..end]);
    let request_line=header.lines().next().unwrap_or("");
    // Browser-initiated cross-site requests must not control an unauthenticated
    // loopback server (local network access does not imply permission).
    let origin = header.lines().filter_map(|line| {
        let (key,value)=line.split_once(':')?;
        key.eq_ignore_ascii_case("origin").then_some(value.trim())
    }).next();
    if let Some(origin) = origin {
        if origin != "http://127.0.0.1:8787" && origin != "http://localhost:8787" {
            write_response(stream,"403 Forbidden","application/json","{\"error\":\"cross_origin_forbidden\"}");
            return;
        }
    }
    let body=std::str::from_utf8(&buf[end..]).unwrap_or("");
    let (status,content_type,result) = match request_line {
        "GET / HTTP/1.1" | "GET / HTTP/1.0" =>
            ("200 OK","text/html; charset=utf-8",html::INDEX.to_string()),
        "GET /health HTTP/1.1" | "GET /health HTTP/1.0" =>
            ("200 OK","application/json","{\"ok\":true}".to_owned()),
        "GET /status HTTP/1.1" | "GET /status HTTP/1.0" =>
            ("200 OK","application/json",status_json(&controller)),
        "GET /wifi/networks HTTP/1.1" | "GET /wifi/networks HTTP/1.0" =>
            match wifi::networks() {
                Ok(networks) => ("200 OK","application/json",
                    format!("{{\"networks\":[{}]}}", networks.iter()
                        .map(|s|format!("\"{}\"",escape_json(s))).collect::<Vec<_>>().join(","))),
                Err(error) => ("503 Service Unavailable","application/json",
                    format!("{{\"error\":\"{}\"}}",escape_json(&error)))
            },
        "POST /wifi/connect HTTP/1.1" | "POST /wifi/connect HTTP/1.0" => {
            let ssid=form_field(body,"ssid");
            let password=form_field(body,"password");
            match (ssid,password) {
                (Ok(Some(s)),Ok(Some(p))) => match wifi::connect(&s,&p) {
                    Ok(()) => ("200 OK","application/json","{\"connected\":true}".into()),
                    Err(e) => ("422 Unprocessable Content","application/json",format!("{{\"error\":\"{}\"}}",escape_json(&e)))
                },
                _ => ("400 Bad Request","application/json","{\"error\":\"invalid_credentials\"}".into())
            }
        }
        "POST /stream/start HTTP/1.1" | "POST /stream/start HTTP/1.0" =>
            match controller.start() {
                Ok(()) => ("202 Accepted","application/json","{\"streaming_requested\":true,\"youtube_ingest_confirmed\":false}".into()),
                Err(e) => ("503 Service Unavailable","application/json",format!("{{\"error\":\"{}\"}}",escape_json(&e)))
            },
        "POST /stream/stop HTTP/1.1" | "POST /stream/stop HTTP/1.0" => {
            controller.stop();
            ("200 OK","application/json","{\"streaming_requested\":false}".into())
        }
        _ => ("404 Not Found","application/json","{\"error\":\"not_found\"}".into())
    };
    write_response(stream,status,content_type,&result);
}

fn main() -> std::io::Result<()> {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(error) => {
            eprintln!("T3Box configuration error: {error}");
            std::process::exit(2);
        }
    };
    // Loopback-only by design; the control API has no authentication yet.
    let controller = Arc::new(Controller::new(config));
    start_engine_ipc(&controller);
    start_phone_ingest();
    let listener = TcpListener::bind("127.0.0.1:8787")?;
    println!("T3Box local dashboard: http://127.0.0.1:8787");
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let controller = Arc::clone(&controller);
                std::thread::spawn(move || handle(stream, controller));
            }
            Err(error) => eprintln!("accept error: {error}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escapes_json() {assert_eq!(escape_json("a\"b\\c"),"a\\\"b\\\\c");}
    #[test]
    fn url_form_roundtrip() {
        assert_eq!(url_decode("Hello+%E6%97%A5%E6%9C%AC").unwrap(),"Hello 日本");
        assert_eq!(form_field("ssid=LT+Event&password=p%40ssw0rd","ssid").unwrap().unwrap(),"LT Event");
        assert!(url_decode("%").is_err());
    }
}
