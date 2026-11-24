//! T3Box Pi 5 stream control, diagnostics and loopback-only configuration UI.
mod stream;
mod wifi;
mod html;

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
