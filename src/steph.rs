//! `steph`, the on-device maintenance CLI (docs/MAINTENANCE.md): the
//! parsers and probes behind its read commands. Every probe reads the
//! system directly and reports what it could not read instead of failing,
//! so `steph status` stays useful on a half-broken device. Nothing here
//! changes the device.
use serde_json::{json, Map, Value};
use std::path::Path;
use std::process::Command;

/// Short service names → systemd units.
pub const SERVICES: [(&str, &[&str]); 8] = [
    ("engine", &["stephania.service"]),
    ("control", &["stephania-control.service"]),
    ("phone", &["stephania-phone.service"]),
    ("ble", &["stephania-ble.service"]),
    ("provision", &["stephania-provision.service"]),
    ("pki", &["stephania-pki.service", "stephania-pki.timer"]),
    ("update", &["stephania-update-confirm.service", "stephania-update-deadline.timer"]),
    ("maint", &["stephania-maint.service", "stephania-sshd.service"]),
];

pub fn units_for(name: &str) -> Option<&'static [&'static str]> {
    SERVICES.iter().find(|(n, _)| *n == name).map(|(_, u)| *u)
}

pub fn all_units() -> Vec<&'static str> { SERVICES.iter().flat_map(|(_, u)| u.iter().copied()).collect() }

/// Output of a command, or why it could not run.
pub fn run(program: &str, args: &[&str]) -> Result<String, String> {
    match Command::new(program).args(args).output() {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => Err(format!("{program} failed: {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => Err(format!("{program}: {e}")),
    }
}

fn read(path: &str) -> Result<String, String> { std::fs::read_to_string(path).map(|s| s.trim().to_string()).map_err(|e| format!("{path}: {e}")) }

fn or_error(r: Result<Value, String>) -> Value { r.unwrap_or_else(|e| json!({"error": e})) }

// --- Parsers (unit-tested) ---------------------------------------------------

/// `systemctl show -p …` for several units: blocks separated by blank lines.
pub fn parse_systemctl_show(text: &str) -> Vec<Map<String, Value>> {
    text.split("\n\n").filter(|b| !b.trim().is_empty()).map(|block| {
        block.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), json!(v))).collect()
    }).collect()
}

/// `vcgencmd get_throttled` → the bits that matter, now and since boot.
pub fn parse_throttled(text: &str) -> Option<Value> {
    let hex = text.trim().strip_prefix("throttled=0x")?;
    let v = u32::from_str_radix(hex, 16).ok()?;
    let bit = |n: u32| v & (1 << n) != 0;
    Some(json!({
        "raw": format!("0x{v:x}"),
        "now": {"under_voltage": bit(0), "frequency_capped": bit(1), "throttled": bit(2), "soft_temp_limit": bit(3)},
        "since_boot": {"under_voltage": bit(16), "frequency_capped": bit(17), "throttled": bit(18), "soft_temp_limit": bit(19)},
    }))
}

/// `timedatectl show` key=value lines.
pub fn parse_key_values(text: &str) -> Map<String, Value> {
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), json!(v))).collect()
}

/// The certificate sshd exposes for this session (`ExposeAuthInfo`,
/// `$SSH_USER_AUTH`): `publickey ssh-ed25519-cert-v01@openssh.com AAAA…`.
pub fn session_certificate(auth_info: &str) -> Option<crate::sshcert::Certificate> {
    auth_info.lines().find_map(|l| {
        let rest = l.strip_prefix("publickey ")?;
        crate::sshcert::parse_certificate(rest).ok()
    })
}

/// `<account>@<window_id>`.
pub fn split_key_id(key_id: &str) -> (String, String) {
    match key_id.rsplit_once('@') {
        Some((a, w)) => (a.to_string(), w.to_string()),
        None => (key_id.to_string(), String::new()),
    }
}

// --- Probes ------------------------------------------------------------------

pub fn version() -> Value {
    let os = read("/etc/os-release").ok().and_then(|t| {
        parse_key_values(&t).get("PRETTY_NAME").and_then(Value::as_str).map(|s| s.trim_matches('"').to_string())
    });
    json!({
        "stephania": read("/etc/stephania-version").ok(),
        "os": os,
        "kernel": read("/proc/sys/kernel/osrelease").ok(),
        "booted_slot": booted_slot(),
    })
}

/// From the kernel command line (`rauc.slot=A`), which the boot selector sets.
pub fn booted_slot() -> Option<String> {
    let cmdline = read("/proc/cmdline").ok()?;
    cmdline.split_whitespace().find_map(|w| w.strip_prefix("rauc.slot=").map(str::to_string))
}

pub fn uptime_seconds() -> Option<u64> {
    read("/proc/uptime").ok()?.split_whitespace().next()?.parse::<f64>().ok().map(|s| s as u64)
}

pub fn services() -> Value {
    let units = all_units();
    let mut args = vec!["show", "-p", "Id,ActiveState,SubState,ActiveEnterTimestamp,NRestarts,MemoryCurrent"];
    args.extend(units.iter().copied());
    or_error(run("systemctl", &args).map(|t| {
        let by_id: Vec<Map<String, Value>> = parse_systemctl_show(&t);
        Value::Array(SERVICES.iter().flat_map(|(name, units)| units.iter().map(move |u| (*name, *u))).map(|(name, unit)| {
            let m = by_id.iter().find(|m| m.get("Id").and_then(Value::as_str) == Some(unit)).cloned().unwrap_or_default();
            let memory = m.get("MemoryCurrent").and_then(Value::as_str).and_then(|v| v.parse::<u64>().ok());
            json!({
                "service": name, "unit": unit,
                "active": m.get("ActiveState").cloned().unwrap_or(json!("unknown")),
                "sub": m.get("SubState").cloned().unwrap_or(Value::Null),
                "since": m.get("ActiveEnterTimestamp").cloned().filter(|v| v != "").unwrap_or(Value::Null),
                "restarts": m.get("NRestarts").and_then(Value::as_str).and_then(|v| v.parse::<u64>().ok()),
                "memory_bytes": memory,
            })
        }).collect())
    }))
}

pub fn update_status() -> Value {
    let rauc = run("rauc", &["status", "--output-format=json"]).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
    json!({
        "booted_slot": booted_slot(),
        "autoboot": read("/boot/selector/autoboot.txt").ok(),
        "trial_boot": read("/proc/device-tree/chosen/bootloader/tryboot").is_ok()
            && std::fs::read("/proc/device-tree/chosen/bootloader/tryboot").map(|b| b.last() == Some(&1)).unwrap_or(false),
        "rauc": or_error(rauc),
    })
}

pub fn net() -> Value {
    let host = read("/etc/hostname").ok();
    let addresses = run("ip", &["-j", "address", "show"]).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
    let interfaces: Value = or_error(addresses.map(|v| Value::Array(v.as_array().into_iter().flatten().filter(|i| i["ifname"] != "lo").map(|i| json!({
        "name": i["ifname"], "state": i["operstate"], "mac": i["address"],
        "addresses": i["addr_info"].as_array().into_iter().flatten().map(|a| format!("{}/{}", a["local"].as_str().unwrap_or(""), a["prefixlen"])).collect::<Vec<_>>(),
    })).collect())));
    let route = run("ip", &["-j", "route", "show", "default"]).ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.get(0).cloned())
        .map(|r| json!({"via": r["gateway"], "dev": r["dev"]}));
    let wifi = run("nmcli", &["-t", "-f", "ACTIVE,SSID,SIGNAL", "device", "wifi"]).ok().and_then(|t| {
        t.lines().find_map(|l| l.strip_prefix("yes:")).map(|rest| {
            let (ssid, signal) = rest.rsplit_once(':').unwrap_or((rest, ""));
            json!({"ssid": ssid.replace("\\:", ":"), "signal": signal.parse::<u32>().ok()})
        })
    });
    let urls = host.as_ref().map(|h| vec![format!("https://{h}.local:9443/")]).unwrap_or_default();
    json!({"host": host.map(|h| format!("{h}.local")), "interfaces": interfaces, "default_route": route, "wifi": wifi, "management_urls": urls})
}

pub fn hw() -> Value {
    let temp = read("/sys/class/thermal/thermal_zone0/temp").ok().and_then(|t| t.parse::<f64>().ok()).map(|m| (m / 100.0).round() / 10.0);
    let video: Vec<Value> = std::fs::read_dir("/sys/class/video4linux").into_iter().flatten().flatten().map(|e| {
        json!({"device": format!("/dev/{}", e.file_name().to_string_lossy()), "name": read(&format!("{}/name", e.path().display())).ok()})
    }).collect();
    json!({
        "temperature_c": temp,
        "throttling": run("vcgencmd", &["get_throttled"]).ok().and_then(|t| parse_throttled(&t)),
        "bootloader": run("vcgencmd", &["bootloader_version"]).ok().map(|t| t.lines().next().unwrap_or("").to_string()),
        "model": read("/proc/device-tree/model").ok().map(|m| m.trim_end_matches('\0').to_string()),
        "capture_devices": video,
        "audio_cards": read("/proc/asound/cards").ok(),
        "usb": run("lsusb", &["-t"]).ok(),
    })
}

pub fn time() -> Value {
    let rtc = Path::new("/sys/class/rtc/rtc0").exists();
    let battery = std::fs::read_dir("/sys/class/rtc/rtc0/device").ok().and_then(|_| read("/sys/class/rtc/rtc0/device/battery_voltage").ok());
    let t = run("timedatectl", &["show"]).map(|t| parse_key_values(&t));
    json!({
        "unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "ntp_synchronized": t.as_ref().ok().and_then(|m| m.get("NTPSynchronized")).map(|v| v == "yes"),
        "timezone": t.as_ref().ok().and_then(|m| m.get("Timezone")).cloned(),
        "rtc": rtc,
        "rtc_battery_microvolts": battery,
    })
}

pub fn disk() -> Value {
    let one = |path: &str| -> Value {
        let Ok(c) = std::ffi::CString::new(path) else { return Value::Null };
        // SAFETY: statvfs writes into a zeroed struct and reads a NUL-terminated path.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 { return json!({"path": path, "error": "unavailable"}) }
        let block = st.f_frsize as u64;
        let total = st.f_blocks as u64 * block;
        let free = st.f_bavail as u64 * block;
        json!({"path": path, "total_bytes": total, "free_bytes": free, "used_percent": ((total - free) * 100).checked_div(total).unwrap_or(0)})
    };
    json!(["/data", "/var/lib/stephania", "/overlay"].iter().filter(|p| Path::new(p).exists()).map(|p| one(p)).collect::<Vec<_>>())
}

pub fn whoami() -> Value {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let user = std::env::var("USER").unwrap_or_else(|_| if uid == 0 { "root".into() } else { uid.to_string() });
    let auth = std::env::var("SSH_USER_AUTH").ok().and_then(|p| std::fs::read_to_string(p).ok());
    let cert = auth.as_deref().and_then(session_certificate);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    match cert {
        Some(c) => {
            let (account, window) = split_key_id(&c.key_id);
            json!({
                "user": user, "level": if uid == 0 { "root" } else { "maint" }, "account": account, "window_id": window,
                "certificate_serial": c.serial, "certificate_valid_before": c.valid_before,
                "certificate_seconds_left": c.valid_before.saturating_sub(now),
            })
        }
        None => json!({"user": user, "level": if uid == 0 { "root" } else { "maint" }, "account": Value::Null,
            "note": "not a maintenance SSH session (no certificate in SSH_USER_AUTH)"}),
    }
}

pub fn status() -> Value {
    let services = services();
    let engine = services.as_array().and_then(|a| a.iter().find(|s| s["unit"] == "stephania.service")).map(|s| s["active"].clone());
    let failed: Vec<Value> = services.as_array().into_iter().flatten()
        .filter(|s| s["active"] == "failed").map(|s| s["unit"].clone()).collect();
    let hw = hw();
    let net = net();
    json!({
        "version": version(),
        "uptime_seconds": uptime_seconds(),
        "engine": engine,
        "failed_units": failed,
        "network": {"host": net["host"], "wifi": net["wifi"], "default_route": net["default_route"],
                    "addresses": net["interfaces"].as_array().into_iter().flatten().flat_map(|i| i["addresses"].as_array().cloned().unwrap_or_default()).collect::<Vec<_>>()},
        "temperature_c": hw["temperature_c"],
        "throttling": hw["throttling"],
        "disk": disk(),
        "session": whoami(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemctl_show_blocks() {
        let text = "Id=stephania.service\nActiveState=active\nSubState=running\nNRestarts=0\n\nId=stephania-control.service\nActiveState=failed\nSubState=failed\nNRestarts=3\n";
        let units = parse_systemctl_show(text);
        assert_eq!(units.len(), 2);
        assert_eq!(units[1]["ActiveState"], "failed");
        assert_eq!(units[1]["NRestarts"], "3");
    }

    #[test]
    fn throttled_bits() {
        let t = parse_throttled("throttled=0x50005\n").unwrap();
        assert_eq!(t["now"]["under_voltage"], true);
        assert_eq!(t["now"]["throttled"], true);
        assert_eq!(t["now"]["frequency_capped"], false);
        assert_eq!(t["since_boot"]["under_voltage"], true);
        assert_eq!(t["since_boot"]["throttled"], true);
        assert!(parse_throttled("garbage").is_none());
    }

    #[test]
    fn key_ids_and_services() {
        assert_eq!(split_key_id("alice@0123456789abcdef"), ("alice".into(), "0123456789abcdef".into()));
        assert_eq!(units_for("engine"), Some(&["stephania.service"][..]));
        assert!(units_for("sshd").is_none());
        assert!(all_units().contains(&"stephania-update-deadline.timer"));
    }

    #[test]
    fn session_certificate_from_auth_info() {
        let ca = crate::sshcert::Key::generate().unwrap();
        let user = crate::sshcert::Key::generate().unwrap();
        let spec = crate::sshcert::CertSpec {
            kind: crate::sshcert::USER_CERT, serial: 7, key_id: "alice@0123456789abcdef", principals: &["maint".to_string()],
            valid_after: 0, valid_before: 4_000_000_000, critical_options: &[], extensions: &["permit-pty"],
        };
        let line = crate::sshcert::sign_certificate(&ca, user.public(), &spec, "").unwrap();
        let info = format!("publickey {line}\n");
        let c = session_certificate(&info).unwrap();
        assert_eq!(c.serial, 7);
        assert_eq!(split_key_id(&c.key_id).0, "alice");
        assert!(session_certificate("password\n").is_none());
    }
}
