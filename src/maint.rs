//! Maintenance windows and short-lived SSH certificates
//! (docs/MAINTENANCE.md). The `stephania-maint` service owns this state;
//! the control plane never sees the CA keys or the code secret.
//!
//! Files in `dir` (0700, owned by stephania-maint):
//! - `state.json` (0600): CA and host key seeds, the code secret, the
//!   certificate serial counter and the last code step used;
//! - `user_ca.pub`, `host_ca.pub`: the CAs' public keys (sshd's
//!   `TrustedUserCAKeys`; what clients pin for host certificates);
//! - `ssh_host_ed25519_key` (0600), `.pub` and `-cert.pub`: sshd's host key
//!   and its certificate, reissued for the current names on every window;
//! - `revoked.krl` (sshd's `RevokedKeys`): every user certificate issued
//!   before the current window, so a certificate never works in a later
//!   window, and all of them once the window has closed.
//!
//! Time is passed in (`now`, Unix seconds) so the rules are testable.
use crate::sshcert::{self, CertSpec, Key, HOST_CERT, USER_CERT};
use crate::totp;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{json, Value};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// Time to start sessions after a window opens.
pub const WINDOW_SECONDS: u64 = 10 * 60;
pub const ROOT_WINDOW_SECONDS: u64 = 5 * 60;
/// Absolute end of sessions started in a window.
pub const SESSION_SECONDS: u64 = 60 * 60;
pub const ROOT_SESSION_SECONDS: u64 = 30 * 60;
/// User certificate lifetime (and back-dating for clock skew).
pub const CERT_SECONDS: u64 = 5 * 60;
pub const CERT_BACKDATE: u64 = 30;
/// Host certificates cover one day; a new one is written per window.
pub const HOST_CERT_SECONDS: u64 = 24 * 3600;
/// Wrong codes allowed before code entry is locked.
pub const CODE_ATTEMPTS: u32 = 5;
const LOCK_FIRST: u64 = 60;
const LOCK_MAX: u64 = 15 * 60;

pub const PRINCIPAL: &str = "maint";
pub const ROOT_PRINCIPAL: &str = "maint-root";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaintError {
    InvalidCode,
    Throttled { retry_after: u64 },
    WindowOpen,
    WindowClosed,
    /// cert.sign from another account or address than the window's opener.
    NotOpener,
    InvalidParams(String),
    Internal(String),
    /// steph asked for a unit outside its allowlist.
    NotAllowed(String),
    /// The change would interrupt the stream and was not forced.
    OnAir,
    /// A command failed (systemctl, RAUC); the message says why.
    Failed(String),
}

impl MaintError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidCode => "invalid_code",
            Self::Throttled { .. } => "code_throttled",
            Self::WindowOpen => "window_open",
            Self::WindowClosed => "window_closed",
            Self::NotOpener => "not_window_opener",
            Self::InvalidParams(_) => "invalid_params",
            Self::Internal(_) => "internal",
            Self::NotAllowed(_) => "not_allowed",
            Self::OnAir => "on_air",
            Self::Failed(_) => "failed",
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::InvalidCode => "code not accepted".into(),
            Self::Throttled { retry_after } => format!("too many wrong codes, retry in {retry_after} s"),
            Self::WindowOpen => "a maintenance window is already open".into(),
            Self::WindowClosed => "no open maintenance window with that id".into(),
            Self::NotOpener => "only the account that opened the window, from the same address, gets certificates for it".into(),
            Self::InvalidParams(m) | Self::Internal(m) | Self::Failed(m) => m.clone(),
            Self::NotAllowed(u) => format!("steph cannot restart {u}"),
            Self::OnAir => "on air: this would interrupt the stream".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: String,
    pub root: bool,
    pub actor: String,
    /// The address the window was opened from; certificates are only
    /// issued to the same account from the same address.
    pub client: IpAddr,
    pub opened_at: u64,
    /// No certificates after this.
    pub expires_at: u64,
    /// sshd is stopped (ending sessions) at this time.
    pub sessions_end_at: u64,
}

impl Window {
    pub fn principal(&self) -> &'static str { if self.root { ROOT_PRINCIPAL } else { PRINCIPAL } }
    pub fn user(&self) -> &'static str { if self.root { "root" } else { PRINCIPAL } }
    pub fn to_json(&self, now: u64) -> Value {
        json!({
            "window_id": self.id, "root": self.root, "actor": self.actor, "client_addr": self.client.to_string(), "user": self.user(),
            "opened_at": self.opened_at, "expires_at": self.expires_at, "sessions_end_at": self.sessions_end_at,
            "accepting": now < self.expires_at,
        })
    }
}

/// Persistent secrets and counters (`state.json`).
struct Secrets {
    user_ca: Key,
    host_ca: Key,
    host_key: Key,
    otp_secret: Vec<u8>,
    serial: u64,
    /// Code steps already used, oldest first (`totp::verify`).
    used_code_steps: Vec<u64>,
}

impl Secrets {
    fn generate() -> Result<Self, String> {
        let mut otp_secret = vec![0u8; 20];
        SystemRandom::new().fill(&mut otp_secret).map_err(|_| "random source failed".to_string())?;
        Ok(Self { user_ca: Key::generate()?, host_ca: Key::generate()?, host_key: Key::generate()?, otp_secret, serial: 0, used_code_steps: Vec::new() })
    }

    fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "user_ca_seed": hex(self.user_ca.seed()), "host_ca_seed": hex(self.host_ca.seed()),
            "host_key_seed": hex(self.host_key.seed()), "otp_secret": hex(&self.otp_secret),
            "serial": self.serial, "used_code_steps": self.used_code_steps,
            // Read by earlier versions.
            "last_code_step": self.used_code_steps.last(),
        })
    }

    fn from_json(v: &Value) -> Option<Self> {
        let seed = |k: &str| -> Option<Key> { Key::from_seed(unhex(v.get(k)?.as_str()?)?.try_into().ok()?).ok() };
        Some(Self {
            user_ca: seed("user_ca_seed")?,
            host_ca: seed("host_ca_seed")?,
            host_key: seed("host_key_seed")?,
            otp_secret: unhex(v.get("otp_secret")?.as_str()?).filter(|s| s.len() >= 16)?,
            serial: v.get("serial")?.as_u64()?,
            used_code_steps: match v.get("used_code_steps").and_then(Value::as_array) {
                Some(steps) => steps.iter().map(Value::as_u64).collect::<Option<Vec<_>>>()?,
                None => v.get("last_code_step").and_then(Value::as_u64).into_iter().collect(),
            },
        })
    }
}

/// Events for the audit log (the binary writes them to the journal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub kind: &'static str,
    pub fields: Value,
}

pub struct Maintenance {
    dir: PathBuf,
    secrets: Secrets,
    window: Option<Window>,
    /// The device's own addresses, which are never clients.
    device_addrs: Vec<IpAddr>,
    /// Serials up to this were issued before the current window.
    revoked_through: u64,
    failures: u32,
    locked_until: u64,
    lock_seconds: u64,
    pub audit: Vec<AuditEvent>,
}

impl Maintenance {
    /// Loads `dir/state.json`, creating secrets and public files on first use.
    /// A malformed state file is an error: it is never silently replaced,
    /// because that would change the CAs the owner has pinned.
    pub fn open(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let state = dir.join("state.json");
        let secrets = match std::fs::read(&state) {
            Ok(raw) => serde_json::from_slice::<Value>(&raw).ok().as_ref().and_then(Secrets::from_json)
                .ok_or_else(|| format!("{} is malformed; refusing to replace it", state.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Secrets::generate()?,
            Err(e) => return Err(format!("{}: {e}", state.display())),
        };
        let revoked_through = secrets.serial;
        let m = Self { dir: dir.to_path_buf(), secrets, window: None, device_addrs: Vec::new(), revoked_through, failures: 0, locked_until: 0, lock_seconds: LOCK_FIRST, audit: Vec::new() };
        m.save()?;
        // No window survives a restart, so no certificate issued so far may.
        m.write_revocations()?;
        write_file(&dir.join("user_ca.pub"), (m.secrets.user_ca.public_line("stephania-user-ca") + "\n").as_bytes(), 0o644)?;
        write_file(&dir.join("host_ca.pub"), (m.secrets.host_ca.public_line("stephania-host-ca") + "\n").as_bytes(), 0o644)?;
        write_file(&dir.join("ssh_host_ed25519_key"), m.secrets.host_key.private_pem("stephania-host")?.as_bytes(), 0o600)?;
        write_file(&dir.join("ssh_host_ed25519_key.pub"), (m.secrets.host_key.public_line("stephania-host") + "\n").as_bytes(), 0o644)?;
        Ok(m)
    }

    fn save(&self) -> Result<(), String> {
        write_file(&self.dir.join("state.json"), self.secrets.to_json().to_string().as_bytes(), 0o600)
    }

    /// Writes `revoked.krl` for every certificate up to `revoked_through`.
    /// sshd reads it on every authentication.
    fn write_revocations(&self) -> Result<(), String> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let krl = sshcert::revocation_list(&self.secrets.user_ca, Some((1, self.revoked_through)), self.revoked_through, now);
        write_file(&self.dir.join("revoked.krl"), &krl, 0o644)
    }

    /// Revokes everything issued so far (window opened or closed).
    fn revoke_all(&mut self) -> Result<(), String> {
        self.revoked_through = self.secrets.serial;
        self.write_revocations()
    }

    /// The device's own addresses (the PKI endpoint's), refused as clients:
    /// a request that claims one comes from the device itself.
    pub fn set_device_addrs(&mut self, addrs: &[IpAddr]) { self.device_addrs = addrs.iter().map(|ip| unmap(*ip)).collect(); }

    pub fn host_ca_line(&self) -> String { self.secrets.host_ca.public_line("stephania-host-ca") }
    pub fn otp_secret(&self) -> &[u8] { &self.secrets.otp_secret }

    /// Replaces the code secret (a new Connect enrollment, delivered sealed
    /// over BLE by stephania-provision). Codes from the old secret stop
    /// working at once; an open window is closed, since it was opened with
    /// a code the owner may no longer be able to vouch for.
    pub fn rotate_otp_secret(&mut self) -> Result<(Vec<u8>, Option<Window>), String> {
        let mut secret = vec![0u8; 20];
        SystemRandom::new().fill(&mut secret).map_err(|_| "random source failed".to_string())?;
        let previous = std::mem::replace(&mut self.secrets.otp_secret, secret.clone());
        let previous_steps = std::mem::take(&mut self.secrets.used_code_steps);
        if let Err(e) = self.save() {
            self.secrets.otp_secret = previous;
            self.secrets.used_code_steps = previous_steps;
            return Err(e);
        }
        self.audit("maintenance.code_secret_rotated", json!({}));
        let closed = self.close(None, "stephania-maint", "code_secret_rotated").ok().flatten();
        Ok((secret, closed))
    }
    pub fn window(&self) -> Option<&Window> { self.window.as_ref() }

    pub fn status(&self, now: u64) -> Value {
        json!({
            "open": self.window.is_some(),
            "window": self.window.as_ref().map(|w| w.to_json(now)),
            "code_locked_until": (self.locked_until > now).then_some(self.locked_until),
        })
    }

    /// Checks the code and opens a window. The caller starts sshd after
    /// writing the host certificate (`write_host_certificate`).
    pub fn open_window(&mut self, actor: &str, client_addr: &str, code: &str, root: bool, now: u64) -> Result<Window, MaintError> {
        if !valid_actor(actor) { return Err(MaintError::InvalidParams("invalid actor".into())); }
        let client = self.client_addr(client_addr)?;
        if self.window.is_some() { return Err(MaintError::WindowOpen); }
        if now < self.locked_until { return Err(MaintError::Throttled { retry_after: self.locked_until - now }); }
        let Some(step) = totp::verify(&self.secrets.otp_secret, code, now, &self.secrets.used_code_steps) else {
            self.failures += 1;
            self.audit("maintenance.code_rejected", json!({"actor": actor, "client_addr": client.to_string(), "root": root, "failures": self.failures}));
            if self.failures >= CODE_ATTEMPTS {
                self.failures = 0;
                self.locked_until = now + self.lock_seconds;
                self.lock_seconds = (self.lock_seconds * 2).min(LOCK_MAX);
                return Err(MaintError::Throttled { retry_after: self.locked_until - now });
            }
            return Err(MaintError::InvalidCode);
        };
        totp::record_used(&mut self.secrets.used_code_steps, step);
        self.save().map_err(MaintError::Internal)?;
        self.revoke_all().map_err(MaintError::Internal)?;
        self.failures = 0;
        self.lock_seconds = LOCK_FIRST;
        let (accept, sessions) = if root { (ROOT_WINDOW_SECONDS, ROOT_SESSION_SECONDS) } else { (WINDOW_SECONDS, SESSION_SECONDS) };
        let window = Window { id: random_id().map_err(MaintError::Internal)?, root, actor: actor.into(), client, opened_at: now, expires_at: now + accept, sessions_end_at: now + sessions };
        self.audit(if root { "maintenance.root_window_opened" } else { "maintenance.window_opened" },
            json!({"actor": actor, "client_addr": client.to_string(), "window_id": window.id, "expires_at": window.expires_at, "sessions_end_at": window.sessions_end_at}));
        self.window = Some(window.clone());
        Ok(window)
    }

    /// Signs a 5-minute user certificate for `public_key` in an open window,
    /// for the account that opened it and the address it was opened from.
    pub fn sign(&mut self, window_id: &str, public_key: &str, actor: &str, client_addr: &str, now: u64) -> Result<Value, MaintError> {
        let client = self.client_addr(client_addr)?;
        let window = match &self.window {
            Some(w) if w.id == window_id && now < w.expires_at => w.clone(),
            _ => return Err(MaintError::WindowClosed),
        };
        if window.actor != actor || window.client != client {
            self.audit("maintenance.certificate_refused", json!({"actor": actor, "client_addr": client.to_string(), "window_id": window.id,
                "window_actor": window.actor, "reason": "not_window_opener"}));
            return Err(MaintError::NotOpener);
        }
        let public = sshcert::parse_public_line(public_key).map_err(MaintError::InvalidParams)?;
        self.secrets.serial += 1;
        self.save().map_err(MaintError::Internal)?;
        let serial = self.secrets.serial;
        let key_id = format!("{}@{}", window.actor, window.id);
        let principals = vec![window.principal().to_string()];
        let options = [("source-address", format!("{client}/{}", if client.is_ipv4() { 32 } else { 128 }))];
        let valid_before = now + CERT_SECONDS;
        let certificate = sshcert::sign_certificate(&self.secrets.user_ca, &public, &CertSpec {
            kind: USER_CERT, serial, key_id: &key_id, principals: &principals,
            valid_after: now.saturating_sub(CERT_BACKDATE), valid_before,
            critical_options: &options, extensions: &["permit-pty"],
        }, &key_id).map_err(MaintError::Internal)?;
        self.audit(if window.root { "maintenance.root_certificate_issued" } else { "maintenance.certificate_issued" },
            json!({"actor": window.actor, "client_addr": client.to_string(), "window_id": window.id, "serial": serial, "valid_before": valid_before}));
        Ok(json!({
            "certificate": certificate, "valid_before": valid_before, "serial": serial,
            "principal": window.principal(), "user": window.user(), "host_ca": self.host_ca_line(),
        }))
    }

    /// Writes a host certificate for `names` (host names and addresses).
    pub fn write_host_certificate(&mut self, names: &[String], now: u64) -> Result<(), String> {
        if names.is_empty() { return Err("no host names to certify".into()); }
        self.secrets.serial += 1;
        self.save()?;
        let cert = sshcert::sign_certificate(&self.secrets.host_ca, self.secrets.host_key.public(), &CertSpec {
            kind: HOST_CERT, serial: self.secrets.serial, key_id: "stephania-host", principals: names,
            valid_after: now.saturating_sub(CERT_BACKDATE), valid_before: now + HOST_CERT_SECONDS,
            critical_options: &[], extensions: &[],
        }, "stephania-host")?;
        write_file(&self.dir.join("ssh_host_ed25519_key-cert.pub"), (cert + "\n").as_bytes(), 0o644)
    }

    /// Closes the window (explicitly or on expiry). Returns it if one was open.
    pub fn close(&mut self, window_id: Option<&str>, actor: &str, reason: &str) -> Result<Option<Window>, MaintError> {
        match (&self.window, window_id) {
            (None, _) => Ok(None),
            (Some(w), Some(id)) if w.id != id => Err(MaintError::WindowClosed),
            (Some(_), _) => {
                let w = self.window.take().expect("checked above");
                self.audit("maintenance.window_closed", json!({"actor": actor, "window_id": w.id, "root": w.root, "reason": reason}));
                if let Err(e) = self.revoke_all() { self.audit("maintenance.revocation_failed", json!({"error": e})); }
                Ok(Some(w))
            }
        }
    }

    /// Closes the window if `opener` opened it (an account was disabled,
    /// deleted or changed). Returns it if it was closed.
    pub fn close_opened_by(&mut self, opener: &str, actor: &str, reason: &str) -> Option<Window> {
        if self.window.as_ref().is_some_and(|w| w.actor == opener) { self.close(None, actor, reason).ok().flatten() } else { None }
    }

    /// Ends a window whose sessions have reached their limit.
    pub fn expire(&mut self, now: u64) -> Option<Window> {
        if self.window.as_ref().is_some_and(|w| now >= w.sessions_end_at) {
            return self.close(None, "stephania-maint", "expired").ok().flatten();
        }
        None
    }

    fn audit(&mut self, kind: &'static str, fields: Value) { self.audit.push(AuditEvent { kind, fields }); }

    fn client_addr(&self, s: &str) -> Result<IpAddr, MaintError> {
        let ip = lan_addr(s)?;
        if self.device_addrs.contains(&ip) { return Err(MaintError::InvalidParams("client is the device itself".into())); }
        Ok(ip)
    }
}

fn valid_actor(a: &str) -> bool {
    (1..=64).contains(&a.len()) && a.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// Only clients on the LAN may get certificates: private IPv4 and IPv6
/// (ULA) addresses and IPv4 link-local. Never loopback, which only the
/// device itself can use. IPv6 link-local is refused with a reason:
/// sshd compares the interface scope of such a peer, and a certificate's
/// source-address cannot name one, so the certificate would never match.
fn lan_addr(s: &str) -> Result<IpAddr, MaintError> {
    let (addr, zone) = s.split_once('%').map_or((s, false), |(a, _)| (a, true));
    let ip: IpAddr = addr.parse().map_err(|_| MaintError::InvalidParams("invalid client_addr".into()))?;
    let ip = unmap(ip);
    if zone && ip.is_ipv4() { return Err(MaintError::InvalidParams("invalid client_addr".into())); }
    if let IpAddr::V6(v6) = ip {
        if zone || (v6.segments()[0] & 0xffc0) == 0xfe80 {
            return Err(MaintError::InvalidParams("IPv6 link-local clients cannot use maintenance SSH; connect to the device's IPv4 or IPv6 LAN address".into()));
        }
    }
    let lan = match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
    };
    if lan { Ok(ip) } else { Err(MaintError::InvalidParams("client is not on the LAN".into())) }
}

/// A dual-stack listener reports IPv4 clients as ::ffff:a.b.c.d.
fn unmap(ip: IpAddr) -> IpAddr {
    match ip { IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4), v4 => v4 }
}

fn random_id() -> Result<String, String> {
    let mut b = [0u8; 8];
    SystemRandom::new().fill(&mut b).map_err(|_| "random source failed".to_string())?;
    Ok(hex(&b))
}

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() { return None; }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn write_file(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(&tmp)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.write_all(data).and_then(|_| f.sync_all()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 1_800_000_000;

    fn dir() -> PathBuf {
        let mut b = [0u8; 8];
        SystemRandom::new().fill(&mut b).unwrap();
        std::env::temp_dir().join(format!("stephania-maint-{}", hex(&b)))
    }

    fn code(m: &Maintenance, t: u64) -> String { format!("{:06}", totp::code(m.otp_secret(), t)) }

    fn ssh_key() -> String { Key::generate().unwrap().public_line("laptop") }

    #[test]
    fn creates_state_once_and_keeps_the_cas() {
        let d = dir();
        let first = Maintenance::open(&d).unwrap();
        let ca = std::fs::read_to_string(d.join("user_ca.pub")).unwrap();
        let again = Maintenance::open(&d).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("user_ca.pub")).unwrap(), ca);
        assert_eq!(first.otp_secret(), again.otp_secret());
        use std::os::unix::fs::PermissionsExt;
        for (f, mode) in [("state.json", 0o600), ("ssh_host_ed25519_key", 0o600), ("user_ca.pub", 0o644)] {
            assert_eq!(std::fs::metadata(d.join(f)).unwrap().permissions().mode() & 0o777, mode, "{f}");
        }
        std::fs::write(d.join("state.json"), b"{}").unwrap();
        assert!(Maintenance::open(&d).is_err(), "a malformed state must not be replaced");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_window_issues_short_user_certificates_bound_to_the_client() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        let w = m.open_window("alice", "192.168.1.20", &code(&m, T), false, T).unwrap();
        assert_eq!((w.expires_at, w.sessions_end_at, w.user()), (T + WINDOW_SECONDS, T + SESSION_SECONDS, "maint"));
        let r = m.sign(&w.id, &ssh_key(), "alice", "192.168.1.20", T + 5).unwrap();
        let cert = sshcert::parse_certificate(r["certificate"].as_str().unwrap()).unwrap();
        assert_eq!(cert.principals, vec!["maint".to_string()]);
        assert_eq!((cert.valid_after, cert.valid_before), (T + 5 - CERT_BACKDATE, T + 5 + CERT_SECONDS));
        assert_eq!(cert.key_id, format!("alice@{}", w.id));
        assert_eq!(cert.critical_options, vec![("source-address".to_string(), "192.168.1.20/32".to_string())]);
        assert_eq!(cert.extensions, vec!["permit-pty".to_string()]);
        let user_ca = std::fs::read_to_string(d.join("user_ca.pub")).unwrap();
        assert_eq!(&cert.signer, &sshcert::parse_public_line(&user_ca).unwrap());
        assert!(m.audit.iter().any(|e| e.kind == "maintenance.certificate_issued"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn root_windows_use_their_own_principal_and_shorter_limits() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        let w = m.open_window("alice", "10.0.0.4", &code(&m, T), true, T).unwrap();
        assert_eq!((w.expires_at, w.sessions_end_at, w.user()), (T + ROOT_WINDOW_SECONDS, T + ROOT_SESSION_SECONDS, "root"));
        let r = m.sign(&w.id, &ssh_key(), "alice", "10.0.0.4", T).unwrap();
        assert_eq!(sshcert::parse_certificate(r["certificate"].as_str().unwrap()).unwrap().principals, vec!["maint-root".to_string()]);
        assert!(m.audit.iter().any(|e| e.kind == "maintenance.root_window_opened"));
        assert!(m.audit.iter().any(|e| e.kind == "maintenance.root_certificate_issued"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn rotating_the_code_secret_retires_old_codes_and_closes_the_window() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        let old = m.otp_secret().to_vec();
        let w = m.open_window("alice", "10.0.0.4", &code(&m, T), false, T).unwrap();
        let (secret, closed) = m.rotate_otp_secret().unwrap();
        assert_eq!(closed.map(|c| c.id), Some(w.id));
        assert_ne!(secret, old);
        assert_eq!(secret.len(), 20);
        let stale = format!("{:06}", totp::code(&old, T + 30));
        assert_eq!(m.open_window("alice", "10.0.0.4", &stale, false, T + 30), Err(MaintError::InvalidCode));
        // The new secret persists, and its codes work (no step is marked used).
        let mut restarted = Maintenance::open(&d).unwrap();
        assert_eq!(restarted.otp_secret(), &secret[..]);
        assert!(restarted.open_window("alice", "10.0.0.4", &code(&restarted, T + 30), false, T + 30).is_ok());
        assert!(m.audit.iter().any(|e| e.kind == "maintenance.code_secret_rotated"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn ipv4_mapped_clients_are_treated_as_ipv4() {
        assert_eq!(lan_addr("::ffff:192.168.1.20").unwrap(), "192.168.1.20".parse::<IpAddr>().unwrap());
        assert!(lan_addr("::ffff:8.8.8.8").is_err());
        assert!(lan_addr("2001:db8::1").is_err());
        assert!(lan_addr("fd12:3456::7").is_ok());
        assert!(lan_addr("169.254.10.20").is_ok());
    }

    #[test]
    fn loopback_link_local_ipv6_and_the_device_itself_are_not_clients() {
        for bad in ["127.0.0.1", "127.1.2.3", "::1", "::ffff:127.0.0.1", "0.0.0.0", "fe80::1", "fe80::1%wlan0", "fd00::1%eth0", "10.0.0.4%eth0"] {
            assert!(matches!(lan_addr(bad), Err(MaintError::InvalidParams(_))), "{bad}");
        }
        assert!(lan_addr("fe80::1%wlan0").unwrap_err().message().contains("link-local"));
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        m.set_device_addrs(&["192.168.1.9".parse().unwrap(), "fd00::9".parse().unwrap()]);
        for own in ["192.168.1.9", "::ffff:192.168.1.9", "fd00::9"] {
            assert!(matches!(m.open_window("alice", own, &code(&m, T), true, T), Err(MaintError::InvalidParams(_))), "{own}");
        }
        assert!(matches!(m.open_window("alice", "127.0.0.1", &code(&m, T), true, T), Err(MaintError::InvalidParams(_))));
        let w = m.open_window("alice", "192.168.1.20", &code(&m, T), true, T).unwrap();
        assert!(matches!(m.sign(&w.id, &ssh_key(), "alice", "127.0.0.1", T), Err(MaintError::InvalidParams(_))));
        assert!(matches!(m.sign(&w.id, &ssh_key(), "alice", "192.168.1.9", T), Err(MaintError::InvalidParams(_))));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn only_the_opener_from_the_same_address_gets_certificates() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        let w = m.open_window("alice", "192.168.1.20", &code(&m, T), true, T).unwrap();
        assert_eq!(m.sign(&w.id, &ssh_key(), "bob", "192.168.1.30", T), Err(MaintError::NotOpener));
        assert_eq!(m.sign(&w.id, &ssh_key(), "bob", "192.168.1.20", T), Err(MaintError::NotOpener));
        assert_eq!(m.sign(&w.id, &ssh_key(), "alice", "192.168.1.30", T), Err(MaintError::NotOpener));
        assert!(m.audit.iter().any(|e| e.kind == "maintenance.certificate_refused" && e.fields["actor"] == "bob"));
        assert!(!m.audit.iter().any(|e| e.kind == "maintenance.root_certificate_issued"));
        assert!(m.sign(&w.id, &ssh_key(), "alice", "::ffff:192.168.1.20", T).is_ok());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn closing_for_an_account_only_closes_its_own_window() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        let w = m.open_window("alice", "192.168.1.20", &code(&m, T), false, T).unwrap();
        assert_eq!(m.close_opened_by("bob", "owner", "account_changed"), None);
        assert!(m.window().is_some());
        assert_eq!(m.close_opened_by("alice", "owner", "account_changed").map(|c| c.id), Some(w.id));
        assert!(m.window().is_none());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn certificates_from_earlier_windows_are_revoked() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        let krl = || std::fs::read(d.join("revoked.krl")).unwrap();
        let revoked_range = |list: &[u8]| -> Option<(u64, u64)> {
            // Header is 8 + 4 + 8 + 8 + 8 bytes, then two strings.
            let mut at = 36;
            for _ in 0..2 { at += 4 + u32::from_be_bytes(list[at..at + 4].try_into().unwrap()) as usize; }
            if at == list.len() { return None; }
            let tail = &list[list.len() - 16..];
            Some((u64::from_be_bytes(tail[..8].try_into().unwrap()), u64::from_be_bytes(tail[8..].try_into().unwrap())))
        };
        assert!(krl().starts_with(b"SSHKRL\n\0"));
        assert_eq!(revoked_range(&krl()), None, "nothing issued yet");
        let w = m.open_window("alice", "192.168.1.20", &code(&m, T), false, T).unwrap();
        let serial = m.sign(&w.id, &ssh_key(), "alice", "192.168.1.20", T).unwrap()["serial"].as_u64().unwrap();
        let in_window = revoked_range(&krl()).map_or(0, |(_, max)| max);
        assert!(serial > in_window, "the window's own certificates are not revoked");
        m.close(Some(&w.id), "alice", "done").unwrap();
        assert_eq!(revoked_range(&krl()), Some((1, serial)), "closing revokes the window's certificates");
        let w2 = m.open_window("alice", "192.168.1.20", &code(&m, T + 30), false, T + 30).unwrap();
        assert!(revoked_range(&krl()).unwrap().1 >= serial);
        m.close(Some(&w2.id), "alice", "done").unwrap();
        // A restart revokes everything issued before it, too.
        let serial_now = m.secrets.serial;
        drop(m);
        Maintenance::open(&d).unwrap();
        assert_eq!(revoked_range(&krl()), Some((1, serial_now)));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_clock_that_went_back_does_not_lock_out_the_owner() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        let w = m.open_window("alice", "10.0.0.4", &code(&m, T), false, T).unwrap();
        m.close(Some(&w.id), "alice", "done").unwrap();
        // Cold boot without RTC or network time: the clock is months behind.
        let behind = T - 90 * 86_400;
        let mut m = Maintenance::open(&d).unwrap();
        let c = code(&m, behind);
        let w = m.open_window("alice", "10.0.0.4", &c, false, behind).expect("a current code opens a window");
        m.close(Some(&w.id), "alice", "done").unwrap();
        assert_eq!(m.open_window("alice", "10.0.0.4", &c, false, behind + 1), Err(MaintError::InvalidCode), "codes still work once");
        // And the code used at the real time stays used once the clock is right again.
        let mut m = Maintenance::open(&d).unwrap();
        assert_eq!(m.open_window("alice", "10.0.0.4", &code(&m, T), false, T), Err(MaintError::InvalidCode));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn state_from_earlier_versions_keeps_its_last_code_step() {
        let d = dir();
        let m = Maintenance::open(&d).unwrap();
        let c = code(&m, T);
        let mut v: Value = serde_json::from_slice(&std::fs::read(d.join("state.json")).unwrap()).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("used_code_steps");
        o.insert("last_code_step".into(), json!(totp::step(T)));
        std::fs::write(d.join("state.json"), v.to_string()).unwrap();
        let mut m = Maintenance::open(&d).unwrap();
        assert_eq!(m.open_window("alice", "10.0.0.4", &c, false, T), Err(MaintError::InvalidCode));
        assert!(m.open_window("alice", "10.0.0.4", &code(&m, T + 30), false, T + 30).is_ok());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn codes_work_once_and_survive_a_restart() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        let c = code(&m, T);
        let w = m.open_window("alice", "10.0.0.4", &c, false, T).unwrap();
        m.close(Some(&w.id), "alice", "done").unwrap();
        assert_eq!(m.open_window("alice", "10.0.0.4", &c, false, T + 1), Err(MaintError::InvalidCode));
        let mut restarted = Maintenance::open(&d).unwrap();
        assert_eq!(restarted.open_window("alice", "10.0.0.4", &c, false, T + 2), Err(MaintError::InvalidCode));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn wrong_codes_lock_entry_with_a_growing_delay() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        let wrong = format!("{:06}", (totp::code(m.otp_secret(), T) + 1) % 1_000_000);
        for _ in 1..CODE_ATTEMPTS { assert_eq!(m.open_window("a", "10.0.0.4", &wrong, false, T), Err(MaintError::InvalidCode)); }
        assert_eq!(m.open_window("a", "10.0.0.4", &wrong, false, T), Err(MaintError::Throttled { retry_after: 60 }));
        assert_eq!(m.open_window("a", "10.0.0.4", &code(&m, T), false, T + 10), Err(MaintError::Throttled { retry_after: 50 }));
        for _ in 0..CODE_ATTEMPTS { let _ = m.open_window("a", "10.0.0.4", &wrong, false, T + 60); }
        assert_eq!(m.status(T + 60)["code_locked_until"], json!(T + 180));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn closed_expired_or_foreign_windows_sign_nothing() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        assert_eq!(m.sign("nope", &ssh_key(), "alice", "10.0.0.4", T).unwrap_err(), MaintError::WindowClosed);
        let w = m.open_window("alice", "10.0.0.4", &code(&m, T), false, T).unwrap();
        assert_eq!(m.open_window("bob", "10.0.0.5", &code(&m, T + 30), false, T + 30), Err(MaintError::WindowOpen));
        assert_eq!(m.sign(&w.id, &ssh_key(), "alice", "10.0.0.4", T + WINDOW_SECONDS).unwrap_err(), MaintError::WindowClosed);
        assert!(matches!(m.sign(&w.id, &ssh_key(), "alice", "8.8.8.8", T), Err(MaintError::InvalidParams(_))));
        assert!(matches!(m.sign(&w.id, "ssh-rsa AAAA x", "alice", "10.0.0.4", T), Err(MaintError::InvalidParams(_))));
        assert_eq!(m.expire(T + SESSION_SECONDS - 1), None);
        assert_eq!(m.expire(T + SESSION_SECONDS).map(|x| x.id), Some(w.id.clone()));
        assert!(m.window().is_none());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn host_certificates_name_the_device() {
        let d = dir();
        let mut m = Maintenance::open(&d).unwrap();
        m.write_host_certificate(&["stephania-1a2b.local".into(), "192.168.1.9".into()], T).unwrap();
        let cert = sshcert::parse_certificate(&std::fs::read_to_string(d.join("ssh_host_ed25519_key-cert.pub")).unwrap()).unwrap();
        assert_eq!(cert.kind, HOST_CERT);
        assert_eq!(cert.principals, vec!["stephania-1a2b.local".to_string(), "192.168.1.9".to_string()]);
        let host_key = std::fs::read_to_string(d.join("ssh_host_ed25519_key.pub")).unwrap();
        assert_eq!(&cert.public, &sshcert::parse_public_line(&host_key).unwrap());
        assert_eq!(&cert.signer, &sshcert::parse_public_line(&m.host_ca_line()).unwrap());
        std::fs::remove_dir_all(&d).ok();
    }
}
