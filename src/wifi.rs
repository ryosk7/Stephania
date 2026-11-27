//! NetworkManager provisioning helper. WPA-PSK only, no enterprise/captive portal.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn networks() -> Result<Vec<String>, String> {
    let result = Command::new("nmcli")
        .args(["-t", "-f", "SSID", "device", "wifi", "list", "--rescan", "no"])
        .output().map_err(|e| e.to_string())?;
    if !result.status.success() { return Err("NetworkManager Wi-Fi scan failed".into()); }
    let text = String::from_utf8_lossy(&result.stdout);
    let mut items: Vec<String> = text.lines().filter(|s| !s.is_empty() && !s.contains('\0'))
        .map(|s| s.replace("\\:", ":").replace("\\\\", "\\")).collect();
    items.sort(); items.dedup();
    Ok(items)
}

pub fn connect(ssid: &str, password: &str) -> Result<(), String> {
    if ssid.is_empty() || ssid.len() > 32 || ssid.chars().any(|c| c.is_control()) {
        return Err("Invalid SSID (maximum 32 bytes)".into());
    }
    let valid = ((8..=63).contains(&password.len()) && password.is_ascii()
        && !password.chars().any(|c| c.is_control()))
        || (password.len() == 64 && password.bytes().all(|b| b.is_ascii_hexdigit()));
    if !valid { return Err("Invalid WPA password".into()); }

    let profile = format!("stephania-{}", hex_ssid(ssid));
    let existing = Command::new("nmcli")
        .args(["-g", "connection.id", "connection", "show", &profile])
        .stdout(Stdio::null()).stderr(Stdio::null()).status();
    if !matches!(existing, Ok(s) if s.success()) {
        let result = Command::new("nmcli").args([
            "connection", "add", "type", "wifi", "ifname", "wlan0",
            "con-name", &profile, "ssid", ssid, "wifi-sec.key-mgmt", "wpa-psk",
        ]).output().map_err(|e| e.to_string())?;
        if !result.status.success() { return Err("Could not add Wi-Fi profile".into()); }
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?.as_nanos();
    let path = format!("/run/stephania-wifi-{now}-{}", std::process::id());
    let mut file = OpenOptions::new().create_new(true).write(true).mode(0o600)
        .open(&path).map_err(|e| e.to_string())?;
    file.write_all(format!("802-11-wireless-security.psk:{password}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    drop(file);
    let result = Command::new("nmcli")
        .args(["--wait", "25", "connection", "up", &profile, "passwd-file", &path])
        .output();
    let _ = fs::remove_file(&path);
    match result {
        Ok(out) if out.status.success() => Ok(()),
        Ok(_) => Err("Could not activate Wi-Fi profile".into()),
        Err(e) => Err(e.to_string())
    }
}

fn hex_ssid(value: &str) -> String {
    value.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_is_stable_and_distinct() {
        assert_eq!(hex_ssid("LT"), "4c54");
        assert_ne!(hex_ssid("AB"), hex_ssid("BA"));
    }
    #[test]
    fn bad_params_rejected_without_device_access() {
        assert!(connect("", "abcdefgh").is_err());
        assert!(connect("a", "bad").is_err());
        assert!(connect("a\nb", "abcdefgh").is_err());
    }
}
