//! Stephania maintenance service (docs/MAINTENANCE.md).
//!
//! Owns the SSH user and host CAs and the maintenance code secret, opens
//! and closes maintenance windows, signs short-lived certificates, and
//! starts `stephania-sshd.service` only while a window is open. Every
//! decision is written to the journal (stderr) as one JSON line per event.
//!
//! Socket `STEPHANIA_MAINT_SOCKET` (default `/run/stephania-maint/maint.sock`,
//! docs/IPC.md framing), for the control plane only:
//! `window.open {actor, client_addr, code, root?}` → window;
//! `cert.sign {window_id, public_key, client_addr}` → `{certificate, …}`;
//! `window.close {window_id?, actor}`; `status {}`.
#[cfg(unix)]
fn main() {
    if let Err(e) = unix::run() {
        eprintln!("stephania-maint: {e}");
        std::process::exit(2);
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("stephania-maint requires a Unix platform");
    std::process::exit(2);
}

#[cfg(unix)]
mod unix {
    use serde_json::{json, Map, Value};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use stephania::ipc::{self, PeerPolicy};
    use stephania::maint::{MaintError, Maintenance};
    use stephania::pki::Endpoint;

    struct Config {
        socket: PathBuf,
        pki_dir: PathBuf,
        systemctl: String,
        sshd_unit: String,
    }

    fn env_or(key: &str, default: &str) -> String {
        std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.into())
    }

    fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

    pub fn run() -> Result<(), String> {
        let dir = PathBuf::from(env_or("STEPHANIA_MAINT_DIR", "/var/lib/stephania/maint"));
        let config = Config {
            socket: PathBuf::from(env_or("STEPHANIA_MAINT_SOCKET", "/run/stephania-maint/maint.sock")),
            pki_dir: PathBuf::from(env_or("STEPHANIA_PKI_DIR", "/var/lib/stephania/pki")),
            systemctl: env_or("STEPHANIA_MAINT_SYSTEMCTL", "systemctl"),
            sshd_unit: env_or("STEPHANIA_MAINT_SSHD_UNIT", "stephania-sshd.service"),
        };
        let policy = PeerPolicy::from_env()?;
        let maint = Maintenance::open(&dir)?;
        let state = Arc::new(Mutex::new(maint));
        let config = Arc::new(config);
        // A restart forgets any window, so sshd must not outlive it.
        sshd(&config, "stop");
        let ticker = (Arc::clone(&state), Arc::clone(&config));
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(5));
            let mut m = ticker.0.lock().unwrap_or_else(|p| p.into_inner());
            if m.expire(now()).is_some() { sshd(&ticker.1, "stop"); }
            flush_audit(&mut m);
        });
        serve(&config, &policy, &state)
    }

    fn sshd(config: &Config, verb: &str) -> bool {
        match std::process::Command::new(&config.systemctl).arg(verb).arg(&config.sshd_unit).status() {
            Ok(s) if s.success() => true,
            Ok(s) => { log("maintenance.sshd_failed", json!({"verb": verb, "status": s.code()})); false }
            Err(e) => { log("maintenance.sshd_failed", json!({"verb": verb, "error": e.to_string()})); false }
        }
    }

    fn log(kind: &str, fields: Value) {
        let mut line = Map::new();
        line.insert("event".into(), json!(kind));
        if let Value::Object(f) = fields { line.extend(f); }
        eprintln!("{}", Value::Object(line));
    }

    fn flush_audit(m: &mut Maintenance) {
        for e in m.audit.drain(..) { log(e.kind, e.fields); }
    }

    /// Names the host certificate covers: the mDNS name and current addresses.
    fn host_names(pki_dir: &Path) -> Result<Vec<String>, String> {
        let endpoint = Endpoint::load(pki_dir).ok_or("device PKI has not published an endpoint yet")?;
        let mut names = vec![endpoint.fqdn()];
        names.extend(endpoint.ips.iter().map(|ip| ip.to_string()));
        Ok(names)
    }

    fn str_param<'a>(p: Option<&'a Map<String, Value>>, k: &str) -> Result<&'a str, MaintError> {
        p.and_then(|p| p.get(k)).and_then(Value::as_str).ok_or_else(|| MaintError::InvalidParams(format!("{k} required")))
    }

    fn dispatch(config: &Config, m: &mut Maintenance, method: &str, params: Option<&Map<String, Value>>) -> Result<Value, MaintError> {
        let t = now();
        match method {
            "status" => Ok(m.status(t)),
            "window.open" => {
                let root = params.and_then(|p| p.get("root")).and_then(Value::as_bool).unwrap_or(false);
                let w = m.open_window(str_param(params, "actor")?, str_param(params, "client_addr")?, str_param(params, "code")?, root, t)?;
                let started = host_names(&config.pki_dir).and_then(|names| m.write_host_certificate(&names, t)).map_err(MaintError::Internal)
                    .and_then(|_| if sshd(config, "start") { Ok(()) } else { Err(MaintError::Internal("could not start sshd".into())) });
                if let Err(e) = started {
                    let _ = m.close(Some(&w.id), "stephania-maint", "start_failed");
                    sshd(config, "stop");
                    return Err(e);
                }
                Ok(w.to_json(t))
            }
            "cert.sign" => m.sign(str_param(params, "window_id")?, str_param(params, "public_key")?, str_param(params, "client_addr")?, t),
            "window.close" => {
                let id = params.and_then(|p| p.get("window_id")).and_then(Value::as_str);
                let actor = str_param(params, "actor")?;
                let closed = m.close(id, actor, "closed")?;
                if closed.is_some() { sshd(config, "stop"); }
                Ok(json!({"closed": closed.is_some()}))
            }
            _ => Err(MaintError::InvalidParams("unknown method".into())),
        }
    }

    fn handle(config: &Config, state: &Mutex<Maintenance>, frame: &[u8]) -> Value {
        let err = |id: Option<&str>, code: &str, msg: &str, extra: Value| {
            let mut e = json!({"code": code, "message": msg});
            if let (Value::Object(o), Value::Object(x)) = (&mut e, extra) { o.extend(x); }
            json!({"version": 1, "request_id": id, "ok": false, "error": e})
        };
        let Ok(Value::Object(req)) = serde_json::from_slice::<Value>(frame) else { return err(None, "invalid_request", "not a JSON object", json!({})) };
        let Some(id) = req.get("request_id").and_then(Value::as_str).filter(|s| (1..=64).contains(&s.len())) else {
            return err(None, "invalid_request", "invalid request_id", json!({}));
        };
        if req.get("version") != Some(&json!(1)) { return err(Some(id), "unsupported_version", "version must be 1", json!({})); }
        let Some(method) = req.get("method").and_then(Value::as_str) else { return err(Some(id), "invalid_request", "method required", json!({})) };
        let mut m = state.lock().unwrap_or_else(|p| p.into_inner());
        let result = dispatch(config, &mut m, method, req.get("params").and_then(Value::as_object));
        flush_audit(&mut m);
        match result {
            Ok(v) => json!({"version": 1, "request_id": id, "ok": true, "result": v}),
            Err(e) => {
                let extra = match e { MaintError::Throttled { retry_after } => json!({"retry_after": retry_after}), _ => json!({}) };
                err(Some(id), e.code(), &e.message(), extra)
            }
        }
    }

    /// One connection at a time: requests are rare and the state is single-owner.
    fn serve(config: &Config, policy: &PeerPolicy, state: &Mutex<Maintenance>) -> Result<(), String> {
        let listener = ipc::bind(&config.socket).map_err(|e| format!("bind {}: {e}", config.socket.display()))?;
        log("maintenance.listening", json!({"socket": config.socket.display().to_string()}));
        // SAFETY: getuid has no preconditions and cannot fail.
        let own = unsafe { libc::getuid() };
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let _ = stream.set_read_timeout(Some(ipc::IO_TIMEOUT));
            let _ = stream.set_write_timeout(Some(ipc::IO_TIMEOUT));
            let permitted = match ipc::peer_uid(&stream) { Some(uid) => policy.allows(uid, own), None => !cfg!(target_os = "linux") };
            if !permitted {
                let _ = ipc::write_frame(&mut stream, br#"{"version":1,"request_id":null,"ok":false,"error":{"code":"forbidden","message":"peer not permitted"}}"#);
                continue;
            }
            while let Ok(Some(frame)) = ipc::read_frame(&mut stream) {
                let body = serde_json::to_vec(&handle(config, state, &frame)).unwrap_or_default();
                if ipc::write_frame(&mut stream, &body).is_err() { break; }
            }
        }
        Ok(())
    }
}
