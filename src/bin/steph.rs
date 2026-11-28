//! steph: the command people use on the device once they are in through a
//! maintenance window (docs/MAINTENANCE.md). Read commands read the system
//! directly (`stephania::steph`); every one takes `--json`, with the shapes
//! in contracts/steph.schema.json. Commands that change the device go
//! through stephania-maint's broker socket, which checks, performs and
//! audits them.
#[cfg(unix)]
fn main() -> std::process::ExitCode { unix::main() }

#[cfg(not(unix))]
fn main() {
    eprintln!("steph runs on the device");
    std::process::exit(2);
}

#[cfg(unix)]
mod unix {
    use serde_json::{json, Value};
    use std::io::{IsTerminal, Write};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, ExitCode};
    use stephania::steph;

    const USAGE: &str = "usage: steph <command> [--json]
  status                    version, slot, uptime, services, network, temperature, disk, session
  whoami                    this session: user, level, account, window, certificate expiry
  version                   OS version, slot, kernel
  logs [<service>] [-f] [-n <lines>] [--since <time>] [--boot <n>]
  services                  every Stephania unit: state, since, restarts, memory
  update status             slots, booted slot, trial boot, RAUC
  net                       interfaces, addresses, Wi-Fi, mDNS name, management URL
  hw                        temperature, throttling, bootloader, capture and audio devices, USB
  time                      clock, NTP, RTC
  disk                      data partition and overlay use
  diag [-o <file>|-]        evidence bundle (tar.gz), secrets excluded
  restart <service> [--while-live]     restart one service (control ble provision pki engine phone)
  reboot [--while-live]                reboot into the committed slot
  update apply <bundle> [--while-live] verify and install a .raucb from the inbox, then trial boot
  window close                         end this window and its sessions now

services: engine control phone ble provision pki update maint";

    pub fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let json_out = args.iter().any(|a| a == "--json");
        let rest: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a != "--json").collect();
        let result = match rest.as_slice() {
            [] | ["help"] | ["-h"] | ["--help"] => { println!("{USAGE}"); return ExitCode::SUCCESS }
            ["status"] => Ok(steph::status()),
            ["whoami"] => Ok(steph::whoami()),
            ["version"] => Ok(steph::version()),
            ["services"] => Ok(steph::services()),
            ["update", "status"] => Ok(steph::update_status()),
            ["net"] => Ok(steph::net()),
            ["hw"] => Ok(steph::hw()),
            ["time"] => Ok(steph::time()),
            ["disk"] => Ok(steph::disk()),
            ["logs", more @ ..] => return logs(more),
            ["diag", more @ ..] => return diag(more),
            ["restart", unit, more @ ..] => return change("ops.restart", json!({"unit": unit}), more, &format!("restart {unit}")),
            ["reboot", more @ ..] => return change("ops.reboot", json!({}), more, "reboot"),
            ["update", "apply", bundle, more @ ..] => {
                let path = std::path::Path::new(bundle);
                let inbox = std::env::var("STEPHANIA_MAINT_INBOX").unwrap_or_else(|_| INBOX.to_string());
                let path = if path.is_absolute() || path.exists() { path.to_path_buf() } else { std::path::Path::new(&inbox).join(path) };
                return change("ops.update_apply", json!({"path": path.display().to_string()}), more, "install the update");
            }
            ["window", "close"] => return change("ops.window_close", json!({}), &[], "close the window"),
            _ => Err(format!("unknown command\n{USAGE}")),
        };
        match result {
            Ok(v) => {
                if json_out { println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()) } else { print_human(&v, 0) }
                ExitCode::SUCCESS
            }
            Err(e) => { eprintln!("steph: {e}"); ExitCode::from(1) }
        }
    }

    /// Indented `key: value` lines; arrays of objects as blocks.
    fn print_human(v: &Value, depth: usize) {
        let pad = "  ".repeat(depth);
        match v {
            Value::Object(m) => for (k, val) in m {
                match val {
                    Value::Object(_) | Value::Array(_) if !is_flat(val) => { println!("{pad}{k}:"); print_human(val, depth + 1) }
                    _ => println!("{pad}{k}: {}", scalar(val)),
                }
            },
            Value::Array(a) => for (i, item) in a.iter().enumerate() {
                if is_flat(item) { println!("{pad}- {}", scalar(item)) } else {
                    if i > 0 { println!() }
                    print_human(item, depth)
                }
            },
            other => println!("{pad}{}", scalar(other)),
        }
    }

    fn is_flat(v: &Value) -> bool {
        match v {
            Value::Array(a) => a.iter().all(|x| !x.is_object() && !x.is_array()),
            Value::Object(_) => false,
            _ => true,
        }
    }

    fn scalar(v: &Value) -> String {
        match v {
            Value::Null => "-".into(),
            Value::String(s) => s.trim_end().replace('\n', "\n    "),
            Value::Array(a) => a.iter().map(scalar).collect::<Vec<_>>().join(", "),
            other => other.to_string(),
        }
    }

    /// Where `stephctl cp <file> device:` puts uploads.
    const INBOX: &str = "/var/lib/stephania/inbox";

    /// A change through stephania-maint. `--while-live` overrides the on-air
    /// refusal after the device name is typed back.
    fn change(method: &str, mut params: Value, rest: &[&str], what: &str) -> ExitCode {
        let mut force = false;
        for a in rest {
            match *a {
                "--while-live" => force = true,
                other => { eprintln!("steph: unknown option {other}"); return ExitCode::from(1) }
            }
        }
        if force {
            let host = std::fs::read_to_string("/etc/hostname").map(|h| h.trim().to_string()).unwrap_or_default();
            eprint!("This may interrupt the stream. Type the device name ({host}) to {what} anyway: ");
            let _ = std::io::stderr().flush();
            let mut typed = String::new();
            let _ = std::io::stdin().read_line(&mut typed);
            if typed.trim() != host || host.is_empty() {
                eprintln!("steph: name did not match; nothing changed");
                return ExitCode::from(3);
            }
        }
        if let Value::Object(o) = &mut params {
            o.insert("actor".into(), json!(steph::actor()));
            o.insert("force".into(), json!(force));
            if method == "ops.window_close" { o.remove("force"); }
        }
        let timeout = if method == "ops.update_apply" { std::time::Duration::from_secs(30 * 60) } else { std::time::Duration::from_secs(120) };
        match steph::ops_call(method, params, timeout) {
            Ok(v) => {
                match method {
                    "ops.update_apply" => eprintln!("Installed. The device reboots into the new slot and commits it if it comes up healthy."),
                    "ops.reboot" => eprintln!("Rebooting."),
                    "ops.window_close" => eprintln!("Window closed."),
                    _ => eprintln!("Restarted {}.", v["target"].as_str().unwrap_or("")),
                }
                ExitCode::SUCCESS
            }
            Err((code, msg)) => {
                let (exit, text) = match code.as_str() {
                    "on_air" => (3, "on air: this would interrupt the stream (use --while-live)".to_string()),
                    "not_allowed" => (4, msg),
                    "failed" => (5, msg),
                    "unreachable" => (6, format!("stephania-maint is not running ({msg})")),
                    "window_closed" => (7, "maintenance window closed".to_string()),
                    _ => (1, if msg.is_empty() { code } else { msg }),
                };
                eprintln!("steph: {text}");
                ExitCode::from(exit)
            }
        }
    }

    /// journalctl for one service (short name) or all Stephania units.
    fn logs(args: &[&str]) -> ExitCode {
        let mut cmd = Command::new("journalctl");
        cmd.arg("--no-hostname");
        let mut units: Vec<&str> = steph::all_units();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match *a {
                "-f" => { cmd.arg("-f"); }
                "-n" | "--since" | "--boot" => {
                    let Some(v) = it.next() else { eprintln!("steph: {a} needs a value"); return ExitCode::from(1) };
                    cmd.arg(a).arg(v);
                }
                name if !name.starts_with('-') => match steph::units_for(name) {
                    Some(u) => units = u.to_vec(),
                    None => { eprintln!("steph: unknown service {name}\n{USAGE}"); return ExitCode::from(1) }
                },
                other => { eprintln!("steph: unknown option {other}"); return ExitCode::from(1) }
            }
        }
        for u in units { cmd.arg("-u").arg(u); }
        if !std::io::stdout().is_terminal() { cmd.arg("--no-pager"); }
        let e = cmd.exec();
        eprintln!("steph: journalctl: {e}");
        ExitCode::from(1)
    }

    /// Evidence bundle like tools/qualification/collect.sh: every read
    /// command's JSON, the Stephania journal of this boot and a few raw
    /// outputs. Never reads tokens, keys or credentials.
    fn diag(args: &[&str]) -> ExitCode {
        let out = match args {
            [] => None,
            ["-o", path] => Some(*path),
            _ => { eprintln!("steph: diag [-o <file>|-]"); return ExitCode::from(1) }
        };
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let name = format!("stephania-diag-{stamp}");
        let base = std::env::temp_dir().join(format!("steph-{}", std::process::id()));
        let dir = base.join(&name);
        if let Err(e) = std::fs::create_dir_all(&dir) { eprintln!("steph: {}: {e}", dir.display()); return ExitCode::from(1) }
        let put = |file: &str, v: &Value| { let _ = std::fs::write(dir.join(file), serde_json::to_string_pretty(v).unwrap_or_default()); };
        put("status.json", &steph::status());
        put("services.json", &steph::services());
        put("update.json", &steph::update_status());
        put("net.json", &steph::net());
        put("hw.json", &steph::hw());
        put("time.json", &steph::time());
        put("disk.json", &steph::disk());
        put("whoami.json", &steph::whoami());
        let raw = |file: &str, program: &str, args: &[&str]| {
            let text = steph::run(program, args).unwrap_or_else(|e| format!("unavailable: {e}\n"));
            let _ = std::fs::write(dir.join(file), format!("$ {program} {}\n{text}", args.join(" ")));
        };
        let mut journal = vec!["-b", "--no-pager", "-o", "short-iso", "-n", "5000"];
        let units = steph::all_units();
        for u in &units { journal.push("-u"); journal.push(u); }
        journal.extend(["-u", "bluetooth.service", "-u", "NetworkManager.service"]);
        raw("journal.txt", "journalctl", &journal);
        raw("kernel.txt", "journalctl", &["-k", "-b", "--no-pager", "-n", "2000"]);
        raw("rauc.txt", "rauc", &["status", "--detailed"]);
        raw("network.txt", "nmcli", &["-f", "GENERAL,IP4", "device", "show"]);
        raw("bootloader-config.txt", "vcgencmd", &["bootloader_config"]);
        if let Ok(e) = std::fs::read_to_string("/var/lib/stephania/pki/endpoint.json") { let _ = std::fs::write(dir.join("endpoint.json"), e); }
        let archive = base.join(format!("{name}.tar.gz"));
        let ok = Command::new("tar").arg("-C").arg(&base).arg("-czf").arg(&archive).arg(&name).status().map(|s| s.success()).unwrap_or(false);
        let result = if !ok {
            eprintln!("steph: tar failed");
            ExitCode::from(1)
        } else {
            match out {
                Some("-") => match std::fs::read(&archive) {
                    Ok(bytes) => { let _ = std::io::stdout().write_all(&bytes); ExitCode::SUCCESS }
                    Err(e) => { eprintln!("steph: {e}"); ExitCode::from(1) }
                },
                target => {
                    let dest = target.map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(format!("{name}.tar.gz")));
                    match std::fs::copy(&archive, &dest) {
                        Ok(_) => { eprintln!("{}", dest.display()); ExitCode::SUCCESS }
                        Err(e) => { eprintln!("steph: {}: {e}", dest.display()); ExitCode::from(1) }
                    }
                }
            }
        };
        let _ = std::fs::remove_dir_all(&base);
        result
    }
}
