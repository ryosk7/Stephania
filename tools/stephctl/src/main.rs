//! stephctl: maintenance access to a Stephania device from a laptop
//! (docs/MAINTENANCE.md). Signs in to the device's Studio account over
//! HTTPS pinned to the device CA, opens a maintenance window with the
//! 6-digit code from Stephania Connect, has a fresh Ed25519 key certified
//! for a few minutes, and runs the system `ssh`/`scp` with it.
mod api;
mod discover;
mod sshkey;
mod store;

use serde_json::{json, Value};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use store::{Device, Session};

const DEFAULT_PORT: u16 = 9443;
/// Where `stephctl cp <file> device:` puts uploads (the maintenance inbox).
const INBOX: &str = "/var/lib/stephania/inbox/";
const CODE_TRIES: usize = 3;

/// A failure with the exit code from docs/MAINTENANCE.md.
struct Fail(u8, String);

impl Fail {
    fn general(msg: impl Into<String>) -> Self { Fail(1, msg.into()) }
    fn unreachable(device: &str) -> Self { Fail(2, format!("cannot reach {device} on the LAN")) }
    fn pin() -> Self { Fail(3, "device certificate changed: refusing (run 'stephctl forget' after a factory reset)".into()) }
    fn not_admin() -> Self { Fail(5, "this account cannot open maintenance".into()) }
}

type Result<T> = std::result::Result<T, Fail>;

const USAGE: &str = "usage:
  stephctl login [<device>] [--username <name>] [--password-stdin] [--trust <fingerprint>]
  stephctl logout [<device>]
  stephctl devices
  stephctl ssh [<device>] [--root] [--keep-window] [--code <code>] [-- <command…>]
  stephctl cp [--root] [--keep-window] [--code <code>] <src…> <dst>   (device:path, scp syntax)
  stephctl window [<device>] [--close]
  stephctl forget <device>

<device> is a name from 'stephctl devices', a <host>.local name or an address.
With one known device it can be left out.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(Fail(code, msg)) => {
            eprintln!("stephctl: {msg}");
            ExitCode::from(code)
        }
    }
}

/// Options shared by the commands. Everything after `--` is the remote command.
#[derive(Default)]
struct Opts {
    positional: Vec<String>,
    remote: Vec<String>,
    root: bool,
    keep_window: bool,
    close: bool,
    password_stdin: bool,
    code: Option<String>,
    username: Option<String>,
    trust: Option<String>,
}

fn parse(args: &[String]) -> Result<Opts> {
    let mut o = Opts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| Fail::general(format!("{name} needs a value")));
        match a.as_str() {
            "--" => { o.remote = it.by_ref().cloned().collect(); break; }
            "--root" => o.root = true,
            "--keep-window" => o.keep_window = true,
            "--close" => o.close = true,
            "--password-stdin" => o.password_stdin = true,
            "--code" => o.code = Some(value("--code")?),
            "--username" => o.username = Some(value("--username")?),
            "--trust" => o.trust = Some(value("--trust")?),
            s if s.starts_with("--") => return Err(Fail::general(format!("unknown option {s}\n{USAGE}"))),
            _ => o.positional.push(a.clone()),
        }
    }
    Ok(o)
}

fn run(args: &[String]) -> Result<ExitCode> {
    let Some(cmd) = args.first() else {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    };
    let o = parse(&args[1..])?;
    let one = |o: &Opts| -> Result<Option<String>> {
        match o.positional.len() {
            0 => Ok(None),
            1 => Ok(Some(o.positional[0].clone())),
            _ => Err(Fail::general(format!("too many arguments\n{USAGE}"))),
        }
    };
    match cmd.as_str() {
        "login" => login(one(&o)?.as_deref(), &o).map(|_| ExitCode::SUCCESS),
        "logout" => logout(one(&o)?.as_deref()).map(|_| ExitCode::SUCCESS),
        "devices" => devices().map(|_| ExitCode::SUCCESS),
        "ssh" => ssh(one(&o)?.as_deref(), &o),
        "cp" => cp(&o),
        "window" => window(one(&o)?.as_deref(), o.close).map(|_| ExitCode::SUCCESS),
        "forget" => forget(one(&o)?.as_deref().ok_or_else(|| Fail::general("forget needs a device"))?).map(|_| ExitCode::SUCCESS),
        "-V" | "--version" | "version" => {
            println!("stephctl {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(Fail::general(format!("unknown command {other}\n{USAGE}"))),
    }
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

// --- Prompts ----------------------------------------------------------------

fn read_line(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|e| Fail::general(e.to_string()))?;
    Ok(line.trim().to_string())
}

fn read_secret(prompt: &str) -> Result<String> {
    if std::io::stdin().is_terminal() {
        rpassword::prompt_password(prompt).map_err(|e| Fail::general(e.to_string()))
    } else {
        read_line(prompt)
    }
}

// --- Devices and connections ------------------------------------------------

/// A known device by name, host name or address; the only one when none is given.
fn known(target: Option<&str>) -> Result<Device> {
    let all = store::load_devices().map_err(Fail::general)?;
    match target {
        None => match all.len() {
            0 => Err(Fail::general("no device yet: run 'stephctl login'")),
            1 => Ok(all[0].clone()),
            _ => Err(Fail::general(format!(
                "several devices are known, name one: {}",
                all.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ")
            ))),
        },
        Some(t) => {
            let t = t.trim_end_matches('.');
            all.into_iter()
                .find(|d| d.name.eq_ignore_ascii_case(t) || d.host.eq_ignore_ascii_case(t)
                    || d.host.eq_ignore_ascii_case(&format!("{t}.local")) || d.addresses.iter().any(|a| a == t))
                .ok_or_else(|| Fail::general(format!("{t} is not known: run 'stephctl login {t}'")))
        }
    }
}

fn is_tls_error(e: &ureq::Error) -> bool {
    matches!(e, ureq::Error::Rustls(_) | ureq::Error::Tls(_))
}

/// A client for the first address that answers with the pinned CA. When
/// none answers, the device may have a new address: ask mDNS and remember
/// it. A certificate that does not chain to the pin is refused (exit 3).
fn connect(device: &mut Device) -> Result<(api::Client, String)> {
    let mut tried = Vec::new();
    let mut candidates = device.addresses.clone();
    if !device.host.is_empty() {
        candidates.push(device.host.clone());
    }
    for round in 0..2 {
        for address in candidates.iter().filter(|a| !tried.contains(*a)).cloned().collect::<Vec<_>>() {
            tried.push(address.clone());
            let client = api::Client::new(&device.ca_pem, &address, device.port).map_err(Fail::general)?;
            match client.call("GET", "/ca.pem", None, None) {
                Ok(r) if r.status == 200 => {
                    if round == 1 || !device.addresses.contains(&address) {
                        let mut addresses = vec![address.clone()];
                        addresses.extend(device.addresses.iter().filter(|a| **a != address).cloned());
                        addresses.truncate(4);
                        device.addresses = addresses;
                        let _ = store::upsert_device(device.clone());
                    }
                    return Ok((client, address));
                }
                Ok(_) => {}
                Err(e) if is_tls_error(&e) => return Err(Fail::pin()),
                Err(_) => {}
            }
        }
        if round == 0 && !device.host.is_empty() {
            candidates = discover::resolve(&device.host);
        }
    }
    Err(Fail::unreachable(if device.host.is_empty() { &device.name } else { &device.host }))
}

fn split_port(target: &str) -> (String, u16) {
    if let Some((h, p)) = target.rsplit_once(':') {
        if !h.contains(':') {
            if let Ok(p) = p.parse() {
                return (h.to_string(), p);
            }
        }
    }
    (target.to_string(), DEFAULT_PORT)
}

fn short_name(host: &str) -> String { host.trim_end_matches('.').trim_end_matches(".local").to_string() }

/// First contact: find the device, show its CA fingerprint and pin it once
/// the user confirms it matches Stephania Connect.
fn first_contact(target: Option<&str>, o: &Opts) -> Result<Device> {
    let (host, addresses, port) = match target {
        Some(t) => {
            let (t, port) = split_port(t);
            if t.parse::<std::net::IpAddr>().is_ok() {
                (String::new(), vec![t.clone()], port)
            } else {
                let host = if t.ends_with(".local") { t.clone() } else { format!("{t}.local") };
                let mut addresses = discover::resolve(&host);
                addresses.push(host.clone());
                (host, addresses, port)
            }
        }
        None => {
            eprintln!("Looking for Stephania devices on the LAN…");
            let found = discover::browse(Duration::from_secs(4));
            let pick = match found.len() {
                0 => return Err(Fail(2, "no Stephania device found on the LAN (give its name or address)".into())),
                1 => found[0].clone(),
                _ => {
                    for (i, f) in found.iter().enumerate() {
                        eprintln!("  {}. {} ({})", i + 1, f.host, f.addresses.join(", "));
                    }
                    let n: usize = read_line("Device number: ")?.parse().map_err(|_| Fail::general("not a number"))?;
                    found.get(n.wrapping_sub(1)).cloned().ok_or_else(|| Fail::general("no such device"))?
                }
            };
            (pick.host, pick.addresses, pick.port)
        }
    };
    let label = if host.is_empty() { addresses[0].clone() } else { host.clone() };
    let (address, ca_pem) = addresses.iter()
        .find_map(|a| api::fetch_ca_unverified(a, port).ok().map(|pem| (a.clone(), pem)))
        .ok_or_else(|| Fail::unreachable(&label))?;
    let der = api::pem_to_der(&ca_pem).ok_or_else(|| Fail::general("the device sent no certificate"))?;
    let sha = api::sha256_hex(&der);
    let fingerprint = api::short_fingerprint(&sha);
    eprintln!("Device {label} ({address})");
    eprintln!("Certificate fingerprint: {fingerprint}");
    match &o.trust {
        Some(given) => {
            let norm = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase();
            if norm(given) != norm(&fingerprint) {
                return Err(Fail(3, "fingerprint does not match --trust: refusing".into()));
            }
        }
        None => {
            eprintln!("Compare it with the fingerprint on the device screen in Stephania Connect.");
            let answer = read_line("Does it match? [y/N] ")?;
            if !matches!(answer.as_str(), "y" | "Y" | "yes" | "Yes") {
                return Err(Fail(3, "not trusted".into()));
            }
        }
    }
    let name = if host.is_empty() { address.clone() } else { short_name(&host) };
    let mut all_addresses = vec![address.clone()];
    all_addresses.extend(addresses.into_iter().filter(|a| *a != address && a.parse::<std::net::IpAddr>().is_ok()));
    Ok(Device { name, host, addresses: all_addresses, port, ca_sha256: sha, ca_pem })
}

// --- Sessions ---------------------------------------------------------------

fn sign_in(client: &api::Client, device: &Device, o: &Opts) -> Result<Session> {
    for attempt in 0..3 {
        let username = match (&o.username, std::env::var("STEPHCTL_USERNAME").ok()) {
            (Some(u), _) => u.clone(),
            (None, Some(u)) if !u.is_empty() => u,
            _ => read_line(&format!("Studio account on {}: ", device.name))?,
        };
        let password = if o.password_stdin {
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line).map_err(|e| Fail::general(e.to_string()))?;
            line.trim_end_matches(['\r', '\n']).to_string()
        } else {
            read_secret("Password: ")?
        };
        let r = client.call("POST", "/api/v1/auth/session", Some(&json!({"username": username, "password": password})), None)
            .map_err(|_| Fail::unreachable(&device.name))?;
        match r.status {
            201 => {
                let cookie = r.set_cookie.clone().ok_or_else(|| Fail::general("the device sent no session"))?;
                let session = Session {
                    cookie,
                    csrf_token: r.body["csrf_token"].as_str().unwrap_or_default().to_string(),
                    expires_at_unix: r.body["expires_at_unix"].as_u64().unwrap_or(0),
                    username: username.clone(),
                };
                store::save_session(&device.name, &session).map_err(Fail::general)?;
                if r.body["user"]["role"].as_str() != Some("admin") {
                    eprintln!("Signed in as {username}, who is not an admin: maintenance needs an admin account.");
                } else {
                    eprintln!("Signed in to {} as {username}.", device.name);
                }
                return Ok(session);
            }
            429 => return Err(Fail::general(format!("too many failed sign-ins, try again in {} s", r.retry_after.unwrap_or(60)))),
            401 if attempt < 2 && !o.password_stdin && o.username.is_none() => eprintln!("Unknown user or wrong password."),
            401 => return Err(Fail::general("unknown user or wrong password")),
            s => return Err(Fail::general(format!("sign-in failed ({s}: {})", r.error_code()))),
        }
    }
    Err(Fail::general("unknown user or wrong password"))
}

/// The saved session, or a new sign-in when it is missing or expired.
fn session(client: &api::Client, device: &Device, o: &Opts) -> Result<Session> {
    match store::load_session(&device.name) {
        Some(s) if s.expires_at_unix > now() + 30 => Ok(s),
        _ => sign_in(client, device, o),
    }
}

/// One authenticated call; signs in again once if the session was rejected.
fn authed(client: &api::Client, device: &Device, o: &Opts, s: &mut Session, method: &str, path: &str, body: Option<&Value>) -> Result<api::Response> {
    for _ in 0..2 {
        let r = client.call(method, path, body, Some((&s.cookie, &s.csrf_token))).map_err(|_| Fail::unreachable(&device.name))?;
        if r.status == 401 && r.error_code() != "invalid_code" {
            store::delete_session(&device.name);
            *s = sign_in(client, device, o)?;
            continue;
        }
        if r.status == 403 && (r.error_code() == "forbidden_role" || r.error_code() == "forbidden") {
            return Err(Fail::not_admin());
        }
        return Ok(r);
    }
    Err(Fail::general("the device keeps refusing the session"))
}

// --- Commands ---------------------------------------------------------------

fn login(target: Option<&str>, o: &Opts) -> Result<()> {
    let existing = target.map_or_else(|| known(None).ok(), |t| known(Some(t)).ok());
    let mut device = match existing {
        Some(d) => d,
        None => first_contact(target, o)?,
    };
    let (client, _) = connect(&mut device)?;
    store::upsert_device(device.clone()).map_err(Fail::general)?;
    sign_in(&client, &device, o)?;
    Ok(())
}

fn logout(target: Option<&str>) -> Result<()> {
    let mut device = known(target)?;
    if let Some(s) = store::load_session(&device.name) {
        if let Ok((client, _)) = connect(&mut device) {
            let _ = client.call("DELETE", "/api/v1/auth/session", None, Some((&s.cookie, &s.csrf_token)));
        }
    }
    store::delete_session(&device.name);
    eprintln!("Signed out of {}.", device.name);
    Ok(())
}

fn devices() -> Result<()> {
    let all = store::load_devices().map_err(Fail::general)?;
    if all.is_empty() {
        println!("No devices. Run 'stephctl login'.");
        return Ok(());
    }
    for d in all {
        let session = match store::load_session(&d.name) {
            Some(s) if s.expires_at_unix > now() => format!("signed in as {}", s.username),
            _ => "signed out".to_string(),
        };
        let host = if d.host.is_empty() { "-" } else { &d.host };
        println!("{}  {}  {}:{}  {}  {}", d.name, host, d.addresses.first().map(String::as_str).unwrap_or("-"), d.port,
            api::short_fingerprint(&d.ca_sha256), session);
    }
    Ok(())
}

fn forget(target: &str) -> Result<()> {
    let device = known(Some(target))?;
    store::delete_session(&device.name);
    store::remove_device(&device.name).map_err(Fail::general)?;
    eprintln!("Forgot {} (its pin and session).", device.name);
    Ok(())
}

fn window(target: Option<&str>, close: bool) -> Result<()> {
    let o = Opts::default();
    let mut device = known(target)?;
    let (client, _) = connect(&mut device)?;
    let mut s = session(&client, &device, &o)?;
    if close {
        let r = authed(&client, &device, &o, &mut s, "DELETE", "/api/v1/maintenance/window", None)?;
        if r.status != 204 {
            return Err(Fail::general(format!("could not close the window ({}: {})", r.status, r.error_code())));
        }
        println!("Maintenance window closed.");
        return Ok(());
    }
    let r = authed(&client, &device, &o, &mut s, "GET", "/api/v1/maintenance", None)?;
    if r.status != 200 {
        return Err(Fail::general(format!("maintenance status failed ({}: {})", r.status, r.error_code())));
    }
    match r.body.get("window").filter(|w| !w.is_null() && r.body["open"] == json!(true)) {
        None => println!("No maintenance window is open."),
        Some(w) => {
            let left = |k: &str| w[k].as_u64().map(|t| t.saturating_sub(now())).unwrap_or(0);
            println!("{} window {} opened by {}; new sessions for {} s, sessions end in {} s.",
                if w["root"] == json!(true) { "Root" } else { "Maintenance" },
                w["window_id"].as_str().unwrap_or("?"), w["actor"].as_str().unwrap_or("?"), left("expires_at"), left("sessions_end_at"));
        }
    }
    Ok(())
}

/// A window and a certificate for one `ssh`/`scp` run.
struct Access {
    window_id: String,
    opened: bool,
    user: String,
    port: u16,
    certificate: String,
    host_ca: String,
}

/// Uses the open window of the requested level, or opens one with a code
/// from Stephania Connect; then has `key` certified.
fn access(client: &api::Client, device: &Device, o: &Opts, s: &mut Session, key: &sshkey::Key) -> Result<Access> {
    for _ in 0..2 {
        let status = authed(client, device, o, s, "GET", "/api/v1/maintenance", None)?;
        if status.status != 200 {
            return Err(Fail::general(format!("maintenance status failed ({}: {})", status.status, status.error_code())));
        }
        let open = status.body.get("window").filter(|w| !w.is_null() && status.body["open"] == json!(true)).cloned();
        let (window_id, opened) = match open {
            Some(w) if (w["root"] == json!(true)) == o.root && w["accepting"] != json!(false) => (w["window_id"].as_str().unwrap_or_default().to_string(), false),
            Some(w) => {
                let level = if w["root"] == json!(true) { "root" } else { "maintenance" };
                return Err(Fail::general(format!("a {level} window is open; close it first with 'stephctl window --close'")));
            }
            None => (open_window(client, device, o, s)?, true),
        };
        let r = authed(client, device, o, s, "POST", "/api/v1/maintenance/certificate",
            Some(&json!({"window_id": window_id, "public_key": key.public_line(&format!("stephctl@{}", device.name))})))?;
        match r.status {
            200 => {
                let port = std::env::var("STEPHCTL_SSH_PORT").ok().and_then(|p| p.parse().ok()).or_else(|| r.body["port"].as_u64().map(|p| p as u16)).unwrap_or(22);
                return Ok(Access {
                    window_id,
                    opened,
                    user: std::env::var("STEPHCTL_SSH_USER").ok().filter(|u| !u.is_empty())
                        .unwrap_or_else(|| r.body["user"].as_str().unwrap_or("maint").to_string()),
                    port,
                    certificate: r.body["certificate"].as_str().unwrap_or_default().to_string(),
                    host_ca: r.body["host_ca"].as_str().unwrap_or_default().to_string(),
                });
            }
            409 => eprintln!("The maintenance window closed meanwhile."),
            s => return Err(Fail::general(format!("certificate request failed ({s}: {})", r.error_code()))),
        }
    }
    Err(Fail(4, "maintenance window closed".into()))
}

fn open_window(client: &api::Client, device: &Device, o: &Opts, s: &mut Session) -> Result<String> {
    for attempt in 0..CODE_TRIES {
        let code = match (&o.code, std::env::var("STEPHCTL_CODE").ok().filter(|c| !c.is_empty())) {
            (Some(c), _) if attempt == 0 => c.clone(),
            (None, Some(c)) if attempt == 0 => c,
            (Some(_), _) | (None, Some(_)) => return Err(Fail(4, "code not accepted".into())),
            _ => read_line(if o.root { "Code from Stephania Connect (root window): " } else { "Code from Stephania Connect: " })?,
        };
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            eprintln!("The code has 6 digits.");
            continue;
        }
        let r = authed(client, device, o, s, "POST", "/api/v1/maintenance/window", Some(&json!({"code": code, "root": o.root})))?;
        match (r.status, r.error_code()) {
            (201, _) => return Ok(r.body["window_id"].as_str().unwrap_or_default().to_string()),
            (401, _) => eprintln!("Code not accepted."),
            (429, _) => return Err(Fail(4, format!("too many wrong codes, try again in {} s", r.retry_after.unwrap_or(60)))),
            (409, _) => return Err(Fail::general("another maintenance window was opened meanwhile; try again")),
            (503, code) => return Err(Fail::general(format!("the device could not open maintenance ({code})"))),
            (st, code) => return Err(Fail::general(format!("opening the window failed ({st}: {code})"))),
        }
    }
    Err(Fail(4, "code not accepted".into()))
}

/// A 0700 directory for the key, the certificate and `known_hosts`;
/// overwritten and removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self> {
        let mut random = [0u8; 8];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random).map_err(|_| Fail::general("random source failed"))?;
        let dir = std::env::temp_dir().join(format!("stephctl-{}", random.iter().map(|b| format!("{b:02x}")).collect::<String>()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir).map_err(|e| Fail::general(format!("{}: {e}", dir.display())))?;
        Ok(Self(dir))
    }

    fn write(&self, name: &str, text: &str) -> Result<PathBuf> {
        let path = self.0.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path).and_then(|mut f| f.write_all(text.as_bytes())).map_err(|e| Fail::general(format!("{}: {e}", path.display())))?;
        Ok(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for e in entries.flatten() {
                if let Ok(meta) = e.metadata() {
                    let _ = std::fs::write(e.path(), vec![0u8; meta.len() as usize]);
                }
                let _ = std::fs::remove_file(e.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn find_program(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|p| p.join(&exe)).find(|p| p.is_file())
}

/// The options both `ssh` and `scp` get: only this key and certificate,
/// no agent, and the device's host CA as the only trusted host key.
fn ssh_options(scratch: &Scratch, device: &Device, address: &str, a: &Access, key_pem: &str) -> Result<Vec<String>> {
    let id = scratch.write("id", key_pem)?;
    let cert = scratch.write("id-cert.pub", &format!("{}\n", a.certificate))?;
    let alias = if device.host.is_empty() { address.to_string() } else { device.host.clone() };
    let mut names = vec![alias.clone()];
    names.extend(device.addresses.iter().filter(|n| **n != alias).cloned());
    let known_hosts = scratch.write("known_hosts", &format!("@cert-authority {} {}\n", names.join(","), a.host_ca))?;
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    Ok(vec![
        "-F".into(), null.into(),
        "-o".into(), "IdentitiesOnly=yes".into(),
        "-o".into(), "IdentityAgent=none".into(),
        "-o".into(), format!("IdentityFile={}", id.display()),
        "-o".into(), format!("CertificateFile={}", cert.display()),
        "-o".into(), format!("UserKnownHostsFile={}", known_hosts.display()),
        "-o".into(), format!("GlobalKnownHostsFile={null}"),
        "-o".into(), "StrictHostKeyChecking=yes".into(),
        "-o".into(), format!("HostKeyAlias={alias}"),
        "-o".into(), "ConnectTimeout=10".into(),
    ])
}

fn host_for_ssh(address: &str) -> String {
    if address.contains(':') { format!("[{address}]") } else { address.to_string() }
}

/// Signs in, gets a window and a certificate, and runs `program` with the
/// options plus `tail(user, address)`. Closes a window it opened unless
/// `--keep-window`.
fn with_access(device: &mut Device, o: &Opts, program: &str, port_flag: &str, tail: impl FnOnce(&str, &str) -> Vec<String>) -> Result<ExitCode> {
    let bin = find_program(program).ok_or(Fail(6, "OpenSSH client not found in PATH".into()))?;
    let (client, address) = connect(device)?;
    let mut s = session(&client, device, o)?;
    let key = sshkey::Key::generate().map_err(Fail::general)?;
    let a = access(&client, device, o, &mut s, &key)?;
    let mut pem = key.private_pem(&format!("stephctl@{}", device.name)).map_err(Fail::general)?;
    drop(key);
    let scratch = Scratch::new()?;
    let options = ssh_options(&scratch, device, &address, &a, &pem);
    // SAFETY: zeroing ASCII bytes keeps the String valid UTF-8.
    unsafe { pem.as_bytes_mut().fill(b'0') };
    let mut args = options?;
    args.push(port_flag.into());
    args.push(a.port.to_string());
    args.extend(tail(&a.user, &address));
    let status = std::process::Command::new(&bin).args(&args).status().map_err(|e| Fail(6, format!("{}: {e}", bin.display())))?;
    drop(scratch);
    if a.opened && !o.keep_window {
        let _ = authed(&client, device, o, &mut s, "DELETE", "/api/v1/maintenance/window", None);
    } else if a.opened {
        eprintln!("Window {} stays open ('stephctl window --close' ends it).", a.window_id);
    }
    Ok(ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8))
}

fn ssh(target: Option<&str>, o: &Opts) -> Result<ExitCode> {
    let mut device = known(target)?;
    let remote = o.remote.clone();
    with_access(&mut device, o, "ssh", "-p", move |user, address| {
        let mut tail = vec![format!("{user}@{}", address)];
        if !remote.is_empty() {
            tail.push("--".into());
            tail.extend(remote);
        }
        tail
    })
}

/// `device:path`, `<name>:path` (a known device) or a local path.
fn remote_part<'a>(arg: &'a str, all: &[Device]) -> Option<(Option<&'a str>, &'a str)> {
    let (prefix, path) = arg.split_once(':')?;
    if Path::new(arg).exists() || (cfg!(windows) && prefix.len() == 1) {
        return None;
    }
    if prefix == "device" || prefix.is_empty() {
        return Some((None, path));
    }
    all.iter().any(|d| d.name.eq_ignore_ascii_case(prefix) || short_name(&d.host).eq_ignore_ascii_case(prefix)).then_some((Some(prefix), path))
}

fn cp(o: &Opts) -> Result<ExitCode> {
    if o.positional.len() < 2 {
        return Err(Fail::general(format!("cp needs a source and a destination\n{USAGE}")));
    }
    let all = store::load_devices().map_err(Fail::general)?;
    let parts: Vec<_> = o.positional.iter().map(|a| remote_part(a, &all)).collect();
    let targets: Vec<Option<&str>> = parts.iter().flatten().map(|(d, _)| *d).collect();
    if targets.is_empty() {
        return Err(Fail::general("one side must be on the device (device:path)"));
    }
    if targets.windows(2).any(|w| w[0] != w[1]) {
        return Err(Fail::general("copy to or from one device at a time"));
    }
    let mut device = known(targets[0])?;
    let positional = o.positional.clone();
    let last = positional.len() - 1;
    with_access(&mut device, o, "scp", "-P", move |user, address| {
        let host = host_for_ssh(address);
        // Legacy SCP protocol: the device's sshd has no SFTP subsystem.
        let mut tail = vec!["-O".to_string()];
        for (i, (arg, part)) in positional.iter().zip(parts).enumerate() {
            tail.push(match part {
                Some((_, path)) => {
                    let path = if path.is_empty() && i == last { INBOX } else { path };
                    format!("{user}@{host}:{path}")
                }
                None => arg.clone(),
            });
        }
        tail
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_and_remote_command() {
        let args: Vec<String> = ["dev", "--root", "--code", "123456", "--", "steph", "status"].iter().map(|s| s.to_string()).collect();
        let o = parse(&args).ok().unwrap();
        assert_eq!(o.positional, ["dev"]);
        assert!(o.root);
        assert_eq!(o.code.as_deref(), Some("123456"));
        assert_eq!(o.remote, ["steph", "status"]);
    }

    #[test]
    fn host_and_port() {
        assert_eq!(split_port("stephania-1.local:9443"), ("stephania-1.local".into(), 9443));
        assert_eq!(split_port("10.0.0.2"), ("10.0.0.2".into(), DEFAULT_PORT));
        assert_eq!(split_port("fe80::1"), ("fe80::1".into(), DEFAULT_PORT));
        assert_eq!(short_name("stephania-8f21.local"), "stephania-8f21");
    }

    #[test]
    fn copy_targets() {
        let d = Device { name: "hall-a".into(), host: "stephania-8f21.local".into(), addresses: vec![], port: 9443, ca_sha256: String::new(), ca_pem: String::new() };
        let all = vec![d];
        assert_eq!(remote_part("device:/tmp/x", &all), Some((None, "/tmp/x")));
        assert_eq!(remote_part("hall-a:", &all), Some((Some("hall-a"), "")));
        assert_eq!(remote_part("stephania-8f21:/var/log", &all), Some((Some("stephania-8f21"), "/var/log")));
        assert_eq!(remote_part("bundle.raucb", &all), None);
        assert_eq!(remote_part("other:/x", &all), None);
    }
}
