//! Device-side provisioning core: BLE message framing, ownership claim and
//! Wi-Fi credential validation (docs/MASTER-ARCHITECTURE.md §15,
//! contracts/ble-protocol.md).
//!
//! This module is transport-independent. The BlueZ GATT server and the
//! NetworkManager D-Bus client are adapters around it and are not
//! implemented yet; everything here is exercised with virtual clocks and
//! in-memory transports.
//!
//! Security status: proof-of-concept. Ownership is proven with an HMAC
//! challenge-response over a 128-bit per-device setup secret (no shared
//! default credential, no secret ever transmitted). How that secret reaches
//! the owner without a monitor or keyboard (printed label / QR) is an open
//! security PoC (risk R02); until it is resolved this must not be described
//! as production-grade enrollment.

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
        if p.parts[index].is_some() {
            self.partial.remove(&id);
            return Err(FrameError::Duplicate);
        }
        let body = &fragment[FRAGMENT_HEADER_BYTES..];
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

pub const SETUP_SECRET_BYTES: usize = 16;
pub const NONCE_BYTES: usize = 32;
pub const PAIRING_TIMEOUT: Duration = Duration::from_secs(120);
pub const MAX_FAILED_ATTEMPTS: u32 = 5;
pub const LOCKOUT: Duration = Duration::from_secs(300);
pub const MAX_OWNER_ID_BYTES: usize = 64;

const CLAIM_CONTEXT: &[u8] = b"t3box-claim-v1";
const OWNER_KEY_CONTEXT: &[u8] = b"t3box-owner-key-v1";
const OPERATION_CONTEXT: &[u8] = b"t3box-operation-v1";
const SEAL_CONTEXT: &[u8] = b"t3box-seal-v1";

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

    /// Serialise for the 0600 state file under /data/t3box/credentials.
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
    NotOwner,
    PhysicalConfirmationRequired,
    Random,
}

struct Pairing {
    nonce: [u8; NONCE_BYTES],
    issued: Instant,
}

/// Ownership state machine. Time is injected so lockouts and expiry are
/// testable without sleeping.
pub struct OwnershipManager {
    setup_secret: [u8; SETUP_SECRET_BYTES],
    state: Ownership,
    pairing: Option<Pairing>,
    operation_nonce: Option<([u8; NONCE_BYTES], Instant)>,
    failures: u32,
    locked_until: Option<Instant>,
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

/// Client side of the claim proof (used by the phone app and by tests).
pub fn claim_proof(setup_secret: &[u8], nonce: &[u8], owner_id: &str) -> [u8; 32] {
    mac(setup_secret, &[CLAIM_CONTEXT, nonce, owner_id.as_bytes()])
}

/// Owner key both sides derive after a successful claim; never transmitted.
pub fn derive_owner_key(setup_secret: &[u8], nonce: &[u8], owner_id: &str) -> [u8; 32] {
    mac(setup_secret, &[OWNER_KEY_CONTEXT, nonce, owner_id.as_bytes()])
}

/// Client side of an authenticated operation (Wi-Fi config, reset request…).
pub fn operation_proof(owner_key: &[u8], nonce: &[u8], operation: &str, payload: &[u8]) -> [u8; 32] {
    mac(owner_key, &[OPERATION_CONTEXT, nonce, operation.as_bytes(), payload])
}

impl OwnershipManager {
    pub fn new(setup_secret: [u8; SETUP_SECRET_BYTES], state: Ownership) -> Self {
        Self { setup_secret, state, pairing: None, operation_nonce: None, failures: 0, locked_until: None, rng: SystemRandom::new() }
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

    fn check_lockout(&mut self, now: Instant) -> Result<(), ClaimError> {
        match self.locked_until {
            Some(until) if now < until => Err(ClaimError::LockedOut),
            Some(_) => { self.locked_until = None; self.failures = 0; Ok(()) }
            None => Ok(()),
        }
    }

    fn record_failure(&mut self, now: Instant) {
        self.failures += 1;
        self.pairing = None;
        if self.failures >= MAX_FAILED_ATTEMPTS {
            self.locked_until = Some(now + LOCKOUT);
        }
    }

    /// Start a claim. Only an unclaimed device can be claimed over BLE;
    /// a claimed or recovery-required device needs factory reset first.
    pub fn begin_pairing(&mut self, now: Instant) -> Result<[u8; NONCE_BYTES], ClaimError> {
        self.check_lockout(now)?;
        if self.state != Ownership::Unclaimed {
            return Err(ClaimError::NotClaimable);
        }
        let nonce = self.nonce()?;
        self.pairing = Some(Pairing { nonce, issued: now });
        Ok(nonce)
    }

    /// Complete a claim. On success returns the new persisted state, which
    /// the caller must write durably before acknowledging over BLE.
    pub fn complete_pairing(&mut self, owner_id: &str, proof: &[u8], now: Instant) -> Result<&Ownership, ClaimError> {
        self.check_lockout(now)?;
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
        let nonce = pairing.nonce;
        if !verify(&self.setup_secret, &[CLAIM_CONTEXT, &nonce, owner_id.as_bytes()], proof) {
            self.record_failure(now);
            return Err(ClaimError::BadProof);
        }
        self.pairing = None;
        self.failures = 0;
        self.state = Ownership::Claimed { owner_id: owner_id.to_owned(), owner_key: derive_owner_key(&self.setup_secret, &nonce, owner_id) };
        Ok(&self.state)
    }

    /// Issue a single-use nonce for an owner-authenticated operation.
    pub fn operation_challenge(&mut self, now: Instant) -> Result<[u8; NONCE_BYTES], ClaimError> {
        if !matches!(self.state, Ownership::Claimed { .. }) {
            return Err(ClaimError::NotOwner);
        }
        let n = self.nonce()?;
        self.operation_nonce = Some((n, now));
        Ok(n)
    }

    /// Verify an owner-authenticated operation. The nonce is consumed whether
    /// or not verification succeeds, so proofs cannot be replayed.
    /// Returns the consumed operation nonce on success (used to unseal).
    pub fn authorize(&mut self, operation: &str, payload: &[u8], proof: &[u8], now: Instant) -> Result<[u8; NONCE_BYTES], ClaimError> {
        self.check_lockout(now)?;
        let Ownership::Claimed { owner_key, .. } = &self.state else { return Err(ClaimError::NotOwner) };
        let Some((nonce, issued)) = self.operation_nonce.take() else { return Err(ClaimError::NoPairingInProgress) };
        if now.duration_since(issued) >= PAIRING_TIMEOUT {
            return Err(ClaimError::PairingExpired);
        }
        if verify(owner_key, &[OPERATION_CONTEXT, &nonce, operation.as_bytes(), payload], proof) {
            self.failures = 0;
            Ok(nonce)
        } else {
            self.record_failure(now);
            Err(ClaimError::BadProof)
        }
    }

    /// Factory reset of ownership. Requires a physical confirmation (button
    /// held at boot) — remote ownership loss alone never unclaims a device.
    pub fn factory_reset(&mut self, physical_confirmation: bool) -> Result<&Ownership, ClaimError> {
        if !physical_confirmation {
            return Err(ClaimError::PhysicalConfirmationRequired);
        }
        self.state = Ownership::Unclaimed;
        self.pairing = None;
        self.operation_nonce = None;
        self.failures = 0;
        self.locked_until = None;
        Ok(&self.state)
    }
}

/// Application-layer confidentiality for operation payloads (Wi-Fi PSKs),
/// so no BLE pairing/bonding is needed. ChaCha20-Poly1305 with
/// `key = MAC(owner_key, "t3box-seal-v1")`, nonce = first 12 bytes of the
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

/// Create the per-device setup secret on first boot (0600, never
/// overwritten). Returns the existing secret on later boots.
#[cfg(unix)]
pub fn load_or_create_setup_secret(path: &std::path::Path) -> std::io::Result<[u8; SETUP_SECRET_BYTES]> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Ok(text) = std::fs::read_to_string(path) {
        return unhex::<SETUP_SECRET_BYTES>(text.trim())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "corrupt setup secret"));
    }
    let mut secret = [0u8; SETUP_SECRET_BYTES];
    SystemRandom::new().fill(&mut secret).map_err(|_| std::io::Error::other("random source unavailable"))?;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.write_all(hex(&secret).as_bytes())?;
    f.sync_all()?;
    // hard_link fails if another process created the secret meanwhile.
    std::fs::hard_link(&tmp, path)?;
    std::fs::remove_file(&tmp)?;
    Ok(secret)
}

// ------------------------------------------------------------ setup file

/// Setup URI carried by the QR code and by the boot-partition setup file:
/// `t3box-setup:v1:<32 hex secret>:<first 8 hex of SHA-256(secret)>`.
/// The secret is generated in the owner's browser by the static setup page
/// (tools/setup-page); the device imports it on first boot.
pub fn parse_setup_uri(text: &str) -> Option<[u8; SETUP_SECRET_BYTES]> {
    let lower = text.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("t3box-setup:v1:")?;
    let (secret_hex, check) = rest.split_once(':')?;
    let secret = unhex::<SETUP_SECRET_BYTES>(secret_hex)?;
    if secret.iter().all(|&b| b == 0) || check.len() != 8 {
        return None;
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, &secret);
    (hex(&digest.as_ref()[..4]) == check).then_some(secret)
}

pub fn setup_uri(secret: &[u8; SETUP_SECRET_BYTES]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, secret);
    format!("t3box-setup:v1:{}:{}", hex(secret), hex(&digest.as_ref()[..4]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupImport {
    /// No setup file present.
    Absent,
    /// Secret replaced (device was UNCLAIMED).
    Imported,
    /// Device already has an owner; file discarded, secret unchanged.
    IgnoredClaimed,
    /// File unreadable or malformed; discarded.
    Invalid,
}

/// Import the boot-partition setup file. Only an UNCLAIMED device accepts a
/// new secret, so re-inserting an SD card can never re-key an owned device.
/// The file is overwritten and removed in every case it exists (best effort:
/// FAT32 provides no secure deletion; documented as residual risk).
#[cfg(unix)]
pub fn import_setup_file(setup_file: &std::path::Path, secret_path: &std::path::Path, ownership: &Ownership) -> std::io::Result<SetupImport> {
    let text = match std::fs::read(setup_file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SetupImport::Absent),
        Err(_) => { discard(setup_file); return Ok(SetupImport::Invalid) }
    };
    let parsed = (text.len() <= 256).then(|| std::str::from_utf8(&text).ok()).flatten().and_then(parse_setup_uri);
    let outcome = match parsed {
        None => SetupImport::Invalid,
        Some(_) if *ownership != Ownership::Unclaimed => SetupImport::IgnoredClaimed,
        Some(secret) => { store_setup_secret(secret_path, &secret)?; SetupImport::Imported }
    };
    discard(setup_file);
    Ok(outcome)
}

#[cfg(unix)]
fn discard(path: &std::path::Path) {
    use std::io::Write;
    if let Ok(len) = std::fs::metadata(path).map(|m| m.len() as usize) {
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(path) {
            let _ = f.write_all(&vec![0u8; len]);
            let _ = f.sync_all();
        }
    }
    let _ = std::fs::remove_file(path);
}

/// Atomically replace the setup secret (0600, fsync, rename).
#[cfg(unix)]
pub fn store_setup_secret(path: &std::path::Path, secret: &[u8; SETUP_SECRET_BYTES]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("import");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.write_all(hex(secret).as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() { std::fs::File::open(dir)?.sync_all()?; }
    Ok(())
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

    const SECRET: [u8; 16] = *b"0123456789abcdef";

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
        assert_eq!(r.push(&[1, 0, 2, 9], now), Err(FrameError::Duplicate));
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
        let nonce = m.begin_pairing(now).unwrap();
        let proof = claim_proof(&SECRET, &nonce, "iphone-1");
        m.complete_pairing("iphone-1", &proof, now).unwrap();
        derive_owner_key(&SECRET, &nonce, "iphone-1")
    }

    #[test]
    fn claim_with_setup_secret_and_authorize_operations() {
        let now = Instant::now();
        let mut m = OwnershipManager::new(SECRET, Ownership::Unclaimed);
        assert_eq!(m.reported_state(), "UNCLAIMED");
        let key = claimed(&mut m, now);
        assert_eq!(m.reported_state(), "CLAIMED");
        assert!(matches!(m.state(), Ownership::Claimed { owner_key, .. } if *owner_key == key));
        // Cannot be re-claimed by someone else.
        assert_eq!(m.begin_pairing(now), Err(ClaimError::NotClaimable));
        let payload = br#"{"ssid":"LT","security":"open"}"#;
        let nonce = m.operation_challenge(now).unwrap();
        let proof = operation_proof(&key, &nonce, "wifi.configure", payload);
        assert_eq!(m.authorize("wifi.configure", payload, &proof, now), Ok(nonce));
        // Replay fails: nonce is single-use.
        assert_eq!(m.authorize("wifi.configure", payload, &proof, now), Err(ClaimError::NoPairingInProgress));
        // Proof is bound to operation and payload.
        let nonce = m.operation_challenge(now).unwrap();
        let proof = operation_proof(&key, &nonce, "wifi.configure", payload);
        assert_eq!(m.authorize("wifi.configure", b"{}", &proof, now), Err(ClaimError::BadProof));
        let nonce = m.operation_challenge(now).unwrap();
        let proof = operation_proof(&key, &nonce, "wifi.configure", payload);
        assert_eq!(m.authorize("factory.reset", payload, &proof, now), Err(ClaimError::BadProof));
    }

    #[test]
    fn wrong_secret_locks_out_after_repeated_failures() {
        let now = Instant::now();
        let mut m = OwnershipManager::new(SECRET, Ownership::Unclaimed);
        for _ in 0..MAX_FAILED_ATTEMPTS {
            let nonce = m.begin_pairing(now).unwrap();
            let proof = claim_proof(b"wrong-secret-xxx", &nonce, "attacker");
            assert_eq!(m.complete_pairing("attacker", &proof, now), Err(ClaimError::BadProof));
        }
        assert_eq!(m.begin_pairing(now), Err(ClaimError::LockedOut));
        assert_eq!(*m.state(), Ownership::Unclaimed);
        // Lockout expires; the legitimate owner can then claim.
        claimed(&mut m, now + LOCKOUT);
        assert_eq!(m.reported_state(), "CLAIMED");
    }

    #[test]
    fn pairing_expires_and_proof_is_bound_to_nonce_and_owner() {
        let now = Instant::now();
        let mut m = OwnershipManager::new(SECRET, Ownership::Unclaimed);
        let old = m.begin_pairing(now).unwrap();
        let fresh = m.begin_pairing(now).unwrap();
        assert_ne!(old, fresh);
        assert_eq!(m.complete_pairing("p", &claim_proof(&SECRET, &old, "p"), now), Err(ClaimError::BadProof));
        let n = m.begin_pairing(now).unwrap();
        assert_eq!(m.complete_pairing("q", &claim_proof(&SECRET, &n, "p"), now), Err(ClaimError::BadProof));
        let n = m.begin_pairing(now).unwrap();
        assert_eq!(m.complete_pairing("p", &claim_proof(&SECRET, &n, "p"), now + PAIRING_TIMEOUT), Err(ClaimError::PairingExpired));
        assert_eq!(m.complete_pairing("p", &[0; 32], now), Err(ClaimError::NoPairingInProgress));
        let n = m.begin_pairing(now).unwrap();
        assert_eq!(m.complete_pairing("bad id", &claim_proof(&SECRET, &n, "bad id"), now), Err(ClaimError::InvalidOwnerId));
    }

    #[test]
    fn reset_requires_physical_confirmation_and_network_failure_keeps_owner() {
        let now = Instant::now();
        let mut m = OwnershipManager::new(SECRET, Ownership::Unclaimed);
        claimed(&mut m, now);
        // Network state is a separate machine; nothing here touches ownership.
        let _network = NetworkState::Failed;
        assert_eq!(m.reported_state(), "CLAIMED");
        assert_eq!(m.factory_reset(false), Err(ClaimError::PhysicalConfirmationRequired));
        assert_eq!(m.reported_state(), "CLAIMED");
        assert_eq!(m.factory_reset(true).unwrap(), &Ownership::Unclaimed);
        let mut r = OwnershipManager::new(SECRET, Ownership::RecoveryRequired);
        assert_eq!(r.begin_pairing(now), Err(ClaimError::NotClaimable));
        assert_eq!(r.operation_challenge(now), Err(ClaimError::NotOwner));
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

    #[cfg(unix)]
    #[test]
    fn setup_secret_is_created_once_with_private_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("t3box-secret-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("setup-secret");
        let a = load_or_create_setup_secret(&path).unwrap();
        let b = load_or_create_setup_secret(&path).unwrap();
        assert_eq!(a, b);
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::write(&path, "nothex").unwrap();
        assert!(load_or_create_setup_secret(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
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
mod setup_tests {
    use super::*;

    fn fixtures() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/fixtures/ble/setup-uris.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn shared_setup_uri_fixtures() {
        let f = fixtures();
        for v in f["valid"].as_array().unwrap() {
            let secret = parse_setup_uri(v["uri"].as_str().unwrap()).unwrap_or_else(|| panic!("{v}"));
            assert_eq!(hex(&secret), v["secret"]);
            assert_eq!(parse_setup_uri(&setup_uri(&secret)), Some(secret));
        }
        for v in f["invalid"].as_array().unwrap() {
            assert_eq!(parse_setup_uri(v.as_str().unwrap()), None, "{v}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn import_only_when_unclaimed_and_always_discards_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("t3box-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (file, secret_path) = (dir.join("t3box-setup.txt"), dir.join("setup-secret"));
        let original = load_or_create_setup_secret(&secret_path).unwrap();
        let new = *b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10";
        assert_eq!(import_setup_file(&file, &secret_path, &Ownership::Unclaimed).unwrap(), SetupImport::Absent);

        let claimed = Ownership::Claimed { owner_id: "o".into(), owner_key: [1; 32] };
        std::fs::write(&file, setup_uri(&new)).unwrap();
        assert_eq!(import_setup_file(&file, &secret_path, &claimed).unwrap(), SetupImport::IgnoredClaimed);
        assert!(!file.exists());
        assert_eq!(load_or_create_setup_secret(&secret_path).unwrap(), original, "owned device is never re-keyed");

        std::fs::write(&file, setup_uri(&new)).unwrap();
        assert_eq!(import_setup_file(&file, &secret_path, &Ownership::RecoveryRequired).unwrap(), SetupImport::IgnoredClaimed);

        std::fs::write(&file, "t3box-setup:v1:garbage").unwrap();
        assert_eq!(import_setup_file(&file, &secret_path, &Ownership::Unclaimed).unwrap(), SetupImport::Invalid);
        assert!(!file.exists());

        std::fs::write(&file, format!("{}\n", setup_uri(&new))).unwrap();
        assert_eq!(import_setup_file(&file, &secret_path, &Ownership::Unclaimed).unwrap(), SetupImport::Imported);
        assert!(!file.exists());
        assert_eq!(load_or_create_setup_secret(&secret_path).unwrap(), new);
        assert_eq!(std::fs::metadata(&secret_path).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
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

    /// Vectors computed by an independent implementation (Python hmac).
    #[test]
    fn hmac_vectors_match_independent_implementation() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/fixtures/ble/hmac-vectors.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let vectors = doc["vectors"].as_array().unwrap();
        assert!(vectors.len() >= 2);
        for v in vectors {
            let (secret, nonce, owner) = (bytes(&v["setup_secret"]), bytes(&v["nonce"]), v["owner_id"].as_str().unwrap());
            assert_eq!(hex(&claim_proof(&secret, &nonce, owner)), v["claim_proof"]);
            let key = derive_owner_key(&secret, &nonce, owner);
            assert_eq!(hex(&key), v["owner_key"]);
            let proof = operation_proof(&key, &bytes(&v["operation_nonce"]), v["operation"].as_str().unwrap(), &bytes(&v["payload_hex"]));
            assert_eq!(hex(&proof), v["operation_proof"]);
        }
    }
}
