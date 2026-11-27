//! NetworkManager backend over D-Bus (pure-Rust zbus) for provisioning.
//!
//! Unlike the `nmcli` backend, this stores the PSK in NetworkManager's
//! system-owned profile (root-only keyfile), so the appliance reconnects on
//! its own after a reboot or an access-point outage.
//!
//! Join semantics, chosen so a failed attempt never breaks a working setup:
//! 1. add a fresh profile `stephania-<hex ssid>` (autoconnect, DHCP);
//! 2. activate it on the Wi-Fi device and wait for ACTIVATED (≤ 35 s);
//! 3. on success delete older profiles with the same id; on failure delete
//!    only the new profile, leaving any previous profile untouched.
//!
//! The PSK crosses only the local system bus to NetworkManager and is never
//! logged; errors are reduced to fixed strings.

use crate::provisioning::{hex, WifiConfiguration, WifiSecurity};
use crate::provisioning_gatt::{NetworkBackend, ScanResult};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use zbus::blocking::{proxy::Builder, Connection, Proxy};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const DEVICE_TYPE_WIFI: u32 = 2;
const ACTIVE_STATE_ACTIVATED: u32 = 2;
const ACTIVE_STATE_DEACTIVATED: u32 = 4;
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(35);
const SCAN_WAIT: Duration = Duration::from_secs(10);

// NM80211ApFlags / NM80211ApSecurityFlags
const AP_FLAGS_PRIVACY: u32 = 0x1;
const SEC_KEY_MGMT_PSK: u32 = 0x100;
const SEC_KEY_MGMT_8021X: u32 = 0x200;
const SEC_KEY_MGMT_SAE: u32 = 0x400;

/// Map access-point flags to the protocol's security names.
pub fn security_name(flags: u32, wpa_flags: u32, rsn_flags: u32) -> &'static str {
    let sec = wpa_flags | rsn_flags;
    if sec & SEC_KEY_MGMT_8021X != 0 {
        "enterprise"
    } else if sec & (SEC_KEY_MGMT_PSK | SEC_KEY_MGMT_SAE) != 0 {
        "wpa-psk"
    } else if flags & AP_FLAGS_PRIVACY != 0 {
        "unsupported" // WEP or unknown privacy scheme
    } else if sec == 0 {
        "open"
    } else {
        "unsupported" // e.g. OWE-only
    }
}

/// NetworkManager signal strength (0–100 %) to an approximate dBm.
pub fn strength_to_rssi(strength: u8) -> i32 {
    i32::from(strength.min(100)) / 2 - 100
}

pub fn profile_id(ssid: &[u8]) -> String {
    format!("stephania-{}", hex(ssid))
}

fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

type Settings<'a> = HashMap<&'static str, HashMap<&'static str, Value<'a>>>;

/// Connection settings for AddConnection. Public for unit tests.
pub fn connection_settings<'a>(config: &'a WifiConfiguration, interface: &'a str, uuid: &'a str) -> Settings<'a> {
    let mut s: Settings = HashMap::new();
    s.insert("connection", HashMap::from([
        ("id", Value::from(profile_id(&config.ssid))),
        ("uuid", Value::from(uuid)),
        ("type", Value::from("802-11-wireless")),
        ("interface-name", Value::from(interface)),
        ("autoconnect", Value::from(true)),
    ]));
    s.insert("802-11-wireless", HashMap::from([
        ("ssid", Value::from(config.ssid.clone())),
        ("mode", Value::from("infrastructure")),
    ]));
    if let WifiSecurity::WpaPsk(psk) = &config.security {
        // psk-flags 0 (default) = system-owned: stored by NM, root-only.
        s.insert("802-11-wireless-security", HashMap::from([
            ("key-mgmt", Value::from("wpa-psk")),
            ("psk", Value::from(psk.as_str())),
        ]));
    }
    s.insert("ipv4", HashMap::from([("method", Value::from("auto"))]));
    s.insert("ipv6", HashMap::from([("method", Value::from("auto"))]));
    s
}

pub struct NetworkManagerDbus {
    conn: Connection,
    pub interface: String,
}

fn proxy<'a>(conn: &Connection, path: &'a str, iface: &'a str) -> Result<Proxy<'a>, String> {
    Builder::<Proxy>::new(conn)
        .destination(NM).map_err(|_| "dbus")?
        .path(path).map_err(|_| "dbus path")?
        .interface(iface).map_err(|_| "dbus")?
        .cache_properties(CacheProperties::No)
        .build()
        .map_err(|_| "NetworkManager unavailable".to_string())
}

impl NetworkManagerDbus {
    pub fn new(interface: &str) -> Result<Self, String> {
        let conn = Connection::system().map_err(|_| "system bus unavailable")?;
        Ok(Self { conn, interface: interface.to_owned() })
    }

    fn device(&self) -> Result<OwnedObjectPath, String> {
        let nm = proxy(&self.conn, NM_PATH, NM)?;
        let devices: Vec<OwnedObjectPath> = nm.call("GetDevices", &()).map_err(|_| "GetDevices failed")?;
        for d in devices {
            let dev = proxy(&self.conn, d.as_str(), "org.freedesktop.NetworkManager.Device")?;
            let ty: u32 = dev.get_property("DeviceType").unwrap_or_default();
            let name: String = dev.get_property("Interface").unwrap_or_default();
            drop(dev);
            if ty == DEVICE_TYPE_WIFI && name == self.interface {
                return Ok(d);
            }
        }
        Err("wifi device not found".into())
    }

    fn profiles_with_id(&self, id: &str) -> Result<Vec<OwnedObjectPath>, String> {
        let settings = proxy(&self.conn, "/org/freedesktop/NetworkManager/Settings", "org.freedesktop.NetworkManager.Settings")?;
        let list: Vec<OwnedObjectPath> = settings.call("ListConnections", &()).map_err(|_| "ListConnections failed")?;
        let mut out = Vec::new();
        for c in list {
            let conn = proxy(&self.conn, c.as_str(), "org.freedesktop.NetworkManager.Settings.Connection")?;
            let s: HashMap<String, HashMap<String, OwnedValue>> = match conn.call("GetSettings", &()) { Ok(s) => s, Err(_) => continue };
            drop(conn);
            let this_id = s.get("connection").and_then(|c| c.get("id")).and_then(|v| String::try_from(v.clone()).ok());
            if this_id.as_deref() == Some(id) { out.push(c); }
        }
        Ok(out)
    }

    fn delete(&self, path: &OwnedObjectPath) {
        if let Ok(p) = proxy(&self.conn, path.as_str(), "org.freedesktop.NetworkManager.Settings.Connection") {
            let _: Result<(), _> = p.call("Delete", &());
        }
    }

    fn wait_active(&self, active: &OwnedObjectPath) -> Result<(), String> {
        let deadline = Instant::now() + JOIN_TIMEOUT;
        while Instant::now() < deadline {
            let p = proxy(&self.conn, active.as_str(), "org.freedesktop.NetworkManager.Connection.Active")?;
            match p.get_property::<u32>("State") {
                Ok(ACTIVE_STATE_ACTIVATED) => return Ok(()),
                Ok(ACTIVE_STATE_DEACTIVATED) | Err(_) => return Err("activation failed".into()), // object gone = failed
                Ok(_) => std::thread::sleep(Duration::from_millis(250)),
            }
        }
        Err("activation timed out".into())
    }
}

impl NetworkBackend for NetworkManagerDbus {
    fn scan(&mut self) -> Result<Vec<ScanResult>, String> {
        let device = self.device()?;
        let wifi = proxy(&self.conn, device.as_str(), "org.freedesktop.NetworkManager.Device.Wireless")?;
        let before: i64 = wifi.get_property("LastScan").unwrap_or(-1);
        // May fail with "scan already in progress"/rate limiting: then just
        // use the most recent results.
        let requested = wifi.call::<_, _, ()>("RequestScan", &(HashMap::<&str, Value>::new(),)).is_ok();
        if requested {
            let deadline = Instant::now() + SCAN_WAIT;
            while Instant::now() < deadline && wifi.get_property::<i64>("LastScan").unwrap_or(before) == before {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        let aps: Vec<OwnedObjectPath> = wifi.call("GetAllAccessPoints", &()).map_err(|_| "GetAllAccessPoints failed")?;
        let mut out = Vec::new();
        for ap in aps {
            let p = proxy(&self.conn, ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint")?;
            let ssid: Vec<u8> = p.get_property("Ssid").unwrap_or_default();
            let Ok(ssid) = String::from_utf8(ssid) else { continue }; // non-UTF-8 SSIDs cannot be represented in JSON text
            let strength: u8 = p.get_property("Strength").unwrap_or(0);
            let flags: u32 = p.get_property("Flags").unwrap_or(0);
            let wpa: u32 = p.get_property("WpaFlags").unwrap_or(0);
            let rsn: u32 = p.get_property("RsnFlags").unwrap_or(0);
            out.push(ScanResult { ssid, rssi: strength_to_rssi(strength), security: security_name(flags, wpa, rsn).into() });
        }
        Ok(out)
    }

    fn connect(&mut self, config: &WifiConfiguration) -> Result<(), String> {
        let device = self.device()?;
        let id = profile_id(&config.ssid);
        let previous = self.profiles_with_id(&id)?;
        let uuid = uuid_v4();
        let settings_proxy = proxy(&self.conn, "/org/freedesktop/NetworkManager/Settings", "org.freedesktop.NetworkManager.Settings")?;
        let added: OwnedObjectPath = settings_proxy
            .call("AddConnection", &(connection_settings(config, &self.interface, &uuid),))
            .map_err(|_| "AddConnection failed")?;
        let nm = proxy(&self.conn, NM_PATH, NM)?;
        let root = OwnedObjectPath::try_from("/").expect("root path");
        let activated: Result<OwnedObjectPath, _> = nm.call("ActivateConnection", &(&added, &device, &root));
        let result = match activated {
            Ok(active) => self.wait_active(&active),
            Err(_) => Err("activation failed".into()),
        };
        match result {
            Ok(()) => { for p in previous { self.delete(&p); } Ok(()) }
            Err(e) => { self.delete(&added); Err(e) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_mapping() {
        assert_eq!(security_name(0, 0, 0), "open");
        assert_eq!(security_name(1, 0, SEC_KEY_MGMT_PSK), "wpa-psk");
        assert_eq!(security_name(1, SEC_KEY_MGMT_PSK, 0), "wpa-psk");
        assert_eq!(security_name(1, 0, SEC_KEY_MGMT_SAE), "wpa-psk");
        assert_eq!(security_name(1, 0, SEC_KEY_MGMT_8021X), "enterprise");
        assert_eq!(security_name(1, 0, 0), "unsupported");
        assert_eq!(security_name(0, 0, 0x800), "unsupported");
    }

    #[test]
    fn strength_mapping() {
        assert_eq!(strength_to_rssi(100), -50);
        assert_eq!(strength_to_rssi(0), -100);
        assert_eq!(strength_to_rssi(255), -50);
    }

    #[test]
    fn settings_store_psk_system_owned_and_autoconnect() {
        let cfg = WifiConfiguration { ssid: b"LT Hall".to_vec(), security: WifiSecurity::WpaPsk("correct horse".into()) };
        let s = connection_settings(&cfg, "wlan0", "u");
        assert_eq!(s["connection"]["id"], Value::from("stephania-4c542048616c6c"));
        assert_eq!(s["connection"]["autoconnect"], Value::from(true));
        assert_eq!(s["802-11-wireless"]["ssid"], Value::from(b"LT Hall".to_vec()));
        assert_eq!(s["802-11-wireless-security"]["psk"], Value::from("correct horse"));
        assert!(!s["802-11-wireless-security"].contains_key("psk-flags"), "default 0 = stored by NM");
        let open = WifiConfiguration { ssid: b"Open".to_vec(), security: WifiSecurity::Open };
        assert!(!connection_settings(&open, "wlan0", "u").contains_key("802-11-wireless-security"));
    }

    #[test]
    fn uuid_format() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert_ne!(u, uuid_v4());
    }
}
