//! GATT characteristic semantics for provisioning (contracts/ble-protocol.md).
//!
//! [`GattService`] is the whole device-side behaviour behind the BLE
//! characteristics: reads return complete JSON messages (the transport
//! fragments them with [`crate::provisioning::fragment`]), writes arrive as
//! fragments and are reassembled per characteristic. A BlueZ D-Bus adapter
//! only has to map characteristic UUIDs to [`Characteristic`] and forward
//! bytes; it is not implemented yet.
//!
//! Network changes go through [`NetworkBackend`] (NetworkManager in
//! production, fakes in tests). Ownership changes are persisted through
//! [`OwnershipStore`] *before* they are acknowledged.

use crate::provisioning::{
    hex, parse_wifi_configuration, unhex, ClaimError, FrameError, NetworkState, Ownership, OwnershipManager,
    Reassembler, WifiConfiguration, NONCE_BYTES,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io;
use std::time::Instant;

pub const PROTOCOL_VERSION: u64 = 1;
pub const MAX_SCAN_RESULTS: usize = 60;
pub const SCAN_PAGE_SIZE: usize = 20;
pub const SERVICE_UUID: &str = "7c3b0001-5d1e-4a6f-9b0e-3a1f2c4d5e60";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Characteristic {
    ProtocolVersion,
    DeviceInformation,
    PairingChallenge,
    PairingResponse,
    WifiScanRequest,
    WifiScanResults,
    WifiConfiguration,
    NetworkStatus,
    ManagementEndpoint,
    DevicePublicKey,
    OperationChallenge,
}

impl Characteristic {
    pub const ALL: [Characteristic; 11] = [
        Self::ProtocolVersion, Self::DeviceInformation, Self::PairingChallenge, Self::PairingResponse,
        Self::WifiScanRequest, Self::WifiScanResults, Self::WifiConfiguration, Self::NetworkStatus,
        Self::ManagementEndpoint, Self::DevicePublicKey, Self::OperationChallenge,
    ];

    fn suffix(self) -> u16 {
        match self {
            Self::ProtocolVersion => 0x0002, Self::DeviceInformation => 0x0003, Self::PairingChallenge => 0x0004,
            Self::PairingResponse => 0x0005, Self::WifiScanRequest => 0x0006, Self::WifiScanResults => 0x0007,
            Self::WifiConfiguration => 0x0008, Self::NetworkStatus => 0x0009, Self::ManagementEndpoint => 0x000a,
            Self::DevicePublicKey => 0x000b, Self::OperationChallenge => 0x000c,
        }
    }

    pub fn uuid(self) -> String {
        format!("7c3b{:04x}-5d1e-4a6f-9b0e-3a1f2c4d5e60", self.suffix())
    }

    pub fn from_uuid(uuid: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.uuid().eq_ignore_ascii_case(uuid))
    }

    pub fn writable(self) -> bool {
        matches!(self, Self::PairingResponse | Self::WifiScanRequest | Self::WifiConfiguration)
    }

    /// Operation name an authenticated write must carry.
    fn operation(self) -> Option<&'static str> {
        match self {
            Self::WifiScanRequest => Some("wifi.scan"),
            Self::WifiConfiguration => Some("wifi.configure"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    pub ssid: String,
    pub rssi: i32,
    pub security: String,
}

pub trait NetworkBackend {
    fn scan(&mut self) -> Result<Vec<ScanResult>, String>;
    /// Join the network. Implementations must not log the PSK.
    fn connect(&mut self, config: &WifiConfiguration) -> Result<(), String>;
}

pub trait OwnershipStore {
    fn save(&mut self, state: &Ownership) -> io::Result<()>;
}

/// Durable 0600 state file: write temp, fsync, rename, fsync directory.
#[cfg(unix)]
pub struct FileOwnershipStore {
    pub path: std::path::PathBuf,
}

#[cfg(unix)]
impl FileOwnershipStore {
    pub fn load(&self) -> Ownership {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ownership::from_json(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ownership::Unclaimed,
            // Unreadable is not the same as absent: never unclaim on I/O error.
            Err(_) => Ownership::RecoveryRequired,
        }
    }
}

#[cfg(unix)]
impl OwnershipStore for FileOwnershipStore {
    fn save(&mut self, state: &Ownership) -> io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let tmp = self.path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        f.write_all(state.to_json().as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, &self.path)?;
        if let Some(dir) = self.path.parent() {
            std::fs::File::open(dir)?.sync_all()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GattError {
    NotReadable,
    NotWritable,
    Frame(FrameError),
    Request(&'static str),
    Claim(ClaimError),
    Storage,
}

impl GattError {
    pub fn code(&self) -> &'static str {
        match self {
            GattError::NotReadable => "not_readable",
            GattError::NotWritable => "not_writable",
            GattError::Frame(FrameError::TooLarge) => "too_large",
            GattError::Frame(FrameError::PayloadTooSmall) => "payload_too_small",
            GattError::Frame(FrameError::Malformed) => "malformed",
            GattError::Frame(FrameError::Inconsistent) => "inconsistent",
            GattError::Frame(FrameError::Duplicate) => "duplicate",
            GattError::Request(code) => code,
            GattError::Claim(c) => match c {
                ClaimError::NotClaimable => "not_claimable",
                ClaimError::NoPairingInProgress => "no_pairing_in_progress",
                ClaimError::PairingExpired => "pairing_expired",
                ClaimError::LockedOut => "locked_out",
                ClaimError::InvalidOwnerId => "invalid_owner_id",
                ClaimError::BadProof => "bad_proof",
                ClaimError::NotOwner => "not_owner",
                ClaimError::PhysicalConfirmationRequired => "physical_confirmation_required",
                ClaimError::Random => "internal_error",
            },
            GattError::Storage => "storage_failed",
        }
    }
}

fn network_name(state: NetworkState) -> &'static str {
    match state {
        NetworkState::Disconnected => "DISCONNECTED",
        NetworkState::Scanning => "SCANNING",
        NetworkState::Connecting => "CONNECTING",
        NetworkState::Connected => "CONNECTED",
        NetworkState::Failed => "FAILED",
    }
}

pub struct GattService<N: NetworkBackend, S: OwnershipStore> {
    ownership: OwnershipManager,
    network: N,
    store: S,
    reassembly: HashMap<Characteristic, Reassembler>,
    network_state: NetworkState,
    network_error: Option<&'static str>,
    scan: Vec<ScanResult>,
    scan_cursor: usize,
    pub model: String,
    pub firmware: String,
}

impl<N: NetworkBackend, S: OwnershipStore> GattService<N, S> {
    pub fn new(ownership: OwnershipManager, network: N, store: S) -> Self {
        Self {
            ownership, network, store,
            reassembly: HashMap::new(),
            network_state: NetworkState::Disconnected,
            network_error: None,
            scan: Vec::new(),
            scan_cursor: 0,
            model: "t3box-pi5".into(),
            firmware: env!("CARGO_PKG_VERSION").into(),
        }
    }

    pub fn ownership(&self) -> &Ownership {
        self.ownership.state()
    }

    pub fn network_state(&self) -> NetworkState {
        self.network_state
    }

    pub fn network(&self) -> &N {
        &self.network
    }

    /// Handle a characteristic read; returns the complete JSON message.
    pub fn read(&mut self, c: Characteristic, now: Instant) -> Result<Vec<u8>, GattError> {
        let value = match c {
            Characteristic::ProtocolVersion => json!({"version": PROTOCOL_VERSION}),
            Characteristic::DeviceInformation => json!({
                "model": self.model, "firmware": self.firmware, "protocol": PROTOCOL_VERSION,
                "ownership": self.ownership.reported_state(), "network": network_name(self.network_state),
            }),
            Characteristic::PairingChallenge => {
                if *self.ownership.state() == Ownership::Unclaimed {
                    let nonce = self.ownership.begin_pairing(now).map_err(GattError::Claim)?;
                    json!({"state": "PAIRING", "nonce": hex(&nonce)})
                } else {
                    json!({"state": self.ownership.reported_state()})
                }
            }
            Characteristic::OperationChallenge => {
                let nonce = self.ownership.operation_challenge(now).map_err(GattError::Claim)?;
                json!({"nonce": hex(&nonce)})
            }
            Characteristic::WifiScanResults => {
                self.require_owner()?;
                let pages = self.scan.len().div_ceil(SCAN_PAGE_SIZE).max(1);
                let page = self.scan_cursor % pages;
                self.scan_cursor = page + 1;
                let networks: Vec<Value> = self.scan.iter().skip(page * SCAN_PAGE_SIZE).take(SCAN_PAGE_SIZE)
                    .map(|n| json!({"ssid": n.ssid, "rssi": n.rssi, "security": n.security})).collect();
                json!({"page": page, "pages": pages, "networks": networks})
            }
            Characteristic::NetworkStatus => match self.network_error {
                Some(error) => json!({"state": network_name(self.network_state), "error": error}),
                None => json!({"state": network_name(self.network_state)}),
            },
            // Fail closed until trusted TLS and device identity exist.
            Characteristic::ManagementEndpoint => json!({"urls": [], "certificate_sha256": null}),
            Characteristic::DevicePublicKey => json!({"public_key": null}),
            Characteristic::PairingResponse | Characteristic::WifiScanRequest | Characteristic::WifiConfiguration => {
                return Err(GattError::NotReadable)
            }
        };
        Ok(value.to_string().into_bytes())
    }

    fn require_owner(&self) -> Result<(), GattError> {
        match self.ownership.state() {
            Ownership::Claimed { .. } => Ok(()),
            _ => Err(GattError::Claim(ClaimError::NotOwner)),
        }
    }

    /// Handle one written fragment. Returns `Some(response JSON)` once a
    /// complete message has been processed.
    pub fn write(&mut self, c: Characteristic, fragment: &[u8], now: Instant) -> Result<Option<Vec<u8>>, GattError> {
        if !c.writable() {
            return Err(GattError::NotWritable);
        }
        let message = match self.reassembly.entry(c).or_default().push(fragment, now).map_err(GattError::Frame)? {
            Some(m) => m,
            None => return Ok(None),
        };
        let request: Value = serde_json::from_slice(&message).map_err(|_| GattError::Request("invalid_json"))?;
        let Value::Object(request) = request else { return Err(GattError::Request("not_object")) };
        let response = match c {
            Characteristic::PairingResponse => self.pairing_response(&request, now)?,
            _ => {
                let payload = self.authenticated(c, &request, now)?;
                match c {
                    Characteristic::WifiScanRequest => self.wifi_scan(),
                    Characteristic::WifiConfiguration => self.wifi_configure(&payload)?,
                    _ => unreachable!("writable() covers all cases"),
                }
            }
        };
        Ok(Some(response.to_string().into_bytes()))
    }

    fn pairing_response(&mut self, request: &serde_json::Map<String, Value>, now: Instant) -> Result<Value, GattError> {
        let (Some(Value::String(owner_id)), Some(Value::String(proof)), 2) = (request.get("owner_id"), request.get("proof"), request.len()) else {
            return Err(GattError::Request("invalid_request"));
        };
        let proof = unhex::<32>(proof).ok_or(GattError::Request("invalid_request"))?;
        let claimed = self.ownership.complete_pairing(owner_id, &proof, now).map_err(GattError::Claim)?.clone();
        if self.store.save(&claimed).is_err() {
            // Not durable means not claimed: roll back so a retry is possible.
            let _ = self.ownership.factory_reset(true);
            return Err(GattError::Storage);
        }
        Ok(json!({"state": "CLAIMED"}))
    }

    /// Verify `{"operation","payload_hex","proof"}` and return the payload.
    fn authenticated(&mut self, c: Characteristic, request: &serde_json::Map<String, Value>, now: Instant) -> Result<Vec<u8>, GattError> {
        let expected = c.operation().ok_or(GattError::NotWritable)?;
        let (Some(Value::String(operation)), Some(Value::String(payload_hex)), Some(Value::String(proof)), 3) =
            (request.get("operation"), request.get("payload_hex"), request.get("proof"), request.len())
        else {
            return Err(GattError::Request("invalid_request"));
        };
        if operation != expected {
            return Err(GattError::Request("wrong_operation"));
        }
        if payload_hex.len() % 2 != 0 || !payload_hex.is_ascii() {
            return Err(GattError::Request("invalid_request"));
        }
        let payload: Vec<u8> = (0..payload_hex.len()).step_by(2)
            .map(|i| u8::from_str_radix(&payload_hex[i..i + 2], 16))
            .collect::<Result<_, _>>().map_err(|_| GattError::Request("invalid_request"))?;
        let proof = unhex::<32>(proof).ok_or(GattError::Request("invalid_request"))?;
        self.ownership.authorize(operation, &payload, &proof, now).map_err(GattError::Claim)?;
        Ok(payload)
    }

    fn wifi_scan(&mut self) -> Value {
        let previous = self.network_state;
        self.network_state = NetworkState::Scanning;
        let result = self.network.scan();
        self.scan_cursor = 0;
        match result {
            Ok(mut list) => {
                list.retain(|n| !n.ssid.is_empty() && n.ssid.len() <= 32);
                // Keep the strongest entry per SSID, then order by signal.
                list.sort_by(|a, b| a.ssid.cmp(&b.ssid).then_with(|| b.rssi.cmp(&a.rssi)));
                list.dedup_by(|later, kept| later.ssid == kept.ssid);
                list.sort_by(|a, b| b.rssi.cmp(&a.rssi).then_with(|| a.ssid.cmp(&b.ssid)));
                list.truncate(MAX_SCAN_RESULTS);
                self.scan = list;
                // A scan does not change connectivity.
                self.network_state = previous;
                json!({"networks": self.scan.len(), "pages": self.scan.len().div_ceil(SCAN_PAGE_SIZE).max(1)})
            }
            Err(_) => {
                self.scan.clear();
                self.network_state = NetworkState::Failed;
                self.network_error = Some("scan_failed");
                json!({"networks": 0, "error": "scan_failed"})
            }
        }
    }

    fn wifi_configure(&mut self, payload: &[u8]) -> Result<Value, GattError> {
        let config = parse_wifi_configuration(payload).map_err(GattError::Request)?;
        self.network_state = NetworkState::Connecting;
        self.network_error = None;
        // Backend errors are reduced to a fixed code so no credential or
        // NetworkManager detail can leak over BLE.
        match self.network.connect(&config) {
            Ok(()) => { self.network_state = NetworkState::Connected; }
            Err(_) => { self.network_state = NetworkState::Failed; self.network_error = Some("join_failed"); }
        }
        Ok(json!({"state": network_name(self.network_state)}))
    }

    /// Called by the GPIO handler when the reset button is held at boot.
    pub fn physical_reset(&mut self) -> Result<(), GattError> {
        self.ownership.factory_reset(true).map_err(GattError::Claim)?;
        self.store.save(self.ownership.state()).map_err(|_| GattError::Storage)?;
        self.network_state = NetworkState::Disconnected;
        self.network_error = None;
        self.scan.clear();
        Ok(())
    }
}

/// Nonce helper for clients/tests.
pub fn nonce_from_json(message: &[u8], field: &str) -> Option<[u8; NONCE_BYTES]> {
    let v: Value = serde_json::from_slice(message).ok()?;
    unhex::<NONCE_BYTES>(v.get(field)?.as_str()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provisioning::{claim_proof, derive_owner_key, fragment, operation_proof, WifiSecurity, MIN_ATT_PAYLOAD};

    const SECRET: [u8; 16] = *b"fedcba9876543210";

    #[derive(Default)]
    struct FakeNetwork {
        joined: Vec<WifiConfiguration>,
        fail_join: bool,
        networks: Vec<ScanResult>,
    }
    impl NetworkBackend for FakeNetwork {
        fn scan(&mut self) -> Result<Vec<ScanResult>, String> { Ok(self.networks.clone()) }
        fn connect(&mut self, config: &WifiConfiguration) -> Result<(), String> {
            self.joined.push(config.clone());
            if self.fail_join { Err("Secrets were required: psk=hunter22".into()) } else { Ok(()) }
        }
    }

    #[derive(Default)]
    struct MemoryStore { saved: Vec<Ownership>, fail: bool }
    impl OwnershipStore for MemoryStore {
        fn save(&mut self, state: &Ownership) -> io::Result<()> {
            if self.fail { return Err(io::Error::other("disk full")); }
            self.saved.push(state.clone());
            Ok(())
        }
    }

    type Service = GattService<FakeNetwork, MemoryStore>;

    fn service(network: FakeNetwork) -> Service {
        GattService::new(OwnershipManager::new(SECRET, Ownership::Unclaimed), network, MemoryStore::default())
    }

    /// Virtual phone: writes through the minimum BLE MTU.
    fn send(s: &mut Service, c: Characteristic, body: &Value, now: Instant) -> Result<Value, GattError> {
        let bytes = body.to_string().into_bytes();
        let frags = fragment(9, &bytes, MIN_ATT_PAYLOAD).expect("fits");
        let mut out = None;
        for f in &frags { out = s.write(c, f, now)?; }
        Ok(serde_json::from_slice(&out.expect("complete")).unwrap())
    }

    fn read(s: &mut Service, c: Characteristic, now: Instant) -> Value {
        serde_json::from_slice(&s.read(c, now).unwrap()).unwrap()
    }

    fn claim(s: &mut Service, now: Instant) -> [u8; 32] {
        let challenge = s.read(Characteristic::PairingChallenge, now).unwrap();
        let nonce = nonce_from_json(&challenge, "nonce").unwrap();
        let r = send(s, Characteristic::PairingResponse,
            &json!({"owner_id":"iphone-1","proof":hex(&claim_proof(&SECRET, &nonce, "iphone-1"))}), now).unwrap();
        assert_eq!(r["state"], "CLAIMED");
        derive_owner_key(&SECRET, &nonce, "iphone-1")
    }

    fn op(s: &mut Service, key: &[u8; 32], c: Characteristic, operation: &str, payload: &[u8], now: Instant) -> Result<Value, GattError> {
        let nonce = nonce_from_json(&s.read(Characteristic::OperationChallenge, now).unwrap(), "nonce").unwrap();
        let proof = operation_proof(key, &nonce, operation, payload);
        send(s, c, &json!({"operation":operation,"payload_hex":hex(payload),"proof":hex(&proof)}), now)
    }

    #[test]
    fn uuids_are_unique_and_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for c in Characteristic::ALL {
            assert!(seen.insert(c.uuid()));
            assert_eq!(Characteristic::from_uuid(&c.uuid().to_uppercase()), Some(c));
        }
        assert_eq!(Characteristic::from_uuid(SERVICE_UUID), None);
    }

    #[test]
    fn virtual_phone_end_to_end_claim_scan_and_join() {
        let now = Instant::now();
        let networks = (0..45).map(|i| ScanResult { ssid: format!("net{i:02}"), rssi: -30 - i, security: "wpa-psk".into() })
            .chain([ScanResult { ssid: "net00".into(), rssi: -90, security: "open".into() }, ScanResult { ssid: String::new(), rssi: 0, security: "open".into() }])
            .collect();
        let mut s = service(FakeNetwork { networks, ..Default::default() });
        assert_eq!(read(&mut s, Characteristic::DeviceInformation, now)["ownership"], "UNCLAIMED");
        // Unclaimed devices expose nothing owner-only.
        assert_eq!(s.read(Characteristic::WifiScanResults, now), Err(GattError::Claim(ClaimError::NotOwner)));
        let key = claim(&mut s, now);
        assert_eq!(s.store.saved.len(), 1, "persisted before acknowledgement");
        assert_eq!(read(&mut s, Characteristic::PairingChallenge, now)["state"], "CLAIMED");

        let r = op(&mut s, &key, Characteristic::WifiScanRequest, "wifi.scan", b"{}", now).unwrap();
        assert_eq!((r["networks"].as_u64(), r["pages"].as_u64()), (Some(45), Some(3)));
        let pages: Vec<Value> = (0..3).map(|_| read(&mut s, Characteristic::WifiScanResults, now)).collect();
        assert_eq!(pages[0]["networks"][0]["ssid"], "net00");
        assert_eq!(pages[0]["networks"][0]["rssi"], -30, "strongest duplicate kept");
        assert_eq!(pages[2]["networks"].as_array().unwrap().len(), 5);
        assert_eq!(read(&mut s, Characteristic::WifiScanResults, now)["page"], 0, "cursor wraps");

        let payload = br#"{"ssid":"LT Hall","security":"wpa-psk","psk":"correct horse"}"#;
        let r = op(&mut s, &key, Characteristic::WifiConfiguration, "wifi.configure", payload, now).unwrap();
        assert_eq!(r["state"], "CONNECTED");
        assert_eq!(s.network().joined[0].security, WifiSecurity::WpaPsk("correct horse".into()));
        assert_eq!(read(&mut s, Characteristic::NetworkStatus, now)["state"], "CONNECTED");
        assert_eq!(read(&mut s, Characteristic::ManagementEndpoint, now)["urls"], json!([]));
    }

    #[test]
    fn join_failure_keeps_owner_and_leaks_nothing() {
        let now = Instant::now();
        let mut s = service(FakeNetwork { fail_join: true, ..Default::default() });
        let key = claim(&mut s, now);
        let r = op(&mut s, &key, Characteristic::WifiConfiguration, "wifi.configure", br#"{"ssid":"LT","security":"wpa-psk","psk":"hunter22"}"#, now).unwrap();
        assert_eq!(r["state"], "FAILED");
        let status = s.read(Characteristic::NetworkStatus, now).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&status).unwrap()["error"], "join_failed");
        assert!(!String::from_utf8_lossy(&status).contains("hunter22"));
        assert!(matches!(s.ownership(), Ownership::Claimed { .. }));
    }

    #[test]
    fn unauthenticated_or_mismatched_operations_are_rejected() {
        let now = Instant::now();
        let mut s = service(FakeNetwork::default());
        let key = claim(&mut s, now);
        let payload = br#"{"ssid":"LT","security":"open"}"#;
        // Proof for scan replayed against configuration.
        assert_eq!(op(&mut s, &key, Characteristic::WifiConfiguration, "wifi.scan", payload, now), Err(GattError::Request("wrong_operation")));
        let wrong_key = [0u8; 32];
        assert_eq!(op(&mut s, &wrong_key, Characteristic::WifiConfiguration, "wifi.configure", payload, now), Err(GattError::Claim(ClaimError::BadProof)));
        assert_eq!(send(&mut s, Characteristic::WifiConfiguration, &json!({"operation":"wifi.configure"}), now), Err(GattError::Request("invalid_request")));
        let bad = op(&mut s, &key, Characteristic::WifiConfiguration, "wifi.configure", br#"{"ssid":"LT","security":"wep"}"#, now);
        assert_eq!(bad, Err(GattError::Request("invalid_security")));
        assert!(s.network().joined.is_empty());
        assert_eq!(s.write(Characteristic::NetworkStatus, &[1, 0, 1, b'x'], now), Err(GattError::NotWritable));
        assert_eq!(s.read(Characteristic::PairingResponse, now), Err(GattError::NotReadable));
    }

    #[test]
    fn claim_is_not_acknowledged_if_it_cannot_be_persisted() {
        let now = Instant::now();
        let mut s = GattService::new(OwnershipManager::new(SECRET, Ownership::Unclaimed), FakeNetwork::default(), MemoryStore { fail: true, ..Default::default() });
        let nonce = nonce_from_json(&s.read(Characteristic::PairingChallenge, now).unwrap(), "nonce").unwrap();
        let r = send(&mut s, Characteristic::PairingResponse, &json!({"owner_id":"p","proof":hex(&claim_proof(&SECRET, &nonce, "p"))}), now);
        assert_eq!(r, Err(GattError::Storage));
        assert_eq!(*s.ownership(), Ownership::Unclaimed);
    }

    #[test]
    fn physical_reset_unclaims_and_persists() {
        let now = Instant::now();
        let mut s = service(FakeNetwork::default());
        claim(&mut s, now);
        s.physical_reset().unwrap();
        assert_eq!(*s.ownership(), Ownership::Unclaimed);
        assert_eq!(s.store.saved.last(), Some(&Ownership::Unclaimed));
        assert_eq!(GattError::Claim(ClaimError::LockedOut).code(), "locked_out");
    }

    #[cfg(unix)]
    #[test]
    fn file_store_is_durable_private_and_fails_safe() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("t3box-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = FileOwnershipStore { path: dir.join("ownership.json") };
        assert_eq!(store.load(), Ownership::Unclaimed);
        let state = Ownership::Claimed { owner_id: "a".into(), owner_key: [3; 32] };
        store.save(&state).unwrap();
        assert_eq!(store.load(), state);
        assert_eq!(std::fs::metadata(&store.path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::write(&store.path, "garbage").unwrap();
        assert_eq!(store.load(), Ownership::RecoveryRequired);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
