//! Stephania maintenance service (docs/MAINTENANCE.md).
//!
//! Owns the SSH user and host CAs and the maintenance code secret, opens
//! and closes maintenance windows, signs short-lived certificates, and
//! starts `stephania-sshd.service` only while a window is open. Every
//! decision is written to the journal (stderr) as one JSON line per event.
//!
//! Socket `STEPHANIA_MAINT_SOCKET` (default `/run/stephania-maint/maint.sock`,
//! docs/IPC.md framing). The control plane (`STEPHANIA_IPC_ALLOWED_UIDS`) may call
//! `window.open {actor, client_addr, code, root?}` → window;
//! `cert.sign {window_id, public_key, actor, client_addr}` → `{certificate, …}`
//! (only for the window's opener, from the address it was opened from);
//! `window.close {window_id?, actor, opener?}` (with `opener`, only a
//! window that account opened); `status {}`.
//! The provisioning core (`STEPHANIA_MAINT_ENROLL_UIDS`) may call only
//! `otp.rotate {}` → `{otp_secret}` (hex), for `management.enroll`.
//! Root and the service's own uid may call everything.
//!
//! A second socket, `STEPHANIA_MAINT_OPS_SOCKET` (default `ops.sock` next
//! to the main socket, group `STEPHANIA_MAINT_OPS_GID`), is the
//! broker for `steph`: the maintenance login (`STEPHANIA_MAINT_OPS_UIDS`)
//! and root may call `ops.status {}`, `ops.restart {unit, actor, force}`
//! (short names, allowlist), `ops.reboot {actor, force}`,
//! `ops.update_apply {path, actor, force}` (a bundle in the inbox) and
//! `ops.window_close {actor}`, only while a window is open. Changes that
//! would interrupt a stream are refused while the engine streams unless
//! `force` is set. The `actor` steph sends is only a claim: the audit
//! names the open window's account and id and the caller's uid, and keeps
//! the claim as `claimed_actor`.
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
    use std::sync::atomic::{AtomicUsize, Ordering};
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
        enroll_uids: Vec<u32>,
        ops_socket: Option<PathBuf>,
        ops_gid: Option<u32>,
        ops_uids: Vec<u32>,
        engine_socket: PathBuf,
        update: String,
        inbox: PathBuf,
    }

    /// What a peer may call.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Role { Full, Control, Enroll, Ops }

    const OPS_METHODS: [&str; 5] = ["ops.status", "ops.restart", "ops.reboot", "ops.update_apply", "ops.window_close"];

    /// steph's allowlist (docs/MAINTENANCE.md "What steph may change"):
    /// short name → unit, and whether restarting it interrupts a stream.
    const RESTARTABLE: [(&str, &str, bool); 6] = [
        ("engine", "stephania.service", true),
        ("phone", "stephania-phone.service", true),
        ("control", "stephania-control.service", false),
        ("ble", "stephania-ble.service", false),
        ("provision", "stephania-provision.service", false),
        ("pki", "stephania-pki.service", false),
    ];

    const CONTROL_METHODS: [&str; 4] = ["status", "window.open", "cert.sign", "window.close"];
    const ENROLL_METHODS: [&str; 1] = ["otp.rotate"];
    const MAX_CONNECTIONS: usize = 4;

    fn uid_list(key: &str) -> Result<Vec<u32>, String> {
        std::env::var(key).unwrap_or_default().split(',').map(str::trim).filter(|p| !p.is_empty())
            .map(|p| p.parse::<u32>().map_err(|_| format!("invalid uid in {key}: {p}"))).collect()
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
            enroll_uids: uid_list("STEPHANIA_MAINT_ENROLL_UIDS")?,
            // Next to maint.sock unless set; "none" turns the broker off.
            ops_socket: match std::env::var("STEPHANIA_MAINT_OPS_SOCKET").ok().filter(|v| !v.is_empty()) {
                Some(v) if v == "none" => None,
                Some(v) => Some(PathBuf::from(v)),
                None => Some(PathBuf::from(env_or("STEPHANIA_MAINT_SOCKET", "/run/stephania-maint/maint.sock")).with_file_name("ops.sock")),
            },
            ops_gid: std::env::var("STEPHANIA_MAINT_OPS_GID").ok().filter(|v| !v.is_empty())
                .map(|v| v.parse::<u32>().map_err(|_| format!("invalid STEPHANIA_MAINT_OPS_GID: {v}"))).transpose()?,
            ops_uids: uid_list("STEPHANIA_MAINT_OPS_UIDS")?,
            engine_socket: PathBuf::from(env_or("STEPHANIA_ENGINE_SOCKET", "/run/stephania/engine.sock")),
            update: env_or("STEPHANIA_MAINT_UPDATE", "/usr/local/bin/stephania-update"),
            inbox: PathBuf::from(env_or("STEPHANIA_MAINT_INBOX", "/var/lib/stephania/inbox")),
        };
        let policy = PeerPolicy::from_env()?;
        let maint = Maintenance::open(&dir)?;
        let state = Arc::new(Mutex::new(maint));
        let config = Arc::new(config);
        // A restart forgets any window, so sshd must not outlive it.
        window_ended(&config);
        let ticker = (Arc::clone(&state), Arc::clone(&config));
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(5));
            let mut m = ticker.0.lock().unwrap_or_else(|p| p.into_inner());
            if m.expire(now()).is_some() { window_ended(&ticker.1); }
            flush_audit(&mut m);
        });
        if let Some(ops) = config.ops_socket.clone() {
            let listener = ipc::bind(&ops).map_err(|e| format!("bind {}: {e}", ops.display()))?;
            if let Some(gid) = config.ops_gid {
                let c = std::ffi::CString::new(ops.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
                // SAFETY: chown with a NUL-terminated path; -1 keeps the owner.
                if unsafe { libc::chown(c.as_ptr(), u32::MAX, gid) } != 0 {
                    return Err(format!("chown {}: {}", ops.display(), std::io::Error::last_os_error()));
                }
            }
            let (config, state) = (Arc::clone(&config), Arc::clone(&state));
            std::thread::spawn(move || serve_on(listener, &config, None, &state, |uid, c| c.ops_uids.contains(&uid).then_some(Role::Ops)));
        }
        let listener = ipc::bind(&config.socket).map_err(|e| format!("bind {}: {e}", config.socket.display()))?;
        log("maintenance.listening", json!({"socket": config.socket.display().to_string()}));
        serve_on(listener, &config, Some(&policy), &state, |uid, c| {
            if c.enroll_uids.contains(&uid) { Some(Role::Enroll) } else { None }
        });
        Ok(())
    }

    /// After any window ends: stop sshd and empty the upload inbox.
    fn window_ended(config: &Config) {
        sshd(config, "stop");
        if let Ok(entries) = std::fs::read_dir(&config.inbox) {
            for e in entries.flatten() {
                let p = e.path();
                let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            }
        }
    }

    /// The bundle path must be a regular file directly in the inbox, where
    /// `stephctl cp <file> device:` puts uploads.
    fn update_target(config: &Config, m: &mut Maintenance, params: Option<&Map<String, Value>>, uid: Option<u32>) -> Result<(PathBuf, String), MaintError> {
        let (actor, identity) = ops_actor(m, params, uid)?;
        let force = params.and_then(|p| p.get("force")).and_then(Value::as_bool).unwrap_or(false);
        let given = PathBuf::from(str_param(params, "path")?);
        let inbox = config.inbox.canonicalize().map_err(|e| MaintError::Internal(format!("inbox: {e}")))?;
        let path = given.canonicalize().map_err(|_| MaintError::InvalidParams("no such bundle".into()))?;
        let meta = std::fs::symlink_metadata(&path).map_err(|_| MaintError::InvalidParams("no such bundle".into()))?;
        if path.parent() != Some(inbox.as_path()) || !meta.is_file() {
            return Err(MaintError::InvalidParams(format!("the bundle must be a file in {} (stephctl cp <file> device:)", inbox.display())));
        }
        if !force && on_air(config) {
            m.audit.push(stephania::maint::AuditEvent { kind: "maintenance.ops_refused", fields: with_identity(&identity, json!({"op": "ops.update_apply", "reason": "on_air"})) });
            return Err(MaintError::OnAir);
        }
        m.audit.push(stephania::maint::AuditEvent { kind: "maintenance.ops_update_apply", fields: with_identity(&identity, json!({"bundle": path.display().to_string(), "force": force})) });
        Ok((path, actor))
    }

    /// `stephania-update <bundle>`: RAUC installs into the other slot, then
    /// the device reboots into it once (tryboot). RAUC's message on refusal.
    fn run_update(config: &Config, path: &Path, actor: &str) -> Result<Value, MaintError> {
        match std::process::Command::new(&config.update).arg(path).output() {
            Ok(o) if o.status.success() => Ok(json!({"installed": true, "rebooting": true})),
            Ok(o) => {
                let msg = String::from_utf8_lossy(&o.stderr).trim().to_string();
                log("maintenance.ops_update_failed", json!({"actor": actor, "status": o.status.code(), "message": msg}));
                Err(MaintError::Failed(if msg.is_empty() { "update failed".into() } else { msg }))
            }
            Err(e) => Err(MaintError::Internal(format!("{}: {e}", config.update))),
        }
    }

    fn on_air(config: &Config) -> bool {
        matches!(stephania::bootctl::engine_status(&config.engine_socket), Ok(st) if st.get("streaming_requested") == Some(&Value::Bool(true)))
    }

    fn systemctl(config: &Config, args: &[&str]) -> Result<(), MaintError> {
        match std::process::Command::new(&config.systemctl).args(args).output() {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(MaintError::Internal(format!("systemctl {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()))),
            Err(e) => Err(MaintError::Internal(format!("systemctl: {e}"))),
        }
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

    /// The device's own addresses (refused as clients), from the PKI endpoint.
    fn refresh_device_addrs(config: &Config, m: &mut Maintenance) {
        if let Some(e) = Endpoint::load(&config.pki_dir) { m.set_device_addrs(&e.ips); }
    }

    /// Who a broker change is audited as: the open window's account and id
    /// (`<account>@<window>`, what the session certificate's key ID says),
    /// the caller's uid, and what steph claimed, kept only if well formed.
    fn ops_actor(m: &Maintenance, params: Option<&Map<String, Value>>, uid: Option<u32>) -> Result<(String, Value), MaintError> {
        let claimed = str_param(params, "actor")?;
        let claimed = if (1..=128).contains(&claimed.len()) && claimed.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'@')) {
            json!(claimed)
        } else {
            json!("(malformed)")
        };
        let actor = m.window().map(|w| format!("{}@{}", w.actor, w.id)).unwrap_or_else(|| "(no window)".into());
        Ok((actor.clone(), json!({"actor": actor, "claimed_actor": claimed, "peer_uid": uid})))
    }

    /// Adds the audit identity to an event's fields.
    fn with_identity(identity: &Value, fields: Value) -> Value {
        let mut out = identity.as_object().cloned().unwrap_or_default();
        if let Value::Object(f) = fields { out.extend(f); }
        Value::Object(out)
    }

    fn dispatch(config: &Config, m: &mut Maintenance, method: &str, params: Option<&Map<String, Value>>, uid: Option<u32>) -> Result<Value, MaintError> {
        let t = now();
        match method {
            "otp.rotate" => {
                let (secret, closed) = m.rotate_otp_secret().map_err(MaintError::Internal)?;
                if closed.is_some() { window_ended(config); }
                Ok(json!({"otp_secret": secret.iter().map(|b| format!("{b:02x}")).collect::<String>()}))
            }
            "status" => Ok(m.status(t)),
            "window.open" => {
                let root = params.and_then(|p| p.get("root")).and_then(Value::as_bool).unwrap_or(false);
                refresh_device_addrs(config, m);
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
            "cert.sign" => {
                refresh_device_addrs(config, m);
                m.sign(str_param(params, "window_id")?, str_param(params, "public_key")?, str_param(params, "actor")?, str_param(params, "client_addr")?, t)
            }
            "window.close" => {
                let id = params.and_then(|p| p.get("window_id")).and_then(Value::as_str);
                let actor = str_param(params, "actor")?;
                let closed = match params.and_then(|p| p.get("opener")).and_then(Value::as_str) {
                    Some(opener) => m.close_opened_by(opener, actor, "account_changed"),
                    None => m.close(id, actor, "closed")?,
                };
                if closed.is_some() { window_ended(config); }
                Ok(json!({"closed": closed.is_some()}))
            }
            "ops.status" => {
                let mut st = m.status(t);
                if let Value::Object(o) = &mut st { o.insert("on_air".into(), json!(on_air(config))); }
                Ok(st)
            }
            "ops.window_close" => {
                let (actor, _) = ops_actor(m, params, uid)?;
                let closed = m.close(None, &actor, "closed_by_steph")?;
                if closed.is_some() { window_ended(config); }
                Ok(json!({"closed": closed.is_some()}))
            }
            "ops.restart" | "ops.reboot" => {
                let (_, identity) = ops_actor(m, params, uid)?;
                let force = params.and_then(|p| p.get("force")).and_then(Value::as_bool).unwrap_or(false);
                let (target, live) = if method == "ops.reboot" { ("reboot", true) } else {
                    let unit = str_param(params, "unit")?;
                    let Some((_, u, live)) = RESTARTABLE.iter().find(|(n, _, _)| *n == unit) else {
                        m.audit.push(stephania::maint::AuditEvent { kind: "maintenance.ops_refused", fields: with_identity(&identity, json!({"op": method, "unit": unit, "reason": "not_allowed"})) });
                        return Err(MaintError::NotAllowed(unit.to_string()));
                    };
                    (*u, *live)
                };
                if live && !force && on_air(config) {
                    m.audit.push(stephania::maint::AuditEvent { kind: "maintenance.ops_refused", fields: with_identity(&identity, json!({"op": method, "target": target, "reason": "on_air"})) });
                    return Err(MaintError::OnAir);
                }
                m.audit.push(stephania::maint::AuditEvent { kind: if method == "ops.reboot" { "maintenance.ops_reboot" } else { "maintenance.ops_restart" },
                    fields: with_identity(&identity, json!({"target": target, "force": force})) });
                flush_audit(m);
                let r = if method == "ops.reboot" { systemctl(config, &["reboot"]) } else { systemctl(config, &["restart", target]) };
                r.map_err(|e| MaintError::Failed(e.message()))?;
                Ok(json!({"done": true, "target": target}))
            }
            _ => Err(MaintError::InvalidParams("unknown method".into())),
        }
    }

    fn handle(config: &Config, state: &Mutex<Maintenance>, role: Role, uid: Option<u32>, frame: &[u8]) -> Value {
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
        let permitted = match role {
            Role::Full => true,
            Role::Control => CONTROL_METHODS.contains(&method),
            Role::Enroll => ENROLL_METHODS.contains(&method),
            Role::Ops => OPS_METHODS.contains(&method),
        };
        if !permitted { return err(Some(id), "forbidden", "method not permitted for this peer", json!({})) }
        let params = req.get("params").and_then(Value::as_object);
        let mut m = state.lock().unwrap_or_else(|p| p.into_inner());
        let result = if method.starts_with("ops.") && method != "ops.status" && m.window().is_none() {
            Err(MaintError::WindowClosed)
        } else if method == "ops.update_apply" {
            // Checked under the lock, installed without it: RAUC takes minutes.
            let checked = update_target(config, &mut m, params, uid);
            flush_audit(&mut m);
            drop(m);
            let result = checked.and_then(|(path, actor)| run_update(config, &path, &actor));
            m = state.lock().unwrap_or_else(|p| p.into_inner());
            result
        } else {
            dispatch(config, &mut m, method, params, uid)
        };
        flush_audit(&mut m);
        match result {
            Ok(v) => json!({"version": 1, "request_id": id, "ok": true, "result": v}),
            Err(e) => {
                let extra = match e { MaintError::Throttled { retry_after } => json!({"retry_after": retry_after}), _ => json!({}) };
                err(Some(id), e.code(), &e.message(), extra)
            }
        }
    }

    /// A thread per connection (at most MAX_CONNECTIONS per socket): the
    /// control plane keeps pooled connections open, which must not shut out
    /// the provisioning core. Requests are serialized by the state lock.
    /// Root and the service itself get every method; `extra` names other
    /// roles by uid; `policy` (main socket only) admits the control plane.
    fn serve_on(listener: std::os::unix::net::UnixListener, config: &Arc<Config>, policy: Option<&PeerPolicy>,
                state: &Arc<Mutex<Maintenance>>, extra: impl Fn(u32, &Config) -> Option<Role>) {
        // SAFETY: getuid has no preconditions and cannot fail.
        let own = unsafe { libc::getuid() };
        let active = Arc::new(AtomicUsize::new(0));
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let _ = stream.set_read_timeout(Some(ipc::IO_TIMEOUT));
            let _ = stream.set_write_timeout(Some(ipc::IO_TIMEOUT));
            // Fail closed on Linux if credentials cannot be read; elsewhere
            // (development) the socket's file mode is the only guard.
            let peer = ipc::peer_uid(&stream);
            let role = match peer {
                Some(uid) if uid == 0 || uid == own => Some(Role::Full),
                Some(uid) => extra(uid, config).or_else(|| policy.filter(|p| p.allows(uid, own)).map(|_| Role::Control)),
                None if cfg!(target_os = "linux") => None,
                None => Some(Role::Full),
            };
            let Some(role) = role else {
                let _ = ipc::write_frame(&mut stream, br#"{"version":1,"request_id":null,"ok":false,"error":{"code":"forbidden","message":"peer not permitted"}}"#);
                continue;
            };
            if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                active.fetch_sub(1, Ordering::SeqCst);
                let _ = ipc::write_frame(&mut stream, br#"{"version":1,"request_id":null,"ok":false,"error":{"code":"busy","message":"too many connections"}}"#);
                continue;
            }
            let (config, state, active) = (Arc::clone(config), Arc::clone(state), Arc::clone(&active));
            std::thread::spawn(move || {
                while let Ok(Some(frame)) = ipc::read_frame(&mut stream) {
                    let body = serde_json::to_vec(&handle(&config, &state, role, peer, &frame)).unwrap_or_default();
                    if ipc::write_frame(&mut stream, &body).is_err() { break; }
                }
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }
}
