//! What stephctl remembers. `devices.toml` in the config directory holds
//! each device's name, addresses, port and pinned CA (no secrets). Studio
//! sessions are kept in the OS credential store (macOS Keychain, Windows
//! Credential Manager, the Linux kernel keyring). Where no credential
//! store works, or with `STEPHCTL_SESSION_STORE=file`, they go to a 0600
//! file next to `devices.toml` instead.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Device {
    /// Short name used on the command line (the device's host name).
    pub name: String,
    /// `<host>.local`.
    pub host: String,
    #[serde(default)]
    pub addresses: Vec<String>,
    pub port: u16,
    /// SHA-256 of the CA certificate (DER), lower-case hex.
    pub ca_sha256: String,
    /// The pinned CA certificate (PEM, public).
    pub ca_pem: String,
}

#[derive(Default, Serialize, Deserialize)]
struct DevicesFile {
    #[serde(default, rename = "device")]
    devices: Vec<Device>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub cookie: String,
    pub csrf_token: String,
    pub expires_at_unix: u64,
    pub username: String,
}

pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("STEPHCTL_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if cfg!(windows) {
        if let Some(app) = std::env::var_os("APPDATA") {
            return PathBuf::from(app).join("stephctl");
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("stephctl");
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_default();
    PathBuf::from(home).join(".config").join("stephctl")
}

fn devices_path() -> PathBuf { config_dir().join("devices.toml") }

pub fn load_devices() -> Result<Vec<Device>, String> {
    match std::fs::read_to_string(devices_path()) {
        Ok(text) => toml::from_str::<DevicesFile>(&text).map(|f| f.devices).map_err(|e| format!("{}: {e}", devices_path().display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", devices_path().display())),
    }
}

pub fn save_devices(devices: &[Device]) -> Result<(), String> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = toml::to_string_pretty(&DevicesFile { devices: devices.to_vec() }).map_err(|e| e.to_string())?;
    let tmp = dir.join("devices.toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, devices_path()).map_err(|e| format!("{}: {e}", devices_path().display()))
}

pub fn upsert_device(device: Device) -> Result<(), String> {
    let mut all = load_devices()?;
    all.retain(|d| d.name != device.name);
    all.push(device);
    all.sort_by(|a, b| a.name.cmp(&b.name));
    save_devices(&all)
}

pub fn remove_device(name: &str) -> Result<bool, String> {
    let mut all = load_devices()?;
    let before = all.len();
    all.retain(|d| d.name != name);
    save_devices(&all)?;
    Ok(all.len() != before)
}

// --- Sessions -------------------------------------------------------------

const SERVICE: &str = "stephctl";

fn file_store() -> bool {
    std::env::var("STEPHCTL_SESSION_STORE").map(|v| v == "file").unwrap_or(false)
}

fn session_file(name: &str) -> PathBuf { config_dir().join("sessions").join(format!("{name}.json")) }

fn write_private(path: &PathBuf, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options.open(path).and_then(|mut f| f.write_all(text.as_bytes())).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn save_session(name: &str, session: &Session) -> Result<(), String> {
    let text = serde_json::to_string(session).map_err(|e| e.to_string())?;
    if !file_store() {
        match keyring::Entry::new(SERVICE, name).and_then(|e| e.set_password(&text)) {
            Ok(()) => {
                let _ = std::fs::remove_file(session_file(name));
                return Ok(());
            }
            Err(e) => eprintln!("stephctl: credential store unavailable ({e}); keeping the session in a private file"),
        }
    }
    write_private(&session_file(name), &text)
}

pub fn load_session(name: &str) -> Option<Session> {
    let text = if file_store() {
        None
    } else {
        keyring::Entry::new(SERVICE, name).and_then(|e| e.get_password()).ok()
    };
    let text = text.or_else(|| std::fs::read_to_string(session_file(name)).ok())?;
    serde_json::from_str(&text).ok()
}

pub fn delete_session(name: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE, name) {
        let _ = entry.delete_credential();
    }
    let _ = std::fs::remove_file(session_file(name));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_round_trip_through_toml() {
        let file = DevicesFile { devices: vec![Device {
            name: "stephania-8f21".into(), host: "stephania-8f21.local".into(), addresses: vec!["192.168.1.20".into()],
            port: 9443, ca_sha256: "ab".repeat(32), ca_pem: "-----BEGIN CERTIFICATE-----\nAA==\n-----END CERTIFICATE-----\n".into(),
        }] };
        let text = toml::to_string_pretty(&file).unwrap();
        assert!(text.contains("[[device]]"));
        let back: DevicesFile = toml::from_str(&text).unwrap();
        assert_eq!(back.devices, file.devices);
    }
}
