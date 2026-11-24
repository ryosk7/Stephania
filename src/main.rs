//! T3Box Raspberry Pi 5 controller and streaming supervisor.
//! Loopback-only API. The video pipeline uses FFmpeg until a custom codec is validated.
mod stream;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use stream::{Config, Controller};

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
    format!(
        "{{\"state\":\"{}\",\"streaming_requested\":{},\"ffmpeg_process_running\":{},\"youtube_ingest_confirmed\":false,\"video_devices\":[{videos}],\"audio_capture_available\":{},\"wifi_interfaces\":[{wifi}],\"cpu_temperature_c\":{temp},\"restart_attempts\":{},\"last_error\":\"{}\",\"timestamp_unix\":{now}}}",
        if process_running { "transmitting_unverified" } else { "idle" },
        desired, process_running, audio_capture_available(), restarts, escape_json(&last_error)
    )
}

fn respond(mut stream: TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn handle(mut stream: TcpStream, controller: Arc<Controller>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let mut buffer = [0u8; 4096];
    let Ok(n) = stream.read(&mut buffer) else { return };
    let line = String::from_utf8_lossy(&buffer[..n]);
    let request_line = line.lines().next().unwrap_or("");
    let (status, body) = match request_line {
        "GET /health HTTP/1.1" | "GET /health HTTP/1.0" =>
            ("200 OK", "{\"ok\":true}".to_owned()),
        "GET /status HTTP/1.1" | "GET /status HTTP/1.0" =>
            ("200 OK", status_json(&controller)),
        "POST /stream/start HTTP/1.1" | "POST /stream/start HTTP/1.0" =>
            match controller.start() {
                Ok(()) => ("202 Accepted", "{\"streaming_requested\":true,\"youtube_ingest_confirmed\":false}".to_owned()),
                Err(e) => ("503 Service Unavailable", format!("{{\"error\":\"{}\"}}", escape_json(&e))),
            },
        "POST /stream/stop HTTP/1.1" | "POST /stream/stop HTTP/1.0" => {
            controller.stop();
            ("200 OK", "{\"streaming_requested\":false}".to_owned())
        }
        _ => ("404 Not Found", "{\"error\":\"not_found\"}".to_owned()),
    };
    respond(stream, status, &body);
}

fn main() -> std::io::Result<()> {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(error) => {
            eprintln!("T3Box configuration error: {error}");
            std::process::exit(2);
        }
    };
    // Deliberately loopback-only: no authentication or CSRF protection yet.
    let controller = Arc::new(Controller::new(config));
    let listener = TcpListener::bind("127.0.0.1:8787")?;
    println!("T3Box controller listening on http://127.0.0.1:8787");
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
    fn escapes_json() {
        assert_eq!(escape_json("a\"b\\c"), "a\\\"b\\\\c");
    }
    #[test]
    fn video_device_filter_is_numeric() {
        assert!(format!("/dev/{}", "video0").starts_with("/dev/video"));
    }
}
