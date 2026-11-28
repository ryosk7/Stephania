//! Device-side provisioning core: BLE message framing, ownership claim and
//! Wi-Fi credential validation (docs/MASTER-ARCHITECTURE.md §15,
//! contracts/ble-protocol.md).
//!
//! This module is transport-independent. The BlueZ GATT server and the
//! NetworkManager D-Bus client are adapters around it and are not
//! implemented yet; everything here is exercised with virtual clocks and
//! in-memory transports.
//!
//! Ownership (protocol v3): the first phone to claim an unclaimed device
//! becomes its owner. The claim is an X25519 key agreement between a fresh
//! device key and a fresh phone key, so the owner key is never transmitted
//! and a passive listener cannot learn it. Physical access (re-flashing the
//! microSD card) is what resets ownership; see contracts/ble-protocol.md.

use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, X25519};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use std::collections::HashMap;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- framing

pub const MAX_MESSAGE_BYTES: usize = 4096;
/// The header's count byte allows 255; 255 × 17 bytes (minimum ATT payload)
/// covers a full 4096-byte message even at the minimum MTU.
pub const MAX_FRAGMENTS: usize = 255;
pub const FRAGMENT_HEADER_BYTES: usize = 3;
pub const REASSEMBLY_TIMEOUT: Duration = Duration::from_secs(10);
/// Smallest ATT MTU (23) leaves 20 bytes of characteristic payload.
pub const MIN_ATT_PAYLOAD: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    TooLarge,
    PayloadTooSmall,
    Malformed,
    Inconsistent,
    Duplicate,
}

/// Split `message` into characteristic writes of at most `att_payload`
/// bytes: `[message_id, index, count, ...chunk]`.
pub fn fragment(message_id: u8, message: &[u8], att_payload: usize) -> Result<Vec<Vec<u8>>, FrameError> {
    if att_payload < MIN_ATT_PAYLOAD {
        return Err(FrameError::PayloadTooSmall);
    }
    if message.is_empty() || message.len() > MAX_MESSAGE_BYTES {
        return Err(FrameError::TooLarge);
    }
    let chunk = att_payload - FRAGMENT_HEADER_BYTES;
    let count = message.len().div_ceil(chunk);
    if count > MAX_FRAGMENTS {
        return Err(FrameError::TooLarge);
    }
    Ok(message.chunks(chunk).enumerate().map(|(i, c)| {
        let mut f = Vec::with_capacity(FRAGMENT_HEADER_BYTES + c.len());
        f.extend_from_slice(&[message_id, i as u8, count as u8]);
        f.extend_from_slice(c);
        f
    }).collect())
}

struct Partial {
    count: usize,
    parts: Vec<Option<Vec<u8>>>,
    bytes: usize,
    started: Instant,
}

/// Bounded reassembly of fragmented writes. At most 4 messages may be in
/// flight; incomplete messages expire after [`REASSEMBLY_TIMEOUT`].
#[derive(Default)]
pub struct Reassembler {
    partial: HashMap<u8, Partial>,
}

impl Reassembler {
    pub const MAX_IN_FLIGHT: usize = 4;

    /// Feed one fragment. Returns the complete message once all parts arrived.
    /// Any error discards that message's state.
    pub fn push(&mut self, fragment: &[u8], now: Instant) -> Result<Option<Vec<u8>>, FrameError> {
        self.partial.retain(|_, p| now.duration_since(p.started) < REASSEMBLY_TIMEOUT);
        if fragment.len() <= FRAGMENT_HEADER_BYTES {
            return Err(FrameError::Malformed);
        }
        let (id, index, count) = (fragment[0], fragment[1] as usize, fragment[2] as usize);
        if count == 0 || count > MAX_FRAGMENTS || index >= count {
            self.partial.remove(&id);
            return Err(FrameError::Malformed);
        }
        if !self.partial.contains_key(&id) && self.partial.len() >= Self::MAX_IN_FLIGHT {
            return Err(FrameError::TooLarge);
        }
        let p = self.partial.entry(id).or_insert_with(|| Partial { count, parts: vec![None; count], bytes: 0, started: now });
        if p.count != count {
            self.partial.remove(&id);
            return Err(FrameError::Inconsistent);
        }
        let body = &fragment[FRAGMENT_HEADER_BYTES..];
        if let Some(stored) = &p.parts[index] {
            // An identical duplicate (a retransmitted write) is ignored; only
            // a conflicting one discards the message.
            if stored.as_slice() == body {
                return Ok(None);
            }
            self.partial.remove(&id);
            return Err(FrameError::Duplicate);
        }
        p.bytes += body.len();
        if p.bytes > MAX_MESSAGE_BYTES {
            self.partial.remove(&id);
            return Err(FrameError::TooLarge);
        }
        p.parts[index] = Some(body.to_vec());
        if p.parts.iter().all(Option::is_some) {
            let p = self.partial.remove(&id).expect("present");
            return Ok(Some(p.parts.into_iter().flatten().flatten().collect()));
        }
        Ok(None)
    }

    pub fn in_flight(&self) -> usize {
        self.partial.len()
    }
}

// -------------------------------------------------------------- ownership

pub const NONCE_BYTES: usize = 32;
/// X25519 public keys and the derived owner key.
pub const KEY_BYTES: usize = 32;
pub const PAIRING_TIMEOUT: Duration = Duration::from_secs(120);
/// Failed owner proofs one BLE peer may make before it is backed off.
pub const MAX_FAILED_ATTEMPTS: u32 = 5;
/// First backoff; it doubles with every further failure up to [`MAX_LOCKOUT`].
pub const LOCKOUT: Duration = Duration::from_secs(30);
pub const MAX_LOCKOUT: Duration = Duration::from_secs(300);
/// Peers whose failures are remembered (attacker-chosen addresses, so bounded).
pub const MAX_TRACKED_PEERS: usize = 32;
/// Outstanding operation nonces: per peer, and in total.
pub const OPERATION_NONCES_PER_PEER: usize = 2;
pub const MAX_OPERATION_NONCES: usize = 8;
pub const MAX_OWNER_ID_BYTES: usize = 64;

const OWNER_KEY_CONTEXT: &[u8] = b"stephania-owner-key-v2";
const CONFIRM_CONTEXT: &[u8] = b"stephania-claim-confirm-v1";
const OPERATION_CONTEXT: &[u8] = b"stephania-operation-v1";
const SEAL_CONTEXT: &[u8] = b"stephania-seal-v1";

/// Persisted ownership. Pairing is deliberately transient: a reboot during
/// pairing returns to the persisted state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ownership {
    Unclaimed,
    Claimed { owner_id: String, owner_key: [u8; 32] },
    RecoveryRequired,
}

impl Ownership {
    pub fn name(&self) -> &'static str {
        match self {
            Ownership::Unclaimed => "UNCLAIMED",
            Ownership::Claimed { .. } => "CLAIMED",
            Ownership::RecoveryRequired => "RECOVERY_REQUIRED",
        }
    }

    /// Serialise for the 0600 state file under /data/stephania/credentials.
    pub fn to_json(&self) -> String {
        match self {
            Ownership::Claimed { owner_id, owner_key } => serde_json::json!({
                "version": 1, "state": self.name(), "owner_id": owner_id, "owner_key": hex(owner_key)
            }).to_string(),
            _ => serde_json::json!({"version": 1, "state": self.name()}).to_string(),
        }
    }

    /// Parse the state file. Anything unreadable becomes RECOVERY_REQUIRED:
    /// a corrupt file must neither unclaim the device nor grant access.
    pub fn from_json(text: &str) -> Self {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return Ownership::RecoveryRequired };
        if v["version"] != 1 {
            return Ownership::RecoveryRequired;
        }
        match v["state"].as_str() {
            Some("UNCLAIMED") => Ownership::Unclaimed,
            Some("RECOVERY_REQUIRED") => Ownership::RecoveryRequired,
            Some("CLAIMED") => {
                let owner_id = v["owner_id"].as_str().filter(|s| valid_owner_id(s)).map(str::to_owned);
                let key = v["owner_key"].as_str().and_then(unhex::<32>);
                match (owner_id, key) {
                    (Some(owner_id), Some(owner_key)) => Ownership::Claimed { owner_id, owner_key },
                    _ => Ownership::RecoveryRequired,
                }
            }
            _ => Ownership::RecoveryRequired,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    NotClaimable,
    NoPairingInProgress,
    PairingExpired,
    LockedOut,
    InvalidOwnerId,
    BadProof,
    /// The phone's key-agreement public key was malformed or degenerate.
    BadKey,
    NotOwner,
    PhysicalConfirmationRequired,
    Random,
}

struct Pairing {
    nonce: [u8; NONCE_BYTES],
    issued: Instant,
    /// Fresh per challenge; consumed by the claim.
    private: EphemeralPrivateKey,
    public: [u8; KEY_BYTES],
}

/// What `pairing.challenge` hands the phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairingChallenge {
    pub nonce: [u8; NONCE_BYTES],
    pub device_key: [u8; KEY_BYTES],
}

struct OperationNonce {
    nonce: [u8; NONCE_BYTES],
    issued: Instant,
    peer: String,
}

struct PeerFailures {
    failures: u32,
    locked_until: Option<Instant>,
    last: Instant,
}

/// Failed owner proofs, counted per BLE peer with exponential backoff.
/// Nothing here is global, so a radio attacker failing proofs only delays
/// its own address and never the owner's.
#[derive(Default)]
struct Lockout {
    peers: HashMap<String, PeerFailures>,
}

impl Lockout {
    fn check(&self, peer: &str, now: Instant) -> Result<(), ClaimError> {
        match self.peers.get(peer).and_then(|p| p.locked_until) {
            Some(until) if now < until => Err(ClaimError::LockedOut),
            _ => Ok(()),
        }
    }

    fn record(&mut self, peer: &str, now: Instant) {
        if !self.peers.contains_key(peer) && self.peers.len() >= MAX_TRACKED_PEERS {
            // Forget the peer that failed longest ago.
            if let Some(oldest) = self.peers.iter().min_by_key(|(_, p)| p.last).map(|(k, _)| k.clone()) {
                self.peers.remove(&oldest);
            }
        }
        let p = self.peers.entry(peer.to_owned()).or_insert(PeerFailures { failures: 0, locked_until: None, last: now });
        p.failures += 1;
        p.last = now;
        if p.failures >= MAX_FAILED_ATTEMPTS {
            let doublings = (p.failures - MAX_FAILED_ATTEMPTS).min(16);
            p.locked_until = Some(now + (LOCKOUT * 2u32.pow(doublings)).min(MAX_LOCKOUT));
        }
    }

    fn success(&mut self, peer: &str) {
        self.peers.remove(peer);
    }
}

/// Ownership state machine. Time is injected so lockouts and expiry are
/// testable without sleeping.
pub struct OwnershipManager {
    state: Ownership,
    pairing: Option<Pairing>,
    operation_nonces: Vec<OperationNonce>,
    lockout: Lockout,
    rng: SystemRandom,
}

fn valid_owner_id(id: &str) -> bool {
    (1..=MAX_OWNER_ID_BYTES).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Length-prefixed concatenation so field boundaries are unambiguous.
fn encode(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.len() + 4).sum());
    for p in parts {
        out.extend_from_slice(&(p.len() as u32).to_be_bytes());
        out.extend_from_slice(p);
    }
    out
}

fn mac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), &encode(parts));
    let mut out = [0u8; 32];
    out.copy_from_slice(tag.as_ref());
    out
}

/// Constant-time verification via ring.
fn verify(key: &[u8], parts: &[&[u8]], tag: &[u8]) -> bool {
    hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, key), &encode(parts), tag).is_ok()
}

/// Owner key both sides derive from the X25519 shared secret, bound to the
/// challenge nonce, the owner id and both public keys; never transmitted.
pub fn derive_owner_key(shared: &[u8], nonce: &[u8], owner_id: &str, device_key: &[u8], phone_key: &[u8]) -> [u8; KEY_BYTES] {
    mac(shared, &[OWNER_KEY_CONTEXT, nonce, owner_id.as_bytes(), device_key, phone_key])
}

/// Returned by the device after a claim so the phone can check that both
/// sides derived the same owner key.
pub fn claim_confirmation(owner_key: &[u8], nonce: &[u8]) -> [u8; 32] {
    mac(owner_key, &[CONFIRM_CONTEXT, nonce])
}

/// Phone side of a claim (Rust clients and tests): a fresh key pair, the
/// public key to send, and the owner key.
pub fn client_claim(challenge: &PairingChallenge, owner_id: &str) -> Result<([u8; KEY_BYTES], [u8; KEY_BYTES]), ClaimError> {
    let rng = SystemRandom::new();
    let private = EphemeralPrivateKey::generate(&X25519, &rng).map_err(|_| ClaimError::Random)?;
    let mut public = [0u8; KEY_BYTES];
    public.copy_from_slice(private.compute_public_key().map_err(|_| ClaimError::Random)?.as_ref());
    let shared = agreement::agree_ephemeral(private, &UnparsedPublicKey::new(&X25519, challenge.device_key), |s| s.to_vec())
        .map_err(|_| ClaimError::BadKey)?;
    Ok((public, derive_owner_key(&shared, &challenge.nonce, owner_id, &challenge.device_key, &public)))
}

/// Client side of an authenticated operation (Wi-Fi config, reset request…).
pub fn operation_proof(owner_key: &[u8], nonce: &[u8], operation: &str, payload: &[u8]) -> [u8; 32] {
    mac(owner_key, &[OPERATION_CONTEXT, nonce, operation.as_bytes(), payload])
}

impl OwnershipManager {
    pub fn new(state: Ownership) -> Self {
        Self { state, pairing: None, operation_nonces: Vec::new(), lockout: Lockout::default(), rng: SystemRandom::new() }
    }

    pub fn state(&self) -> &Ownership {
        &self.state
    }

    /// Owner key of a claimed device (for unsealing operation payloads).
    pub fn owner_key(&self) -> Option<&[u8; 32]> {
        match &self.state { Ownership::Claimed { owner_key, .. } => Some(owner_key), _ => None }
    }

    pub fn is_pairing(&self) -> bool {
        self.pairing.is_some()
    }

    /// The state the BLE PairingChallenge characteristic reports.
    pub fn reported_state(&self) -> &'static str {
        if self.pairing.is_some() { "PAIRING" } else { self.state.name() }
    }

    fn nonce(&self) -> Result<[u8; NONCE_BYTES], ClaimError> {
        let mut n = [0u8; NONCE_BYTES];
        self.rng.fill(&mut n).map_err(|_| ClaimError::Random)?;
        Ok(n)
    }

    /// Start a claim: a fresh nonce and a fresh X25519 key. Only an
    /// unclaimed device can be claimed over BLE; an owned device is reset by
    /// re-flashing its card (or `factory_reset` with physical confirmation).
    pub fn begin_pairing(&mut self, now: Instant) -> Result<PairingChallenge, ClaimError> {
        if self.state != Ownership::Unclaimed {
            return Err(ClaimError::NotClaimable);
        }
        let nonce = self.nonce()?;
        let private = EphemeralPrivateKey::generate(&X25519, &self.rng).map_err(|_| ClaimError::Random)?;
        let mut public = [0u8; KEY_BYTES];
        public.copy_from_slice(private.compute_public_key().map_err(|_| ClaimError::Random)?.as_ref());
        self.pairing = Some(Pairing { nonce, issued: now, private, public });
        Ok(PairingChallenge { nonce, device_key: public })
    }

    /// Complete a claim with the phone's public key: the first valid claim
    /// wins. On success the new state (see `state()`) must be written durably
    /// before the returned confirmation is sent over BLE. A bad key ends the
    /// pairing but is not counted toward any lockout: it guesses nothing.
    pub fn complete_pairing(&mut self, owner_id: &str, phone_key: &[u8], now: Instant) -> Result<[u8; 32], ClaimError> {
        if self.state != Ownership::Unclaimed {
            return Err(ClaimError::NotClaimable);
        }
        let Some(pairing) = self.pairing.as_ref() else { return Err(ClaimError::NoPairingInProgress) };
        if now.duration_since(pairing.issued) >= PAIRING_TIMEOUT {
            self.pairing = None;
            return Err(ClaimError::PairingExpired);
        }
        if !valid_owner_id(owner_id) {
            return Err(ClaimError::InvalidOwnerId);
        }
        if phone_key.len() != KEY_BYTES {
            self.pairing = None;
            return Err(ClaimError::BadKey);
        }
        let pairing = self.pairing.take().expect("checked above");
        let device_key = pairing.public;
        // ring rejects the all-zero shared secret (low-order points).
        let Ok(shared) = agreement::agree_ephemeral(pairing.private, &UnparsedPublicKey::new(&X25519, phone_key), |s| s.to_vec()) else {
            return Err(ClaimError::BadKey);
        };
        let owner_key = derive_owner_key(&shared, &pairing.nonce, owner_id, &device_key, phone_key);
        self.state = Ownership::Claimed { owner_id: owner_id.to_owned(), owner_key };
        Ok(claim_confirmation(&owner_key, &pairing.nonce))
    }

    /// Roll back a claim that could not be persisted, so a retry works.
    pub fn abandon_claim(&mut self) {
        if matches!(self.state, Ownership::Claimed { .. }) {
            self.state = Ownership::Unclaimed;
        }
    }

    /// Issue a single-use nonce for an owner-authenticated operation.
    /// Several stay outstanding (a few per peer, a few in total), so a
    /// challenge from another central does not invalidate the owner's.
    pub fn operation_challenge(&mut self, peer: &str, now: Instant) -> Result<[u8; NONCE_BYTES], ClaimError> {
        if !matches!(self.state, Ownership::Claimed { .. }) {
            return Err(ClaimError::NotOwner);
        }
        let n = self.nonce()?;
        if self.operation_nonces.iter().filter(|o| o.peer == peer).count() >= OPERATION_NONCES_PER_PEER {
            let oldest = self.operation_nonces.iter().position(|o| o.peer == peer).expect("counted above");
            self.operation_nonces.remove(oldest);
        }
        if self.operation_nonces.len() >= MAX_OPERATION_NONCES {
            self.operation_nonces.remove(0);
        }
        self.operation_nonces.push(OperationNonce { nonce: n, issued: now, peer: peer.to_owned() });
        Ok(n)
    }

    /// Verify an owner-authenticated operation from `peer` against the
    /// outstanding nonces (from any peer, so a retry after reconnecting from
    /// a new address still works). The matching nonce is consumed, so proofs
    /// cannot be replayed. Returns it on success (used to unseal).
    ///
    /// Only a proof that matches no issued nonce counts as a failure: that is
    /// a guess at the owner key. Failures back off that peer only.
    pub fn authorize(&mut self, peer: &str, operation: &str, payload: &[u8], proof: &[u8], now: Instant) -> Result<[u8; NONCE_BYTES], ClaimError> {
        self.lockout.check(peer, now)?;
        let Ownership::Claimed { owner_key, .. } = &self.state else { return Err(ClaimError::NotOwner) };
        if self.operation_nonces.is_empty() {
            return Err(ClaimError::NoPairingInProgress);
        }
        let matched = self.operation_nonces.iter()
            .position(|o| verify(owner_key, &[OPERATION_CONTEXT, &o.nonce, operation.as_bytes(), payload], proof));
        let Some(i) = matched else {
            self.lockout.record(peer, now);
            return Err(ClaimError::BadProof);
        };
        let OperationNonce { nonce, issued, .. } = self.operation_nonces.remove(i);
        if now.duration_since(issued) >= PAIRING_TIMEOUT {
            return Err(ClaimError::PairingExpired);
        }
        self.lockout.success(peer);
        Ok(nonce)
    }

    /// Factory reset of ownership. Requires a physical confirmation (button
    /// held at boot) — remote ownership loss alone never unclaims a device.
    pub fn factory_reset(&mut self, physical_confirmation: bool) -> Result<&Ownership, ClaimError> {
        if !physical_confirmation {
            return Err(ClaimError::PhysicalConfirmationRequired);
        }
        self.state = Ownership::Unclaimed;
        self.pairing = None;
        self.operation_nonces.clear();
        self.lockout = Lockout::default();
        Ok(&self.state)
    }
}

/// Application-layer confidentiality for operation payloads (Wi-Fi PSKs),
/// so no BLE pairing/bonding is needed. ChaCha20-Poly1305 with
/// `key = MAC(owner_key, "stephania-seal-v1")`, nonce = first 12 bytes of the
/// single-use operation nonce, AAD = operation name.
pub fn seal(owner_key: &[u8], op_nonce: &[u8; NONCE_BYTES], operation: &str, plaintext: &[u8]) -> Vec<u8> {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
    let key = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &mac(owner_key, &[SEAL_CONTEXT])).expect("32-byte key"));
    let nonce = Nonce::try_assume_unique_for_key(&op_nonce[..12]).expect("12-byte nonce");
    let mut buf = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::from(operation.as_bytes()), &mut buf).expect("seal");
    buf
}

/// Inverse of [`seal`]; `None` if tampered or sealed for another nonce/operation.
pub fn open(owner_key: &[u8], op_nonce: &[u8; NONCE_BYTES], operation: &str, sealed: &[u8]) -> Option<Vec<u8>> {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
    let key = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &mac(owner_key, &[SEAL_CONTEXT])).ok()?);
    let nonce = Nonce::try_assume_unique_for_key(&op_nonce[..12]).ok()?;
    let mut buf = sealed.to_vec();
    let plain = key.open_in_place(nonce, Aad::from(operation.as_bytes()), &mut buf).ok()?;
    Some(plain.to_vec())
}

// --------------------------------------------------------------- network

/// Network state, deliberately independent of ownership: a failed join
/// never changes who owns the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkState {
    Disconnected,
    Scanning,
    Connecting,
    Connected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiSecurity {
    Open,
    WpaPsk(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiConfiguration {
    pub ssid: Vec<u8>,
    pub security: WifiSecurity,
}

/// Validate a WiFiConfiguration payload:
/// `{"ssid": "<utf8>", "security": "open"|"wpa-psk", "psk": "<8..63 printable or 64 hex>"}`.
pub fn parse_wifi_configuration(payload: &[u8]) -> Result<WifiConfiguration, &'static str> {
    let v: serde_json::Value = serde_json::from_slice(payload).map_err(|_| "invalid_json")?;
    let obj = v.as_object().ok_or("not_object")?;
    let ssid = obj.get("ssid").and_then(|s| s.as_str()).ok_or("ssid_required")?;
    if ssid.is_empty() || ssid.len() > 32 {
        return Err("invalid_ssid");
    }
    let security = match (obj.get("security").and_then(|s| s.as_str()), obj.get("psk")) {
        (Some("open"), None) if obj.len() == 2 => WifiSecurity::Open,
        (Some("wpa-psk"), Some(serde_json::Value::String(psk))) if obj.len() == 3 => {
            let printable = (8..=63).contains(&psk.len()) && psk.bytes().all(|b| (0x20..=0x7e).contains(&b));
            let raw = psk.len() == 64 && psk.bytes().all(|b| b.is_ascii_hexdigit());
            if !(printable || raw) {
                return Err("invalid_psk");
            }
            WifiSecurity::WpaPsk(psk.clone())
        }
        _ => return Err("invalid_security"),
    };
    Ok(WifiConfiguration { ssid: ssid.as_bytes().to_vec(), security })
}

// ------------------------------------------------------------------ utils

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

pub fn unhex_vec(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn fragmentation_round_trips_at_minimum_and_large_mtu() {
        let msg: Vec<u8> = (0..MAX_MESSAGE_BYTES).map(|i| (i * 31) as u8).collect();
        for att in [MIN_ATT_PAYLOAD, 64, 67, 182, 244, 509] {
            let fits = (att - FRAGMENT_HEADER_BYTES) * MAX_FRAGMENTS >= msg.len();
            let Ok(frags) = fragment(7, &msg, att) else {
                assert!(!fits, "att {att} should fit 4 KiB");
                continue;
            };
            assert!(fits);
            assert!(frags.iter().all(|f| f.len() <= att));
            let mut r = Reassembler::default();
            let now = Instant::now();
            // Out-of-order delivery is accepted.
            let mut result = None;
            for f in frags.iter().rev() { result = r.push(f, now).unwrap().or(result); }
            assert_eq!(result.unwrap(), msg, "att {att}");
            assert_eq!(r.in_flight(), 0);
        }
        let small = b"{\"ssid\":\"LT\",\"security\":\"open\"}";
        let frags = fragment(1, small, MIN_ATT_PAYLOAD).unwrap();
        let mut r = Reassembler::default();
        let mut out = None;
        for f in &frags { out = r.push(f, Instant::now()).unwrap(); }
        assert_eq!(out.unwrap(), small);
    }

    #[test]
    fn reassembly_rejects_malicious_fragments() {
        let now = Instant::now();
        let mut r = Reassembler::default();
        assert_eq!(r.push(&[1, 0, 0, 9], now), Err(FrameError::Malformed));
        assert_eq!(r.push(&[1, 2, 2, 9], now), Err(FrameError::Malformed));
        assert_eq!(r.push(&[1, 0, 2], now), Err(FrameError::Malformed));
        assert_eq!(r.push(&[1, 0, 2, 9], now), Ok(None));
        // Identical duplicate: ignored, the message is still assembled.
        assert_eq!(r.push(&[1, 0, 2, 9], now), Ok(None));
        assert_eq!(r.push(&[1, 1, 2, 7], now), Ok(Some(vec![9, 7])));
        assert_eq!(r.push(&[1, 0, 2, 9], now), Ok(None));
        assert_eq!(r.push(&[1, 0, 2, 8], now), Err(FrameError::Duplicate), "conflicting duplicate");
        assert_eq!(r.push(&[1, 1, 2, 7], now), Ok(None), "the message was discarded");
        r.push(&[1, 0, 2, 7], now).unwrap();
        assert_eq!(r.push(&[2, 0, 2, 9], now), Ok(None));
        assert_eq!(r.push(&[2, 1, 3, 9], now), Err(FrameError::Inconsistent));
        // Byte bound across fragments.
        let big = vec![0u8; 400];
        let mut f = vec![3, 0, MAX_FRAGMENTS as u8];
        f.extend_from_slice(&big);
        for i in 0..11u8 { f[1] = i; let res = r.push(&f, now); if i < 10 { assert_eq!(res, Ok(None)); } else { assert_eq!(res, Err(FrameError::TooLarge)); } }
        // In-flight bound.
        for id in 10..14 { r.push(&[id, 0, 2, 1], now).unwrap(); }
        assert_eq!(r.in_flight(), Reassembler::MAX_IN_FLIGHT);
        assert_eq!(r.push(&[20, 0, 2, 1], now), Err(FrameError::TooLarge));
        // Expiry frees state.
        assert_eq!(r.push(&[20, 0, 2, 1], now + REASSEMBLY_TIMEOUT), Ok(None));
        assert_eq!(r.in_flight(), 1);
        assert!(fragment(0, &[], 100).is_err());
        assert!(fragment(0, &[0; MAX_MESSAGE_BYTES + 1], 509).is_err());
        assert_eq!(fragment(0, b"x", 19), Err(FrameError::PayloadTooSmall));
    }

    fn claimed(m: &mut OwnershipManager, now: Instant) -> [u8; 32] {
        let challenge = m.begin_pairing(now).unwrap();
        let (phone_key, owner_key) = client_claim(&challenge, "iphone-1").unwrap();
        let confirm = m.complete_pairing("iphone-1", &phone_key, now).unwrap();
        assert_eq!(confirm, claim_confirmation(&owner_key, &challenge.nonce), "both sides derive the same key");
        owner_key
    }

    #[test]
    fn first_claim_wins_and_authorizes_operations() {
        let now = Instant::now();
        let mut m = OwnershipManager::new(Ownership::Unclaimed);
        assert_eq!(m.reported_state(), "UNCLAIMED");
        let key = claimed(&mut m, now);
        assert_eq!(m.reported_state(), "CLAIMED");
        assert!(matches!(m.state(), Ownership::Claimed { owner_key, .. } if *owner_key == key));
        // Cannot be re-claimed by someone else.
        assert_eq!(m.begin_pairing(now), Err(ClaimError::NotClaimable));
        let payload = br#"{"ssid":"LT","security":"open"}"#;
        let nonce = m.operation_challenge("p", now).unwrap();
        let proof = operation_proof(&key, &nonce, "wifi.configure", payload);
        assert_eq!(m.authorize("p", "wifi.configure", payload, &proof, now), Ok(nonce));
        // Replay fails: nonce is single-use.
        assert_eq!(m.authorize("p", "wifi.configure", payload, &proof, now), Err(ClaimError::NoPairingInProgress));
        // Proof is bound to operation and payload.
        let nonce = m.operation_challenge("p", now).unwrap();
        let proof = operation_proof(&key, &nonce, "wifi.configure", payload);
        assert_eq!(m.authorize("p", "wifi.configure", b"{}", &proof, now), Err(ClaimError::BadProof));
        assert_eq!(m.authorize("p", "factory.reset", payload, &proof, now), Err(ClaimError::BadProof));
        // A retry from a new address (reconnect) still finds the nonce.
        assert_eq!(m.authorize("p-new-address", "wifi.configure", payload, &proof, now), Ok(nonce));
        let nonce = m.operation_challenge("p", now).unwrap();
        let proof = operation_proof(&key, &nonce, "wifi.scan", b"");
        assert_eq!(m.authorize("p", "wifi.scan", b"", &proof, now + PAIRING_TIMEOUT), Err(ClaimError::PairingExpired));
    }

    #[test]
    fn another_centrals_challenges_and_failures_do_not_lock_the_owner_out() {
        let now = Instant::now();
        let mut m = OwnershipManager::new(Ownership::Unclaimed);
        let key = claimed(&mut m, now);
        // The owner asks for a nonce; an attacker then asks for many and
        // fails proofs until it is backed off.
        let owner_nonce = m.operation_challenge("owner", now).unwrap();
        for _ in 0..MAX_FAILED_ATTEMPTS {
            m.operation_challenge("attacker", now).unwrap();
            assert_eq!(m.authorize("attacker", "wifi.scan", b"", &[0; 32], now), Err(ClaimError::BadProof));
        }
        assert_eq!(m.authorize("attacker", "wifi.scan", b"", &[0; 32], now), Err(ClaimError::LockedOut));
        // The owner's nonce survived and its correct proof is accepted.
        let proof = operation_proof(&key, &owner_nonce, "wifi.scan", b"");
        assert_eq!(m.authorize("owner", "wifi.scan", b"", &proof, now), Ok(owner_nonce));
        // Exponential backoff with a cap, for the failing peer only.
        assert_eq!(m.authorize("attacker", "wifi.scan", b"", &[0; 32], now + LOCKOUT), Err(ClaimError::BadProof));
        assert_eq!(m.authorize("attacker", "wifi.scan", b"", &[0; 32], now + LOCKOUT * 2), Err(ClaimError::LockedOut));
        assert_eq!(m.authorize("attacker", "wifi.scan", b"", &[0; 32], now + LOCKOUT * 3), Err(ClaimError::BadProof));
        let mut t = now + LOCKOUT * 3;
        for _ in 0..10 {
            t += MAX_LOCKOUT;
            assert_eq!(m.authorize("attacker", "wifi.scan", b"", &[0; 32], t), Err(ClaimError::BadProof));
        }
        assert_eq!(m.authorize("attacker", "wifi.scan", b"", &[0; 32], t + MAX_LOCKOUT - Duration::from_secs(1)), Err(ClaimError::LockedOut));
        assert_eq!(m.authorize("attacker", "wifi.scan", b"", &[0; 32], t + MAX_LOCKOUT), Err(ClaimError::BadProof), "capped");
        // Challenge floods are bounded per peer and in total.
        for _ in 0..20 { m.operation_challenge("attacker", t).unwrap(); }
        assert_eq!(m.operation_nonces.len(), OPERATION_NONCES_PER_PEER);
        for i in 0..20 { m.operation_challenge(&format!("rotating-{i}"), t).unwrap(); }
        assert_eq!(m.operation_nonces.len(), MAX_OPERATION_NONCES);
        // Failures from many addresses stay bounded too.
        for i in 0..100 { let _ = m.authorize(&format!("rotating-{i}"), "wifi.scan", b"", &[0; 32], t); }
        assert!(m.lockout.peers.len() <= MAX_TRACKED_PEERS);
    }

    #[test]
    fn bad_keys_are_refused_without_locking_anyone_out() {
        let now = Instant::now();
        let mut m = OwnershipManager::new(Ownership::Unclaimed);
        for _ in 0..MAX_FAILED_ATTEMPTS * 2 {
            m.begin_pairing(now).unwrap();
            // All-zero is a low-order point: the shared secret would be zero.
            assert_eq!(m.complete_pairing("attacker", &[0; 32], now), Err(ClaimError::BadKey));
            assert!(!m.is_pairing(), "a bad key ends that pairing");
        }
        assert_eq!(*m.state(), Ownership::Unclaimed);
        claimed(&mut m, now);
        assert_eq!(m.reported_state(), "CLAIMED");
    }

    #[test]
    fn claim_is_bound_to_the_latest_challenge_and_expires() {
        let now = Instant::now();
        let mut m = OwnershipManager::new(Ownership::Unclaimed);
        let old = m.begin_pairing(now).unwrap();
        let fresh = m.begin_pairing(now).unwrap();
        assert_ne!(old.nonce, fresh.nonce);
        assert_ne!(old.device_key, fresh.device_key, "a fresh device key per challenge");
        // A phone that answered the old challenge derives a different key.
        let (phone_key, stale_key) = client_claim(&old, "p").unwrap();
        let confirm = m.complete_pairing("p", &phone_key, now).unwrap();
        assert_ne!(confirm, claim_confirmation(&stale_key, &old.nonce));
        let mut m = OwnershipManager::new(Ownership::Unclaimed);
        let c = m.begin_pairing(now).unwrap();
        let (k, _) = client_claim(&c, "p").unwrap();
        assert_eq!(m.complete_pairing("p", &k, now + PAIRING_TIMEOUT), Err(ClaimError::PairingExpired));
        assert_eq!(m.complete_pairing("p", &k, now), Err(ClaimError::NoPairingInProgress));
        let c = m.begin_pairing(now).unwrap();
        let (k, _) = client_claim(&c, "bad id").unwrap();
        assert_eq!(m.complete_pairing("bad id", &k, now), Err(ClaimError::InvalidOwnerId));
        assert_eq!(m.complete_pairing("p", &k[..31], now), Err(ClaimError::BadKey));
    }

    #[test]
    fn unpersisted_claim_can_be_abandoned() {
        let now = Instant::now();
        let mut m = OwnershipManager::new(Ownership::Unclaimed);
        claimed(&mut m, now);
        m.abandon_claim();
        assert_eq!(*m.state(), Ownership::Unclaimed);
        claimed(&mut m, now);
    }

    #[test]
    fn reset_requires_physical_confirmation_and_network_failure_keeps_owner() {
        let now = Instant::now();
        let mut m = OwnershipManager::new(Ownership::Unclaimed);
        claimed(&mut m, now);
        // Network state is a separate machine; nothing here touches ownership.
        let _network = NetworkState::Failed;
        assert_eq!(m.reported_state(), "CLAIMED");
        assert_eq!(m.factory_reset(false), Err(ClaimError::PhysicalConfirmationRequired));
        assert_eq!(m.reported_state(), "CLAIMED");
        assert_eq!(m.factory_reset(true).unwrap(), &Ownership::Unclaimed);
        let mut r = OwnershipManager::new(Ownership::RecoveryRequired);
        assert_eq!(r.begin_pairing(now), Err(ClaimError::NotClaimable));
        assert_eq!(r.operation_challenge("p", now), Err(ClaimError::NotOwner));
    }

    #[test]
    fn persisted_state_round_trips_and_corruption_never_unclaims() {
        let state = Ownership::Claimed { owner_id: "iphone-1".into(), owner_key: [7; 32] };
        assert_eq!(Ownership::from_json(&state.to_json()), state);
        assert_eq!(Ownership::from_json(&Ownership::Unclaimed.to_json()), Ownership::Unclaimed);
        for bad in ["", "{", r#"{"version":2,"state":"UNCLAIMED"}"#, r#"{"version":1,"state":"CLAIMED","owner_id":"x"}"#,
                    r#"{"version":1,"state":"CLAIMED","owner_id":"x","owner_key":"zz"}"#, r#"{"version":1,"state":"PAIRING"}"#] {
            assert_eq!(Ownership::from_json(bad), Ownership::RecoveryRequired, "{bad}");
        }
    }

    #[test]
    fn wifi_configuration_validation() {
        let ok = |s: &str| parse_wifi_configuration(s.as_bytes());
        assert_eq!(ok(r#"{"ssid":"LT 会場","security":"open"}"#).unwrap().security, WifiSecurity::Open);
        assert!(ok(r#"{"ssid":"LT","security":"wpa-psk","psk":"password"}"#).is_ok());
        assert!(ok(&format!(r#"{{"ssid":"LT","security":"wpa-psk","psk":"{}"}}"#, "a".repeat(64))).is_ok());
        for (bad, why) in [
            (r#"{"ssid":"","security":"open"}"#, "invalid_ssid"),
            (&format!(r#"{{"ssid":"{}","security":"open"}}"#, "a".repeat(33)), "invalid_ssid"),
            (r#"{"ssid":"LT","security":"wpa-psk","psk":"short"}"#, "invalid_psk"),
            (&format!(r#"{{"ssid":"LT","security":"wpa-psk","psk":"{}"}}"#, "g".repeat(64)), "invalid_psk"),
            (r#"{"ssid":"LT","security":"wpa-psk","psk":"tab\tpassword"}"#, "invalid_psk"),
            (r#"{"ssid":"LT","security":"open","psk":"password"}"#, "invalid_security"),
            (r#"{"ssid":"LT","security":"wep"}"#, "invalid_security"),
            (r#"{"ssid":"LT","security":"open","x":1}"#, "invalid_security"),
            ("[]", "not_object"),
        ] {
            assert_eq!(ok(bad), Err(why), "{bad}");
        }
    }

    #[test]
    fn seal_round_trip_and_binding() {
        let key = [9u8; 32];
        let nonce = [3u8; 32];
        let sealed = seal(&key, &nonce, "wifi.configure", b"secret");
        assert_eq!(sealed.len(), 6 + 16);
        assert_eq!(open(&key, &nonce, "wifi.configure", &sealed).as_deref(), Some(&b"secret"[..]));
        assert_eq!(open(&key, &nonce, "wifi.scan", &sealed), None, "bound to operation");
        assert_eq!(open(&key, &[4u8; 32], "wifi.configure", &sealed), None, "bound to nonce");
        let mut tampered = sealed.clone();
        tampered[0] ^= 1;
        assert_eq!(open(&key, &nonce, "wifi.configure", &tampered), None);
    }

    #[test]
    fn hex_helpers() {
        assert_eq!(hex(&[0, 255, 16]), "00ff10");
        assert_eq!(unhex::<3>("00ff10"), Some([0, 255, 16]));
        assert_eq!(unhex::<3>("00ff1"), None);
        assert_eq!(unhex::<1>("é"), None);
    }
}

#[cfg(test)]
mod interop {
    use super::*;

    fn bytes(v: &serde_json::Value) -> Vec<u8> {
        let s = v.as_str().unwrap();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// ChaCha20-Poly1305 seal vectors computed with Python `cryptography`.
    #[test]
    fn seal_vectors_match_independent_implementation() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/fixtures/ble/seal-vectors.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        for v in doc["vectors"].as_array().unwrap() {
            let key = bytes(&v["owner_key"]);
            let nonce: [u8; 32] = bytes(&v["operation_nonce"]).try_into().unwrap();
            let op = v["operation"].as_str().unwrap();
            assert_eq!(hex(&seal(&key, &nonce, op, &bytes(&v["plaintext_hex"]))), v["sealed_hex"]);
            assert_eq!(open(&key, &nonce, op, &bytes(&v["sealed_hex"])), Some(bytes(&v["plaintext_hex"])));
        }
    }

    /// X25519 claim and HMAC vectors computed independently in Python
    /// (contracts/fixtures/ble/generate.py). The device side is replayed with
    /// the vector's private key through ring's fixed-randomness test source.
    #[test]
    #[allow(deprecated)]
    fn claim_vectors_match_independent_implementation() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/fixtures/ble/claim-vectors.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let vectors = doc["vectors"].as_array().unwrap();
        assert!(vectors.len() >= 2);
        for v in vectors {
            let device_private = bytes(&v["device_private"]);
            let rng = ring::test::rand::FixedSliceRandom { bytes: &device_private };
            let private = EphemeralPrivateKey::generate(&X25519, &rng).unwrap();
            assert_eq!(hex(private.compute_public_key().unwrap().as_ref()), v["device_key"]);
            let phone_key = bytes(&v["phone_key"]);
            let shared = agreement::agree_ephemeral(private, &UnparsedPublicKey::new(&X25519, &phone_key), |s| s.to_vec()).unwrap();
            assert_eq!(hex(&shared), v["shared"]);
            let (nonce, owner) = (bytes(&v["nonce"]), v["owner_id"].as_str().unwrap());
            let key = derive_owner_key(&shared, &nonce, owner, &bytes(&v["device_key"]), &phone_key);
            assert_eq!(hex(&key), v["owner_key"]);
            assert_eq!(hex(&claim_confirmation(&key, &nonce)), v["confirm"]);
            let proof = operation_proof(&key, &bytes(&v["operation_nonce"]), v["operation"].as_str().unwrap(), &bytes(&v["payload_hex"]));
            assert_eq!(hex(&proof), v["operation_proof"]);
        }
    }
}
