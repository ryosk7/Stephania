//! T3Box provisioning service.
//!
//! Owns the ownership state and Wi-Fi provisioning logic and exposes GATT
//! characteristic semantics on a local Unix socket
//! (`/run/t3box/provisioning.sock`, docs/IPC.md framing). The BlueZ GATT
//! adapter is a separate thin process that forwards characteristic reads and
//! writes here; it is not implemented yet.
//!
//! Methods (protocol v2, contracts/ble-protocol.md):
//! `gatt.request {peer, fragment_hex}` → `{response_hex|null, message_id}`;
//! the GATT adapter indicates `response_hex` fragmented under `message_id`.
//! `gatt.disconnect {peer}` drops that peer's partial transfer. Physical reset is intentionally not
//! reachable over the socket; it will come from the GPIO handler.
#[cfg(unix)]
fn main() {
    if let Err(e) = unix::run() {
        eprintln!("t3box-provision: {e}");
        std::process::exit(2);
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("t3box-provision requires a Unix platform");
    std::process::exit(2);
}

#[cfg(unix)]
mod unix {
    use serde_json::{json, Value};
    use std::path::PathBuf;
    use std::time::Instant;
    use t3box::ipc::{self, PeerPolicy};
    use t3box::nm::{NetworkManagerBackend, Nmcli};
    use t3box::provisioning::{hex, import_setup_file, load_or_create_setup_secret, OwnershipManager, SetupImport};
    use t3box::provisioning_gatt::{FileOwnershipStore, NetworkBackend, OwnershipStore, RpcService, ScanResult};

    fn env_path(key: &str, default: &str) -> PathBuf {
        std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| default.into())
    }

    /// Deterministic network for the virtual lab only.
    #[cfg(feature = "ci-virtual")]
    struct VirtualNetwork;
    #[cfg(feature = "ci-virtual")]
    impl NetworkBackend for VirtualNetwork {
        fn scan(&mut self) -> Result<Vec<ScanResult>, String> {
            Ok(vec![ScanResult { ssid: "virtual-lab".into(), rssi: -40, security: "wpa-psk".into() }])
        }
        fn connect(&mut self, config: &t3box::provisioning::WifiConfiguration) -> Result<(), String> {
            if config.ssid == b"virtual-lab" { Ok(()) } else { Err("not found".into()) }
        }
    }

    pub fn run() -> Result<(), String> {
        let socket = env_path("T3BOX_PROVISION_SOCKET", "/run/t3box/provisioning.sock");
        let credentials = env_path("T3BOX_CREDENTIALS_DIR", "/data/t3box/credentials");
        let policy = PeerPolicy::from_env()?;
        let store = FileOwnershipStore { path: credentials.join("ownership.json") };
        let state = store.load();
        // First boot: adopt the browser-generated secret copied to the boot
        // partition (contracts/ble-protocol.md "Setup secret delivery").
        let setup_file = env_path("T3BOX_SETUP_FILE", "/boot/firmware/t3box-setup.txt");
        let secret_path = credentials.join("setup-secret");
        match import_setup_file(&setup_file, &secret_path, &state).map_err(|e| format!("setup file import: {e}"))? {
            SetupImport::Absent => {}
            SetupImport::Imported => eprintln!("t3box-provision: setup secret imported from boot partition"),
            SetupImport::IgnoredClaimed => eprintln!("t3box-provision: setup file ignored (device already owned) and removed"),
            SetupImport::Invalid => eprintln!("t3box-provision: invalid setup file removed"),
        }
        let secret = load_or_create_setup_secret(&secret_path).map_err(|e| format!("setup secret: {e}"))?;
        let ownership = OwnershipManager::new(secret, state);
        eprintln!("t3box-provision: ownership {}", ownership.reported_state());
        let backend = std::env::var("T3BOX_NETWORK_BACKEND").unwrap_or_else(|_| "nm".into());
        match backend.as_str() {
            // Default: NetworkManager over D-Bus (stores the PSK for reboot
            // reconnection). `nmcli` remains as a fallback backend.
            #[cfg(feature = "networkmanager")]
            "nm" => {
                let iface = std::env::var("T3BOX_WIFI_INTERFACE").unwrap_or_else(|_| "wlan0".into());
                let backend = t3box::nm_dbus::NetworkManagerDbus::new(&iface)?;
                serve(&socket, &policy, RpcService::new(ownership, backend, store))
            }
            "nmcli" => {
                let secrets_dir = env_path("T3BOX_PROVISION_RUNTIME", "/run/t3box/provision");
                std::fs::create_dir_all(&secrets_dir).map_err(|e| format!("runtime dir: {e}"))?;
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&secrets_dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("runtime dir: {e}"))?;
                let iface = std::env::var("T3BOX_WIFI_INTERFACE").unwrap_or_else(|_| "wlan0".into());
                serve(&socket, &policy, RpcService::new(ownership, NetworkManagerBackend { runner: Nmcli, interface: iface, secrets_dir }, store))
            }
            #[cfg(feature = "ci-virtual")]
            "virtual" => serve(&socket, &policy, RpcService::new(ownership, VirtualNetwork, store)),
            other => Err(format!("unsupported T3BOX_NETWORK_BACKEND: {other}")),
        }
    }

    fn unhex(s: &str) -> Option<Vec<u8>> {
        if !s.len().is_multiple_of(2) || !s.is_ascii() { return None; }
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
    }

    fn valid_peer(p: &str) -> bool {
        (1..=64).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'_' | b'.'))
    }

    fn handle<N: NetworkBackend, S: OwnershipStore>(service: &mut RpcService<N, S>, frame: &[u8]) -> Value {
        let err = |id: Option<&str>, code: &str, msg: &str| json!({"version":1,"request_id":id,"ok":false,"error":{"code":code,"message":msg}});
        let Ok(Value::Object(req)) = serde_json::from_slice::<Value>(frame) else { return err(None, "invalid_request", "not a JSON object") };
        let id = req.get("request_id").and_then(Value::as_str).filter(|s| (1..=64).contains(&s.len()));
        let Some(id) = id else { return err(None, "invalid_request", "invalid request_id") };
        if req.get("version") != Some(&json!(1)) { return err(Some(id), "unsupported_version", "version must be 1"); }
        let params = req.get("params").and_then(Value::as_object);
        let Some(peer) = params.and_then(|p| p.get("peer")).and_then(Value::as_str).filter(|p| valid_peer(p)) else {
            return err(Some(id), "invalid_params", "peer required");
        };
        let result = match req.get("method").and_then(Value::as_str) {
            Some("gatt.request") => {
                let Some(fragment) = params.and_then(|p| p.get("fragment_hex")).and_then(Value::as_str).and_then(unhex).filter(|f| !f.is_empty()) else {
                    return err(Some(id), "invalid_params", "fragment_hex required");
                };
                let message_id = fragment[0];
                let response = service.handle_fragment(peer, &fragment, Instant::now());
                json!({"response_hex": response.map(|b| hex(&b)), "message_id": message_id})
            }
            Some("gatt.disconnect") => { service.disconnect(peer); json!({}) }
            _ => return err(Some(id), "unknown_method", "unknown method"),
        };
        json!({"version":1,"request_id":id,"ok":true,"result":result})
    }

    /// One connection at a time: provisioning is low-rate and the service is
    /// single-owner state, so serial handling is the simplest bounded design.
    fn serve<N: NetworkBackend, S: OwnershipStore>(socket: &std::path::Path, policy: &PeerPolicy, mut service: RpcService<N, S>) -> Result<(), String> {
        let listener = ipc::bind(socket).map_err(|e| format!("bind {}: {e}", socket.display()))?;
        eprintln!("t3box-provision: listening on {}", socket.display());
        let own = unsafe_getuid();
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
                let body = serde_json::to_vec(&handle(&mut service, &frame)).unwrap_or_default();
                if ipc::write_frame(&mut stream, &body).is_err() { break; }
            }
        }
        Ok(())
    }

    fn unsafe_getuid() -> u32 {
        // SAFETY: getuid has no preconditions and cannot fail.
        unsafe { libc::getuid() }
    }
}
