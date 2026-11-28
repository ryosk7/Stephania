//! Provisioning protocol v3: one request/response channel over GATT
//! (contracts/ble-protocol.md).
//!
//! Designed around common BlueZ / Linux BLE failure modes:
//! - No dynamic characteristic reads (no ATT long reads, no read side
//!   effects): every operation is a request message written in fragments
//!   to the single RPC characteristic; the response is indicated back.
//! - No BLE pairing/bonding: ownership is an X25519 key agreement, requests
//!   are authenticated with HMAC and Wi-Fi credentials are sealed at the
//!   application layer.
//! - Lost responses are recoverable: mutating requests are kept in a small
//!   replay cache, so a client that reconnects and resends the identical
//!   request (same message id and bytes) gets the same response without the
//!   operation running twice. Only requests whose bytes already bind a fresh
//!   value (the phone's claim key, or a proof over a single-use nonce) are
//!   cached, so a cached response is never served to an unrelated request,
//!   and unauthenticated requests cannot evict cached ones. Challenges are
//!   never cached: a retry simply gets a fresh one.
//! - Per-peer reassembly state, dropped on disconnect, so interleaved or
//!   abandoned transfers from different centrals cannot corrupt each other.
//! - Stateless operations (no read cursors): scan results return in one
//!   response.
//!
//! A transport (BlueZ adapter, socket test harness) calls
//! [`RpcService::handle_fragment`] for every written fragment and indicates
//! the returned bytes, fragmented with [`crate::provisioning::fragment`]
//! under the request's message id.

use crate::management_tokens::TokenStore;
use crate::provisioning::{
    hex, open, seal, parse_wifi_configuration, unhex, ClaimError, FrameError, NetworkState, Ownership, OwnershipManager,
    Reassembler, WifiConfiguration,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: u64 = 3;
/// New service UUID per protocol major version, so clients never apply a
/// cached GATT layout of another version.
pub const SERVICE_UUID: &str = "7c3b0200-5d1e-4a6f-9b0e-3a1f2c4d5e60";
/// Static, tiny (fits one minimum-MTU read): `{"v":3,"p":"steph"}`.
pub const INFO_UUID: &str = "7c3b0201-5d1e-4a6f-9b0e-3a1f2c4d5e60";
/// Write (with response) request fragments; responses are indicated.
pub const RPC_UUID: &str = "7c3b0202-5d1e-4a6f-9b0e-3a1f2c4d5e60";
pub const INFO_VALUE: &[u8] = br#"{"v":3,"p":"steph"}"#;

/// Scan results are returned in one response; 40 worst-case entries
/// (~75 bytes each) stay under the 4096-byte message limit.
pub const MAX_SCAN_RESULTS: usize = 40;
pub const REPLAY_TTL: Duration = Duration::from_secs(60);
pub const REPLAY_ENTRIES: usize = 8;
pub const MAX_PEERS: usize = 4;

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
    /// Whether a wired (Ethernet) connection is up, so setup can skip Wi-Fi.
    fn wired(&mut self) -> bool { false }
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

pub fn claim_error_code(c: &ClaimError) -> &'static str {
    match c {
        ClaimError::NotClaimable => "not_claimable",
        ClaimError::NoPairingInProgress => "no_challenge",
        ClaimError::PairingExpired => "challenge_expired",
        ClaimError::LockedOut => "locked_out",
        ClaimError::InvalidOwnerId => "invalid_owner_id",
        ClaimError::BadProof => "bad_proof",
        ClaimError::BadKey => "bad_key",
        ClaimError::NotOwner => "not_owner",
        ClaimError::PhysicalConfirmationRequired => "physical_confirmation_required",
        ClaimError::Random => "internal_error",
    }
}

fn frame_error_code(e: &FrameError) -> &'static str {
    match e {
        FrameError::TooLarge => "too_large",
        FrameError::PayloadTooSmall => "payload_too_small",
        FrameError::Malformed => "malformed",
        FrameError::Inconsistent => "inconsistent",
        FrameError::Duplicate => "duplicate",
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

/// Error code carried in a `{"error": code}` response. The `error` key is
/// reserved for request failures; status fields use other names
/// (`last_error`, `scan_error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcError(pub &'static str);

impl From<ClaimError> for RpcError {
    fn from(c: ClaimError) -> Self { RpcError(claim_error_code(&c)) }
}

struct Replay {
    digest: [u8; 32],
    response: Vec<u8>,
    at: Instant,
}

/// Per-central state, dropped on disconnect.
struct Peer {
    reassembler: Reassembler,
    last: Instant,
    /// Completed a claim or an owner-authenticated operation on this link.
    authenticated: bool,
}

pub struct RpcService<N: NetworkBackend, S: OwnershipStore> {
    ownership: OwnershipManager,
    network: N,
    store: S,
    peers: HashMap<String, Peer>,
    replay: Vec<Replay>,
    network_state: NetworkState,
    network_error: Option<&'static str>,
    /// SSIDs that the last scan saw with any security, so an open network
    /// with the same name (an evil twin) is not joined (`weaker_security`).
    secured_ssids: Vec<String>,
    pub model: String,
    pub firmware: String,
    management: Option<Management>,
}

/// LAN management enrollment: where the device PKI publishes its endpoint
/// (crate::pki) and where issued token hashes are stored.
#[derive(Debug, Clone)]
pub struct Management {
    pub pki_dir: std::path::PathBuf,
    pub tokens: TokenStore,
    /// Maintenance code secret, rotated on every enrollment
    /// (docs/MAINTENANCE.md). `None`: enrollments carry no `otp_secret`.
    pub otp: Option<OtpSource>,
}

/// Where `management.enroll` gets a fresh maintenance code secret.
#[derive(Debug, Clone)]
pub enum OtpSource {
    /// stephania-maint's socket (`otp.rotate`), which owns the secret.
    Socket(std::path::PathBuf),
    /// Tests: a fixed secret.
    Fixed(Vec<u8>),
}

impl OtpSource {
    pub fn rotate(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Fixed(secret) => Ok(secret.clone()),
            #[cfg(unix)]
            Self::Socket(path) => {
                let mut stream = std::os::unix::net::UnixStream::connect(path).map_err(|e| format!("{}: {e}", path.display()))?;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                let req = json!({"version": 1, "request_id": "enroll", "method": "otp.rotate", "params": {}});
                crate::ipc::write_frame(&mut stream, req.to_string().as_bytes()).map_err(|e| e.to_string())?;
                let frame = crate::ipc::read_frame(&mut stream).map_err(|e| e.message.clone())?.ok_or("stephania-maint closed the connection")?;
                let r: Value = serde_json::from_slice(&frame).map_err(|e| e.to_string())?;
                if r.get("ok") != Some(&json!(true)) { return Err(format!("otp.rotate failed: {}", r["error"])); }
                let hex = r["result"]["otp_secret"].as_str().ok_or("otp.rotate returned no secret")?;
                let secret: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect();
                secret.filter(|s| s.len() >= 16).ok_or_else(|| "otp.rotate returned a malformed secret".into())
            }
            #[cfg(not(unix))]
            Self::Socket(_) => Err("stephania-maint needs a Unix platform".into()),
        }
    }
}

fn exact(req: &Map<String, Value>, keys: &[&str]) -> Result<(), RpcError> {
    if req.len() == keys.len() && keys.iter().all(|k| req.contains_key(*k)) { Ok(()) } else { Err(RpcError("invalid_request")) }
}

fn hex_field<const N: usize>(req: &Map<String, Value>, key: &str) -> Result<[u8; N], RpcError> {
    req.get(key).and_then(Value::as_str).and_then(unhex::<N>).ok_or(RpcError("invalid_request"))
}

fn hex_bytes(req: &Map<String, Value>, key: &str, max: usize) -> Result<Vec<u8>, RpcError> {
    let s = req.get(key).and_then(Value::as_str).ok_or(RpcError("invalid_request"))?;
    if !s.len().is_multiple_of(2) || s.len() > max * 2 || !s.is_ascii() {
        return Err(RpcError("invalid_request"));
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16)).collect::<Result<_, _>>()
        .map_err(|_| RpcError("invalid_request"))
}

fn error_bytes(e: RpcError) -> Vec<u8> {
    json!({"error": e.0}).to_string().into_bytes()
}

impl<N: NetworkBackend, S: OwnershipStore> RpcService<N, S> {
    pub fn new(ownership: OwnershipManager, network: N, store: S) -> Self {
        Self {
            ownership, network, store,
            peers: HashMap::new(),
            replay: Vec::new(),
            network_state: NetworkState::Disconnected,
            network_error: None,
            secured_ssids: Vec::new(),
            model: "stephania-pi5".into(),
            firmware: env!("CARGO_PKG_VERSION").into(),
            management: None,
        }
    }

    pub fn with_management(mut self, management: Management) -> Self {
        self.management = Some(management);
        self
    }

    fn endpoint(&self) -> Option<crate::pki::Endpoint> {
        self.management.as_ref().and_then(|m| crate::pki::Endpoint::load(&m.pki_dir))
    }

    pub fn ownership(&self) -> &Ownership { self.ownership.state() }
    pub fn network_state(&self) -> NetworkState { self.network_state }
    pub fn network(&self) -> &N { &self.network }

    /// Forget a peer's partial transfer (call on disconnect).
    pub fn disconnect(&mut self, peer: &str) {
        self.peers.remove(peer);
    }

    /// Whether `peer` has completed a claim or an owner-authenticated
    /// operation since it connected. The BLE adapter uses this to let
    /// unauthenticated centrals hold the single-central gate only briefly.
    pub fn authenticated(&self, peer: &str) -> bool {
        self.peers.get(peer).is_some_and(|p| p.authenticated)
    }

    fn mark_authenticated(&mut self, peer: &str) {
        if let Some(p) = self.peers.get_mut(peer) { p.authenticated = true; }
    }

    /// Owner-authenticated operation from `peer`; see [`OwnershipManager::authorize`].
    fn authorize(&mut self, peer: &str, operation: &str, payload: &[u8], proof: &[u8], now: Instant) -> Result<[u8; 32], RpcError> {
        let nonce = self.ownership.authorize(peer, operation, payload, proof, now)?;
        self.mark_authenticated(peer);
        Ok(nonce)
    }

    /// Feed one written fragment from `peer`. Returns the response to
    /// indicate once the request is complete (errors included, as
    /// `{"error": code}`), or `None` while more fragments are expected.
    pub fn handle_fragment(&mut self, peer: &str, fragment: &[u8], now: Instant) -> Option<Vec<u8>> {
        if !self.peers.contains_key(peer) && self.peers.len() >= MAX_PEERS {
            // Evict the stalest peer rather than refusing a new central.
            if let Some(oldest) = self.peers.iter().min_by_key(|(_, p)| p.last).map(|(k, _)| k.clone()) {
                self.peers.remove(&oldest);
            }
        }
        let entry = self.peers.entry(peer.to_owned())
            .or_insert_with(|| Peer { reassembler: Reassembler::default(), last: now, authenticated: false });
        entry.last = now;
        let message = match entry.reassembler.push(fragment, now) {
            Ok(Some(m)) => m,
            Ok(None) => return None,
            Err(e) => return Some(error_bytes(RpcError(frame_error_code(&e)))),
        };
        let message_id = fragment.first().copied().unwrap_or_default();
        Some(self.handle_message(peer, message_id, &message, now))
    }

    fn handle_message(&mut self, peer: &str, message_id: u8, message: &[u8], now: Instant) -> Vec<u8> {
        let digest = {
            let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
            ctx.update(&[message_id]);
            ctx.update(message);
            let mut d = [0u8; 32];
            d.copy_from_slice(ctx.finish().as_ref());
            d
        };
        self.replay.retain(|r| now.duration_since(r.at) < REPLAY_TTL);
        if let Some(hit) = self.replay.iter().find(|r| r.digest == digest) {
            return hit.response.clone();
        }
        let (response, cacheable) = match self.dispatch(peer, message, now) {
            Ok((v, cacheable)) => (v.to_string().into_bytes(), cacheable),
            Err(e) => (error_bytes(e), false),
        };
        if cacheable {
            if self.replay.len() >= REPLAY_ENTRIES { self.replay.remove(0); }
            self.replay.push(Replay { digest, response: response.clone(), at: now });
        }
        response
    }

    /// Returns the response and whether it must be replayed for an
    /// identical retry. Only operations whose request bytes bind a fresh
    /// value are cacheable (see the module docs); challenges are not.
    fn dispatch(&mut self, peer: &str, message: &[u8], now: Instant) -> Result<(Value, bool), RpcError> {
        let Ok(Value::Object(req)) = serde_json::from_slice::<Value>(message) else { return Err(RpcError("invalid_json")) };
        let op = req.get("op").and_then(Value::as_str).ok_or(RpcError("invalid_request"))?.to_owned();
        match op.as_str() {
            "device.info" => {
                exact(&req, &["op"])?;
                Ok((json!({
                    "model": self.model, "firmware": self.firmware, "protocol": PROTOCOL_VERSION,
                    "ownership": self.ownership.reported_state(), "network": network_name(self.network_state),
                }), false))
            }
            "pairing.challenge" => {
                exact(&req, &["op"])?;
                if *self.ownership.state() == Ownership::Unclaimed {
                    let c = self.ownership.begin_pairing(now)?;
                    // Not cached: a retry gets a fresh challenge, which
                    // supersedes this one.
                    Ok((json!({"state": "PAIRING", "nonce": hex(&c.nonce), "device_key": hex(&c.device_key)}), false))
                } else {
                    Ok((json!({"state": self.ownership.reported_state()}), false))
                }
            }
            "pairing.claim" => {
                exact(&req, &["op", "owner_id", "phone_key"])?;
                let owner_id = req["owner_id"].as_str().ok_or(RpcError("invalid_request"))?;
                let phone_key = hex_field::<32>(&req, "phone_key")?;
                let confirm = self.ownership.complete_pairing(owner_id, &phone_key, now)?;
                let claimed = self.ownership.state().clone();
                if self.store.save(&claimed).is_err() {
                    // Not durable means not claimed: roll back so a retry works.
                    self.ownership.abandon_claim();
                    return Err(RpcError("storage_failed"));
                }
                self.mark_authenticated(peer);
                // Cached: the request carries the phone's fresh key.
                Ok((json!({"state": "CLAIMED", "confirm": hex(&confirm)}), true))
            }
            "operation.challenge" => {
                exact(&req, &["op"])?;
                // Not cached: a retry gets another single-use nonce.
                let nonce = self.ownership.operation_challenge(peer, now)?;
                Ok((json!({"nonce": hex(&nonce)}), false))
            }
            "wifi.scan" => {
                exact(&req, &["op", "proof"])?;
                let proof = hex_field::<32>(&req, "proof")?;
                self.authorize(peer, "wifi.scan", b"", &proof, now)?;
                Ok((self.wifi_scan(), true))
            }
            "wifi.configure" => {
                exact(&req, &["op", "sealed", "proof"])?;
                let sealed = hex_bytes(&req, "sealed", 512)?;
                let proof = hex_field::<32>(&req, "proof")?;
                let nonce = self.authorize(peer, "wifi.configure", &sealed, &proof, now)?;
                let key = *self.ownership.owner_key().ok_or(RpcError("not_owner"))?;
                let plain = open(&key, &nonce, "wifi.configure", &sealed).ok_or(RpcError("bad_seal"))?;
                Ok((self.wifi_configure(&plain)?, true))
            }
            "network.status" => {
                exact(&req, &["op"])?;
                let wired = self.network.wired();
                Ok((match self.network_error {
                    Some(error) => json!({"state": network_name(self.network_state), "wired": wired, "last_error": error}),
                    None => json!({"state": network_name(self.network_state), "wired": wired}),
                }, false))
            }
            // Unauthenticated hint only; URLs and the CA pin are delivered
            // sealed by management.enroll so a BLE man-in-the-middle cannot
            // substitute them.
            "management.endpoint" => {
                exact(&req, &["op"])?;
                Ok((json!({"available": self.endpoint().is_some()}), false))
            }
            "management.enroll" => {
                exact(&req, &["op", "proof"])?;
                let proof = hex_field::<32>(&req, "proof")?;
                let nonce = self.authorize(peer, "management.enroll", b"", &proof, now)?;
                let key = *self.ownership.owner_key().ok_or(RpcError("not_owner"))?;
                let endpoint = self.endpoint().ok_or(RpcError("management_unavailable"))?;
                let tokens = &self.management.as_ref().ok_or(RpcError("management_unavailable"))?.tokens;
                let now_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let otp = self.management.as_ref().and_then(|m| m.otp.clone());
                let issued = tokens.issue(now_unix).map_err(|_| RpcError("storage_failed"))?;
                let mut plain = json!({"token": issued.token, "token_id": issued.id, "urls": endpoint.urls(), "ca_sha256": endpoint.ca_sha256});
                // LAN access must not depend on maintenance: without
                // stephania-maint the enrollment simply carries no code secret.
                match otp.map(|o| o.rotate()) {
                    Some(Ok(secret)) => { plain["otp_secret"] = json!(crate::sshcert::base64_encode(&secret)); }
                    Some(Err(e)) => eprintln!("stephania-provision: no maintenance code secret for this enrollment: {e}"),
                    None => {}
                }
                Ok((json!({"sealed": hex(&seal(&key, &nonce, "management.enroll", plain.to_string().as_bytes()))}), true))
            }
            _ => Err(RpcError("unknown_op")),
        }
    }

    fn wifi_scan(&mut self) -> Value {
        let previous = self.network_state;
        self.network_state = NetworkState::Scanning;
        let result = self.network.scan();
        // A scan does not change connectivity.
        self.network_state = previous;
        match result {
            Ok(mut list) => {
                list.retain(|n| !n.ssid.is_empty() && n.ssid.len() <= 32 && !n.ssid.chars().any(char::is_control));
                // Keep the strongest entry per SSID and security, so an open
                // access point cannot hide a secured one with the same name.
                list.sort_by(|a, b| (&a.ssid, &a.security).cmp(&(&b.ssid, &b.security)).then_with(|| b.rssi.cmp(&a.rssi)));
                list.dedup_by(|later, kept| later.ssid == kept.ssid && later.security == kept.security);
                self.secured_ssids = list.iter().filter(|n| n.security != "open").map(|n| n.ssid.clone()).collect();
                list.sort_by(|a, b| b.rssi.cmp(&a.rssi).then_with(|| a.ssid.cmp(&b.ssid)));
                list.truncate(MAX_SCAN_RESULTS);
                let networks: Vec<Value> = list.iter().map(|n| json!({"ssid": n.ssid, "rssi": n.rssi, "security": n.security})).collect();
                json!({"networks": networks})
            }
            Err(_) => json!({"networks": [], "scan_error": "scan_failed"}),
        }
    }

    fn wifi_configure(&mut self, plain: &[u8]) -> Result<Value, RpcError> {
        let config = parse_wifi_configuration(plain).map_err(RpcError)?;
        if matches!(config.security, crate::provisioning::WifiSecurity::Open) && self.secured_ssids.iter().any(|s| s.as_bytes() == config.ssid.as_slice()) {
            return Err(RpcError("weaker_security"));
        }
        self.network_state = NetworkState::Connecting;
        self.network_error = None;
        // Backend errors are reduced to a fixed code so no credential or
        // NetworkManager detail can leak over BLE.
        match self.network.connect(&config) {
            Ok(()) => self.network_state = NetworkState::Connected,
            Err(_) => { self.network_state = NetworkState::Failed; self.network_error = Some("join_failed"); }
        }
        Ok(json!({"state": network_name(self.network_state)}))
    }

    /// Called by the GPIO handler when the reset button is held at boot.
    pub fn physical_reset(&mut self) -> Result<(), RpcError> {
        self.ownership.factory_reset(true)?;
        // A new owner must not inherit the previous owner's LAN access.
        if let Some(m) = &self.management {
            m.tokens.clear().map_err(|_| RpcError("storage_failed"))?;
        }
        self.store.save(self.ownership.state()).map_err(|_| RpcError("storage_failed"))?;
        self.network_state = NetworkState::Disconnected;
        self.network_error = None;
        self.replay.clear();
        self.peers.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provisioning::{claim_confirmation, client_claim, fragment, operation_proof, seal, PairingChallenge, WifiSecurity, MAX_MESSAGE_BYTES, MIN_ATT_PAYLOAD};

    fn challenge_of(c: &Value) -> PairingChallenge {
        PairingChallenge { nonce: unhex::<32>(c["nonce"].as_str().unwrap()).unwrap(), device_key: unhex::<32>(c["device_key"].as_str().unwrap()).unwrap() }
    }

    #[derive(Default)]
    struct FakeNetwork { joined: Vec<WifiConfiguration>, fail_join: bool, networks: Vec<ScanResult>, wired: bool }
    impl NetworkBackend for FakeNetwork {
        fn scan(&mut self) -> Result<Vec<ScanResult>, String> { Ok(self.networks.clone()) }
        fn connect(&mut self, config: &WifiConfiguration) -> Result<(), String> {
            self.joined.push(config.clone());
            if self.fail_join { Err("Secrets were required: psk=hunter22".into()) } else { Ok(()) }
        }
        fn wired(&mut self) -> bool { self.wired }
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

    type Svc = RpcService<FakeNetwork, MemoryStore>;

    fn svc(network: FakeNetwork) -> Svc {
        RpcService::new(OwnershipManager::new(Ownership::Unclaimed), network, MemoryStore::default())
    }

    /// Virtual phone: requests in minimum-MTU fragments.
    struct Phone { id: u8, peer: &'static str }
    impl Phone {
        fn new(peer: &'static str) -> Self { Self { id: 0, peer } }
        fn send_with_id(&self, s: &mut Svc, id: u8, body: &Value, now: Instant) -> Value {
            let bytes = body.to_string().into_bytes();
            let mut out = None;
            for f in fragment(id, &bytes, MIN_ATT_PAYLOAD).unwrap() { out = s.handle_fragment(self.peer, &f, now); }
            serde_json::from_slice(&out.expect("complete")).unwrap()
        }
        fn send(&mut self, s: &mut Svc, body: Value, now: Instant) -> Value {
            self.id = self.id.wrapping_add(1);
            self.send_with_id(s, self.id, &body, now)
        }
        fn claim(&mut self, s: &mut Svc, now: Instant) -> [u8; 32] {
            let c = challenge_of(&self.send(s, json!({"op":"pairing.challenge"}), now));
            let (phone_key, owner_key) = client_claim(&c, "iphone-1").unwrap();
            let r = self.send(s, json!({"op":"pairing.claim","owner_id":"iphone-1","phone_key":hex(&phone_key)}), now);
            assert_eq!(r, json!({"state":"CLAIMED","confirm":hex(&claim_confirmation(&owner_key, &c.nonce))}));
            owner_key
        }
        fn nonce(&mut self, s: &mut Svc, now: Instant) -> [u8; 32] {
            unhex::<32>(self.send(s, json!({"op":"operation.challenge"}), now)["nonce"].as_str().unwrap()).unwrap()
        }
        fn configure(&mut self, s: &mut Svc, key: &[u8; 32], config: &[u8], now: Instant) -> Value {
            let nonce = self.nonce(s, now);
            let sealed = seal(key, &nonce, "wifi.configure", config);
            let proof = operation_proof(key, &nonce, "wifi.configure", &sealed);
            self.send(s, json!({"op":"wifi.configure","sealed":hex(&sealed),"proof":hex(&proof)}), now)
        }
        fn scan(&mut self, s: &mut Svc, key: &[u8; 32], now: Instant) -> Value {
            let nonce = self.nonce(s, now);
            let proof = operation_proof(key, &nonce, "wifi.scan", b"");
            self.send(s, json!({"op":"wifi.scan","proof":hex(&proof)}), now)
        }
    }

    #[test]
    fn end_to_end_claim_scan_join_at_minimum_mtu() {
        let now = Instant::now();
        let networks = (0..45).map(|i| ScanResult { ssid: format!("net{i:02}"), rssi: -30 - i, security: "wpa-psk".into() })
            .chain([ScanResult { ssid: "net00".into(), rssi: -90, security: "open".into() }, ScanResult { ssid: String::new(), rssi: 0, security: "open".into() }])
            .collect();
        let mut s = svc(FakeNetwork { networks, ..Default::default() });
        let mut p = Phone::new("AA:BB");
        assert_eq!(p.send(&mut s, json!({"op":"device.info"}), now)["ownership"], "UNCLAIMED");
        assert_eq!(p.send(&mut s, json!({"op":"operation.challenge"}), now), json!({"error":"not_owner"}));
        let key = p.claim(&mut s, now);
        assert_eq!(s.store.saved.len(), 1, "persisted before acknowledgement");
        assert_eq!(p.send(&mut s, json!({"op":"pairing.challenge"}), now), json!({"state":"CLAIMED"}));
        let r = p.scan(&mut s, &key, now);
        let list = r["networks"].as_array().unwrap();
        assert_eq!(list.len(), MAX_SCAN_RESULTS);
        assert_eq!((list[0]["ssid"].as_str(), list[0]["rssi"].as_i64()), (Some("net00"), Some(-30)), "strongest duplicate kept");
        let r = p.configure(&mut s, &key, br#"{"ssid":"LT Hall","security":"wpa-psk","psk":"correct horse"}"#, now);
        assert_eq!(r, json!({"state":"CONNECTED"}));
        assert_eq!(s.network().joined[0].security, WifiSecurity::WpaPsk("correct horse".into()));
        assert_eq!(p.send(&mut s, json!({"op":"network.status"}), now), json!({"state":"CONNECTED","wired":false}));
        // A cable makes Wi-Fi optional: the phone skips that step.
        s.network.wired = true;
        assert_eq!(p.send(&mut s, json!({"op":"network.status"}), now)["wired"], true);
        assert_eq!(p.send(&mut s, json!({"op":"management.endpoint"}), now), json!({"available": false}));
    }

    #[test]
    fn open_twin_of_a_secured_network_is_listed_separately_and_refused() {
        let now = Instant::now();
        let net = |ssid: &str, rssi, security: &str| ScanResult { ssid: ssid.into(), rssi, security: security.into() };
        let networks = vec![net("Hall", -70, "wpa-psk"), net("Hall", -40, "open"), net("Hall", -80, "wpa-psk"), net("Cafe", -60, "open")];
        let mut s = svc(FakeNetwork { networks, ..Default::default() });
        let mut p = Phone::new("p");
        let key = p.claim(&mut s, now);
        let r = p.scan(&mut s, &key, now);
        assert_eq!(r["networks"], json!([
            {"ssid":"Hall","rssi":-40,"security":"open"}, {"ssid":"Cafe","rssi":-60,"security":"open"}, {"ssid":"Hall","rssi":-70,"security":"wpa-psk"},
        ]));
        assert_eq!(p.configure(&mut s, &key, br#"{"ssid":"Hall","security":"open"}"#, now), json!({"error":"weaker_security"}));
        assert!(s.network().joined.is_empty(), "never joined the open twin");
        assert_eq!(p.configure(&mut s, &key, br#"{"ssid":"Cafe","security":"open"}"#, now), json!({"state":"CONNECTED"}));
        assert_eq!(p.configure(&mut s, &key, br#"{"ssid":"Hall","security":"wpa-psk","psk":"correct horse"}"#, now), json!({"state":"CONNECTED"}));
        assert_eq!(s.network().joined.len(), 2);
    }

    #[test]
    fn worst_case_scan_response_fits_one_message_at_minimum_mtu() {
        let now = Instant::now();
        let networks = (0..MAX_SCAN_RESULTS + 10).map(|i| ScanResult { ssid: format!("{i:02}{}", "\u{3042}".repeat(10)), rssi: -100, security: "unsupported".into() }).collect();
        let mut s = svc(FakeNetwork { networks, ..Default::default() });
        let mut p = Phone::new("p");
        let key = p.claim(&mut s, now);
        let nonce = p.nonce(&mut s, now);
        let proof = operation_proof(&key, &nonce, "wifi.scan", b"");
        let body = json!({"op":"wifi.scan","proof":hex(&proof)}).to_string().into_bytes();
        let mut out = None;
        for f in fragment(77, &body, MIN_ATT_PAYLOAD).unwrap() { out = s.handle_fragment("p", &f, now); }
        let out = out.unwrap();
        assert!(out.len() <= MAX_MESSAGE_BYTES, "{} bytes", out.len());
        assert!(fragment(77, &out, MIN_ATT_PAYLOAD).is_ok(), "indicatable at minimum MTU");
    }

    #[test]
    fn identical_retry_replays_response_without_rerunning() {
        let now = Instant::now();
        let mut s = svc(FakeNetwork::default());
        let p = Phone::new("phone");
        let lost = p.send_with_id(&mut s, 1, &json!({"op":"pairing.challenge"}), now);
        // Challenge retry (indication lost): a fresh challenge supersedes it.
        let c = p.send_with_id(&mut s, 1, &json!({"op":"pairing.challenge"}), now);
        assert_ne!(c["nonce"], lost["nonce"]);
        let (phone_key, owner_key) = client_claim(&challenge_of(&c), "iphone-1").unwrap();
        let claim = json!({"op":"pairing.claim","owner_id":"iphone-1","phone_key":hex(&phone_key)});
        let claimed = json!({"state":"CLAIMED","confirm":hex(&claim_confirmation(&owner_key, &challenge_of(&c).nonce))});
        assert_eq!(p.send_with_id(&mut s, 2, &claim, now), claimed);
        assert!(s.authenticated("phone"));
        // Lost indication + reconnect from a new address: same bytes, same id.
        let q = Phone::new("phone-new-rpa");
        assert_eq!(q.send_with_id(&mut s, 2, &claim, now), claimed);
        assert!(!s.authenticated("phone-new-rpa"), "a replayed response proves nothing about this link");
        assert_eq!(s.store.saved.len(), 1, "claim ran once");
        // Different id = new request (not a replay).
        assert_eq!(q.send_with_id(&mut s, 3, &claim, now), json!({"error":"not_claimable"}));
        // Unauthenticated requests from anyone cannot evict the cached claim.
        let attacker = Phone::new("attacker");
        for id in 0..=255u8 { attacker.send_with_id(&mut s, id, &json!({"op":"operation.challenge"}), now); }
        assert_eq!(q.send_with_id(&mut s, 2, &claim, now), claimed);
        // Replay expires.
        assert_eq!(q.send_with_id(&mut s, 2, &claim, now + REPLAY_TTL), json!({"error":"not_claimable"}));

        // An authenticated operation retried from a new address after its
        // response was lost runs once and is replayed.
        let n = unhex::<32>(q.send_with_id(&mut s, 4, &json!({"op":"operation.challenge"}), now)["nonce"].as_str().unwrap()).unwrap();
        let config = br#"{"ssid":"LT","security":"open"}"#;
        let sealed = seal(&owner_key, &n, "wifi.configure", config);
        let req = json!({"op":"wifi.configure","sealed":hex(&sealed),"proof":hex(&operation_proof(&owner_key, &n, "wifi.configure", &sealed))});
        assert_eq!(q.send_with_id(&mut s, 5, &req, now), json!({"state":"CONNECTED"}));
        assert_eq!(p.send_with_id(&mut s, 5, &req, now), json!({"state":"CONNECTED"}));
        assert_eq!(s.network().joined.len(), 1, "joined once");
    }

    #[test]
    fn challenges_are_never_replayed_across_sessions_or_peers() {
        let now = Instant::now();
        let mut s = svc(FakeNetwork::default());
        // An attacker pre-caches pairing challenges under every id the app
        // could use, then leaves.
        let attacker = Phone::new("attacker");
        let seen: Vec<Value> = (1..=8u8).map(|id| attacker.send_with_id(&mut s, id, &json!({"op":"pairing.challenge"}), now)).collect();
        s.disconnect("attacker");
        // The owner's challenge under one of those ids is fresh, so its claim
        // is bound to the live pairing and the confirmation matches.
        let owner = Phone::new("owner");
        let c = owner.send_with_id(&mut s, 2, &json!({"op":"pairing.challenge"}), now);
        assert!(seen.iter().all(|old| old["nonce"] != c["nonce"]));
        let (phone_key, key) = client_claim(&challenge_of(&c), "iphone-1").unwrap();
        let r = owner.send_with_id(&mut s, 3, &json!({"op":"pairing.claim","owner_id":"iphone-1","phone_key":hex(&phone_key)}), now);
        assert_eq!(r, json!({"state":"CLAIMED","confirm":hex(&claim_confirmation(&key, &challenge_of(&c).nonce))}));

        // A later session reusing message ids gets a fresh operation nonce,
        // never the consumed one of the earlier session.
        let mut used = Vec::new();
        for _session in 0..2 {
            let n = owner.send_with_id(&mut s, 4, &json!({"op":"operation.challenge"}), now);
            let n = unhex::<32>(n["nonce"].as_str().unwrap()).unwrap();
            assert!(!used.contains(&n), "nonce served twice");
            used.push(n);
            let proof = operation_proof(&key, &n, "wifi.scan", b"");
            assert!(owner.send_with_id(&mut s, 5, &json!({"op":"wifi.scan","proof":hex(&proof)}), now).get("networks").is_some());
        }
        // An attacker's challenges and failed proofs neither displace the
        // owner's nonce nor lock the owner out.
        let n = unhex::<32>(owner.send_with_id(&mut s, 6, &json!({"op":"operation.challenge"}), now)["nonce"].as_str().unwrap()).unwrap();
        for id in 0..20u8 {
            attacker.send_with_id(&mut s, id, &json!({"op":"operation.challenge"}), now);
            attacker.send_with_id(&mut s, id.wrapping_add(100), &json!({"op":"wifi.scan","proof":hex(&[id; 32])}), now);
        }
        assert_eq!(attacker.send_with_id(&mut s, 99, &json!({"op":"wifi.scan","proof":hex(&[0u8; 32])}), now), json!({"error":"locked_out"}));
        assert!(!s.authenticated("attacker"));
        let proof = operation_proof(&key, &n, "wifi.scan", b"");
        assert!(owner.send_with_id(&mut s, 7, &json!({"op":"wifi.scan","proof":hex(&proof)}), now).get("networks").is_some());
    }

    #[test]
    fn join_failure_keeps_owner_and_leaks_nothing() {
        let now = Instant::now();
        let mut s = svc(FakeNetwork { fail_join: true, ..Default::default() });
        let mut p = Phone::new("p");
        let key = p.claim(&mut s, now);
        assert_eq!(p.configure(&mut s, &key, br#"{"ssid":"LT","security":"wpa-psk","psk":"hunter22"}"#, now), json!({"state":"FAILED"}));
        let status = p.send(&mut s, json!({"op":"network.status"}), now);
        assert_eq!(status, json!({"state":"FAILED","wired":false,"last_error":"join_failed"}));
        assert!(!status.to_string().contains("hunter22"));
        assert!(matches!(s.ownership(), Ownership::Claimed { .. }));
    }

    #[test]
    fn sealed_payload_and_proofs_are_enforced() {
        let now = Instant::now();
        let mut s = svc(FakeNetwork::default());
        let mut p = Phone::new("p");
        let key = p.claim(&mut s, now);
        let config = br#"{"ssid":"LT","security":"open"}"#;
        // Proof valid but ciphertext sealed under another key: rejected.
        let nonce = p.nonce(&mut s, now);
        let sealed = seal(&[0u8; 32], &nonce, "wifi.configure", config);
        let proof = operation_proof(&key, &nonce, "wifi.configure", &sealed);
        assert_eq!(p.send(&mut s, json!({"op":"wifi.configure","sealed":hex(&sealed),"proof":hex(&proof)}), now), json!({"error":"bad_seal"}));
        // Plaintext sent without sealing: rejected.
        let nonce = p.nonce(&mut s, now);
        let proof = operation_proof(&key, &nonce, "wifi.configure", config);
        assert_eq!(p.send(&mut s, json!({"op":"wifi.configure","sealed":hex(config),"proof":hex(&proof)}), now), json!({"error":"bad_seal"}));
        // Wrong owner key.
        let nonce = p.nonce(&mut s, now);
        let sealed = seal(&key, &nonce, "wifi.configure", config);
        let proof = operation_proof(&[1u8; 32], &nonce, "wifi.configure", &sealed);
        assert_eq!(p.send(&mut s, json!({"op":"wifi.configure","sealed":hex(&sealed),"proof":hex(&proof)}), now), json!({"error":"bad_proof"}));
        // Scan proof cannot be used for configure.
        let nonce = p.nonce(&mut s, now);
        let proof = operation_proof(&key, &nonce, "wifi.scan", b"");
        assert_eq!(p.send(&mut s, json!({"op":"wifi.configure","sealed":"00","proof":hex(&proof)}), now), json!({"error":"bad_proof"}));
        assert!(s.network().joined.is_empty());
        for bad in [json!({"op":"nope"}), json!({"op":"device.info","x":1}), json!({"x":1})] {
            assert!(p.send(&mut s, bad, now).get("error").is_some());
        }
    }

    #[test]
    fn per_peer_reassembly_isolates_interleaved_transfers() {
        let now = Instant::now();
        let mut s = svc(FakeNetwork::default());
        let a = fragment(5, json!({"op":"device.info"}).to_string().as_bytes(), MIN_ATT_PAYLOAD).unwrap();
        let b = fragment(5, json!({"op":"network.status"}).to_string().as_bytes(), MIN_ATT_PAYLOAD).unwrap();
        assert!(a.len() > 1 && b.len() > 1);
        assert_eq!(s.handle_fragment("A", &a[0], now), None);
        assert_eq!(s.handle_fragment("B", &b[0], now), None);
        let mut ra = None;
        for f in &a[1..] { ra = s.handle_fragment("A", f, now); }
        let mut rb = None;
        for f in &b[1..] { rb = s.handle_fragment("B", f, now); }
        assert_eq!(serde_json::from_slice::<Value>(&ra.unwrap()).unwrap()["protocol"], 3);
        assert_eq!(serde_json::from_slice::<Value>(&rb.unwrap()).unwrap()["state"], "DISCONNECTED");
        // Abandoned transfer is dropped on disconnect.
        assert_eq!(s.handle_fragment("A", &a[0], now), None);
        s.disconnect("A");
        let mut r = None;
        for f in &a[1..] { r = s.handle_fragment("A", f, now); }
        assert!(r.is_none(), "stale fragments after disconnect never complete a message");
        // Framing violations answer with an error instead of hanging.
        let bad = s.handle_fragment("C", &[1, 3, 2, b'x'], now).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bad).unwrap(), json!({"error":"malformed"}));
    }

    #[test]
    fn claim_not_acknowledged_if_not_persisted_and_reset_clears_state() {
        let now = Instant::now();
        let mut s = RpcService::new(OwnershipManager::new(Ownership::Unclaimed), FakeNetwork::default(), MemoryStore { fail: true, ..Default::default() });
        let mut p = Phone::new("p");
        let c = challenge_of(&p.send(&mut s, json!({"op":"pairing.challenge"}), now));
        let (phone_key, _) = client_claim(&c, "p").unwrap();
        let r = p.send(&mut s, json!({"op":"pairing.claim","owner_id":"p","phone_key":hex(&phone_key)}), now);
        assert_eq!(r, json!({"error":"storage_failed"}));
        assert_eq!(*s.ownership(), Ownership::Unclaimed);
        let mut t = svc(FakeNetwork::default());
        p.claim(&mut t, now);
        t.physical_reset().unwrap();
        assert_eq!(*t.ownership(), Ownership::Unclaimed);
        assert_eq!(t.store.saved.last(), Some(&Ownership::Unclaimed));
    }

    #[test]
    fn info_value_fits_one_minimum_mtu_read() {
        assert!(INFO_VALUE.len() <= MIN_ATT_PAYLOAD + 2, "ATT_MTU 23 read returns up to 22 bytes");
        let v: Value = serde_json::from_slice(INFO_VALUE).unwrap();
        assert_eq!(v["v"], PROTOCOL_VERSION);
    }

    #[cfg(unix)]
    #[test]
    fn file_store_is_durable_private_and_fails_safe() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("stephania-owner-{}", std::process::id()));
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

    #[test]
    fn management_enroll_issues_a_sealed_token_and_pin_only_to_the_owner() {
        let dir = std::env::temp_dir().join(format!("stephania-enroll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tokens = TokenStore { path: dir.join("tokens.json") };
        let mut s = svc(FakeNetwork::default()).with_management(Management { pki_dir: dir.join("pki"), tokens: tokens.clone(), otp: Some(OtpSource::Fixed(vec![9; 20])) });
        let mut phone = Phone::new("aa:bb");
        let now = Instant::now();
        let key = phone.claim(&mut s, now);

        // No PKI yet: fails closed, nothing issued.
        assert_eq!(phone.send(&mut s, json!({"op":"management.endpoint"}), now), json!({"available": false}));
        let n = phone.nonce(&mut s, now);
        let proof = operation_proof(&key, &n, "management.enroll", b"");
        assert_eq!(phone.send(&mut s, json!({"op":"management.enroll","proof":hex(&proof)}), now), json!({"error":"management_unavailable"}));
        assert_eq!(tokens.count(), 0);

        let (_, endpoint) = crate::pki::ensure(&dir.join("pki"), "stephania", &["192.168.0.9".parse().unwrap()], 9443, std::time::SystemTime::now()).unwrap();
        assert_eq!(phone.send(&mut s, json!({"op":"management.endpoint"}), now), json!({"available": true}));

        // Wrong key: rejected.
        let n = phone.nonce(&mut s, now);
        let bad = operation_proof(&[7u8; 32], &n, "management.enroll", b"");
        assert_eq!(phone.send(&mut s, json!({"op":"management.enroll","proof":hex(&bad)}), now), json!({"error":"bad_proof"}));

        let n = phone.nonce(&mut s, now);
        let proof = operation_proof(&key, &n, "management.enroll", b"");
        let req = json!({"op":"management.enroll","proof":hex(&proof)});
        let r = phone.send_with_id(&mut s, 77, &req, now);
        let sealed = crate::provisioning::unhex_vec(r["sealed"].as_str().unwrap()).unwrap();
        let plain: Value = serde_json::from_slice(&open(&key, &n, "management.enroll", &sealed).unwrap()).unwrap();
        assert_eq!(plain["ca_sha256"], json!(endpoint.ca_sha256));
        assert_eq!(plain["urls"], json!(endpoint.urls()));
        let token = plain["token"].as_str().unwrap();
        assert!(token.starts_with("t3b_"));
        assert_eq!(plain["otp_secret"], json!(crate::sshcert::base64_encode(&[9; 20])));
        assert!(!String::from_utf8_lossy(&sealed).contains(&token[4..]));
        // Identical retry after a lost indication: same token, issued once.
        assert_eq!(phone.send_with_id(&mut s, 77, &req, now), r);
        assert_eq!(tokens.count(), 1);

        // A factory reset revokes every LAN token.
        s.physical_reset().unwrap();
        assert_eq!(tokens.count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

}
