//! NetworkManager backend for provisioning via `nmcli`.
//!
//! Commands go through [`CommandRunner`] so parsing and argument
//! construction are unit-tested without NetworkManager. PSKs are passed via
//! a 0600 `passwd-file` in a private runtime directory, never on the command
//! line (visible in /proc) and never logged.
//!
//! Limitation: with `passwd-file` the PSK is used for this activation only
//! and is not stored in the profile, so automatic reconnection after reboot
//! needs the planned D-Bus adapter (AddConnection with secrets). Tracked in
//! contracts/ble-protocol.md open items.

use crate::provisioning::{WifiConfiguration, WifiSecurity};
use crate::provisioning_gatt::{NetworkBackend, ScanResult};
use std::path::PathBuf;

pub struct Output {
    pub success: bool,
    pub stdout: String,
}

pub trait CommandRunner {
    fn run(&mut self, args: &[String]) -> Result<Output, String>;
}

/// Runs the real `nmcli`.
pub struct Nmcli;

impl CommandRunner for Nmcli {
    fn run(&mut self, args: &[String]) -> Result<Output, String> {
        let out = std::process::Command::new("nmcli").args(args)
            .stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .output().map_err(|e| format!("nmcli unavailable: {}", e.kind()))?;
        Ok(Output { success: out.status.success(), stdout: String::from_utf8_lossy(&out.stdout).into_owned() })
    }
}

/// Split one `nmcli -t` line on unescaped ':' and unescape `\:` / `\\`.
pub fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => if let Some(n) = chars.next() { fields.last_mut().expect("non-empty").push(n) },
            ':' => fields.push(String::new()),
            c => fields.last_mut().expect("non-empty").push(c),
        }
    }
    fields
}

/// Parse `nmcli -t -f SSID,SIGNAL,SECURITY device wifi list` output.
/// SIGNAL (0–100 %) is mapped to an approximate dBm for the BLE contract.
pub fn parse_scan(stdout: &str) -> Vec<ScanResult> {
    stdout.lines().filter_map(|line| {
        let f = split_terse(line);
        if f.len() != 3 || f[0].is_empty() || f[0].len() > 32 || f[0].chars().any(char::is_control) {
            return None;
        }
        let signal: i32 = f[1].trim().parse().ok().filter(|s| (0..=100).contains(s))?;
        let sec = f[2].trim();
        let security = if sec.is_empty() || sec == "--" { "open" }
            else if sec.contains("802.1X") || sec.contains("EAP") { "enterprise" }
            else if sec.contains("WPA") || sec.contains("RSN") || sec.contains("SAE") { "wpa-psk" }
            else { "unsupported" };
        Some(ScanResult { ssid: f[0].clone(), rssi: signal / 2 - 100, security: security.into() })
    }).collect()
}

fn profile_name(ssid: &[u8]) -> String {
    format!("stephania-{}", crate::provisioning::hex(ssid))
}

pub struct NetworkManagerBackend<R: CommandRunner> {
    pub runner: R,
    pub interface: String,
    /// Private directory (0700) for transient passwd files, e.g. /run/stephania/provision.
    pub secrets_dir: PathBuf,
}

impl<R: CommandRunner> NetworkManagerBackend<R> {
    fn run(&mut self, args: &[&str]) -> Result<Output, String> {
        self.runner.run(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn write_secret(&self, psk: &str) -> Result<PathBuf, String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut random = [0u8; 8];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random).map_err(|_| "random unavailable")?;
        let path = self.secrets_dir.join(format!("wifi-{}", crate::provisioning::hex(&random)));
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)
            .map_err(|e| format!("secret file: {}", e.kind()))?;
        f.write_all(format!("802-11-wireless-security.psk:{psk}\n").as_bytes()).map_err(|e| format!("secret file: {}", e.kind()))?;
        Ok(path)
    }
}

impl<R: CommandRunner> NetworkBackend for NetworkManagerBackend<R> {
    fn scan(&mut self) -> Result<Vec<ScanResult>, String> {
        let iface = self.interface.clone();
        let out = self.run(&["-t", "-f", "SSID,SIGNAL,SECURITY", "device", "wifi", "list", "ifname", &iface, "--rescan", "yes"])?;
        if !out.success {
            return Err("scan failed".into());
        }
        Ok(parse_scan(&out.stdout))
    }

    fn connect(&mut self, config: &WifiConfiguration) -> Result<(), String> {
        let ssid = std::str::from_utf8(&config.ssid).map_err(|_| "ssid not utf-8")?.to_owned();
        let profile = profile_name(&config.ssid);
        let iface = self.interface.clone();
        // Recreate the profile so security settings always match the request.
        let _ = self.run(&["connection", "delete", &profile]);
        let mut add = vec!["connection", "add", "type", "wifi", "ifname", &iface, "con-name", &profile, "ssid", &ssid,
            "connection.autoconnect", "yes"];
        if matches!(config.security, WifiSecurity::WpaPsk(_)) {
            add.extend(["wifi-sec.key-mgmt", "wpa-psk"]);
        }
        if !self.run(&add)?.success {
            return Err("profile add failed".into());
        }
        let result = match &config.security {
            WifiSecurity::Open => self.run(&["--wait", "30", "connection", "up", &profile]),
            WifiSecurity::WpaPsk(psk) => {
                let path = self.write_secret(psk)?;
                let path_str = path.to_string_lossy().into_owned();
                let r = self.run(&["--wait", "30", "connection", "up", &profile, "passwd-file", &path_str]);
                let _ = std::fs::remove_file(&path);
                r
            }
        }?;
        if result.success { Ok(()) } else { Err("activation failed".into()) }
    }

    fn wired(&mut self) -> bool {
        self.run(&["-t", "-f", "TYPE,STATE", "device"])
            .is_ok_and(|o| o.success && parse_wired(&o.stdout))
    }
}

/// `nmcli -t -f TYPE,STATE device`: any Ethernet device fully connected.
pub fn parse_wired(stdout: &str) -> bool {
    stdout.lines().any(|l| l.split_once(':').is_some_and(|(t, s)| t == "ethernet" && s == "connected"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder { calls: Vec<Vec<String>>, fail_on: Option<&'static str>, stdout: String, secret_seen: Option<String> }
    impl CommandRunner for Recorder {
        fn run(&mut self, args: &[String]) -> Result<Output, String> {
            if let Some(i) = args.iter().position(|a| a == "passwd-file") {
                self.secret_seen = std::fs::read_to_string(&args[i + 1]).ok();
            }
            self.calls.push(args.to_vec());
            let fail = self.fail_on.is_some_and(|f| args.iter().any(|a| a == f));
            Ok(Output { success: !fail, stdout: self.stdout.clone() })
        }
    }

    fn backend(r: Recorder) -> (NetworkManagerBackend<Recorder>, PathBuf) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("stephania-nm-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (NetworkManagerBackend { runner: r, interface: "wlan0".into(), secrets_dir: dir.clone() }, dir)
    }

    #[test]
    fn terse_escaping() {
        assert_eq!(split_terse(r"a\:b:50:WPA2"), vec!["a:b", "50", "WPA2"]);
        assert_eq!(split_terse(r"back\\slash:1:"), vec![r"back\slash", "1", ""]);
    }

    #[test]
    fn scan_parsing_and_security_mapping() {
        let out = "Hall:80:WPA2\nCafe\\:Free:40:\nCorp:60:WPA2 802.1X\n:90:WPA2\nBad:abc:WPA2\nWEP:10:WEP\nNew:100:WPA3 SAE\n";
        let r = parse_scan(out);
        assert_eq!(r.iter().map(|n| (n.ssid.as_str(), n.rssi, n.security.as_str())).collect::<Vec<_>>(), vec![
            ("Hall", -60, "wpa-psk"), ("Cafe:Free", -80, "open"), ("Corp", -70, "enterprise"),
            ("WEP", -95, "unsupported"), ("New", -50, "wpa-psk"),
        ]);
    }

    #[test]
    fn psk_never_on_command_line_and_secret_file_removed() {
        let (mut b, dir) = backend(Recorder::default());
        let config = WifiConfiguration { ssid: b"LT Hall".to_vec(), security: WifiSecurity::WpaPsk("correct horse".into()) };
        b.connect(&config).unwrap();
        assert!(b.runner.calls.iter().flatten().all(|a| !a.contains("correct horse")));
        assert_eq!(b.runner.secret_seen.as_deref(), Some("802-11-wireless-security.psk:correct horse\n"));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "secret file removed");
        let add = b.runner.calls.iter().find(|c| c.get(1).map(String::as_str) == Some("add")).unwrap();
        assert!(add.contains(&"stephania-4c542048616c6c".to_string()));
        assert!(add.contains(&"wpa-psk".to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn open_network_and_failures() {
        let (mut b, dir) = backend(Recorder::default());
        b.connect(&WifiConfiguration { ssid: b"Open".to_vec(), security: WifiSecurity::Open }).unwrap();
        let add = b.runner.calls.iter().find(|c| c.get(1).map(String::as_str) == Some("add")).unwrap();
        assert!(!add.contains(&"wpa-psk".to_string()));
        let (mut f, dir2) = backend(Recorder { fail_on: Some("up"), ..Default::default() });
        let config = WifiConfiguration { ssid: b"X".to_vec(), security: WifiSecurity::WpaPsk("password".into()) };
        assert_eq!(f.connect(&config), Err("activation failed".into()));
        assert_eq!(std::fs::read_dir(&dir2).unwrap().count(), 0, "secret removed on failure too");
        let (mut s, dir3) = backend(Recorder { fail_on: Some("list"), ..Default::default() });
        assert!(s.scan().is_err());
        for d in [dir, dir2, dir3] { let _ = std::fs::remove_dir_all(d); }
    }

    #[test]
    fn wired_means_a_connected_ethernet_device() {
        assert!(parse_wired("wifi:disconnected\nethernet:connected\nloopback:connected (externally)\n"));
        assert!(!parse_wired("wifi:connected\nethernet:unavailable\n"));
        assert!(!parse_wired("ethernet:connecting (getting IP configuration)\n"));
        assert!(!parse_wired(""));
    }
}
