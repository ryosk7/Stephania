//! Device PKI for trusted LAN TLS (docs/MASTER-ARCHITECTURE.md §26).
//!
//! Each device creates its own CA on first use. The phone learns the CA's
//! SHA-256 over authenticated BLE (`management.endpoint`) and pins it; no
//! public CA is involved and no browser warning is clicked through.
//!
//! The CA is limited so that a PC that installs it as a root trusts as
//! little as possible, even if the CA key were stolen from the SD card:
//! - extended key usage `serverAuth` only (no code signing, S/MIME or
//!   client authentication on platforms that enforce EKU chaining);
//! - path length 0 (no subordinate CAs), key usage `keyCertSign`/`cRLSign`;
//! - name constraints: DNS `<host>.local`, the private and IPv4 link-local
//!   ranges in [`permitted_ranges`] (the device's DHCP address is not
//!   known in advance and the pin must survive address changes), and a
//!   permitted e-mail subtree `invalid` that no real mailbox matches.
//!   rcgen cannot encode URI constraints and rustls rejects any directory
//!   name constraint, so those forms rely on the EKU instead.
//!
//! The remaining exposure is that a stolen CA key can certify other hosts'
//! private addresses for PCs that installed the CA (docs/AUTH.md).
//!
//! Files in `dir`:
//! - `ca-key.pem` (0600) and `ca.pem`: the CA, created once. It is replaced
//!   only when nothing pins it yet (no `endpoint.json`) or when the host
//!   name changed; any other inconsistency is an error, never a silent new
//!   pin;
//! - `server-key.pem` (0640) and `server.pem`: the TLS server certificate,
//!   reissued when the host name or addresses change, when it no longer
//!   verifies against `ca.pem`, or within 30 days of expiry (397-day
//!   validity, accepted by Apple platforms);
//! - `endpoint.json`: `{host, port, ips, urls, ca_sha256, not_after_unix}`,
//!   read by the provisioning service and the control plane;
//! - `addresses.json`: when each address was last seen ([`stable_addresses`]).
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType, ExtendedKeyUsagePurpose, GeneralSubtree,
    IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints, SanType, SerialNumber,
};
use rustls::client::danger::ServerCertVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde_json::{json, Value};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;

pub const SERVER_VALIDITY: Duration = Duration::from_secs(397 * 86_400);
pub const RENEW_BEFORE: Duration = Duration::from_secs(30 * 86_400);
const CA_VALIDITY: Duration = Duration::from_secs(10 * 365 * 86_400);
/// An address that went away stays certified this long, so a Wi-Fi drop or
/// DHCP renewal does not reissue the certificate (and restart the control
/// plane) twice.
pub const ADDRESS_GRACE: Duration = Duration::from_secs(3600);
/// At most this many addresses are certified (current ones first).
pub const MAX_ADDRESSES: usize = 8;

/// Ranges the device CA may certify (besides its own name): RFC 1918,
/// IPv4 link-local (direct cable without DHCP) and IPv6 ULA. Loopback and
/// IPv6 link-local are not useful LAN addresses and are not certified.
pub fn permitted_ranges() -> Vec<(IpAddr, u8)> {
    vec![
        (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)), 8),
        (IpAddr::V4(Ipv4Addr::new(172, 16, 0, 0)), 12),
        (IpAddr::V4(Ipv4Addr::new(192, 168, 0, 0)), 16),
        (IpAddr::V4(Ipv4Addr::new(169, 254, 0, 0)), 16),
        (IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 0)), 8),
    ]
}

const LOOPBACK: (IpAddr, u8) = (IpAddr::V4(Ipv4Addr::new(127, 0, 0, 0)), 8);

/// How a CA is made. Only tests and CI use anything but the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Also permit and certify 127.0.0.0/8 (for TLS tests on one machine;
    /// never on a device). Applies to a CA when it is created.
    pub test_loopback: bool,
}

impl Options {
    fn ranges(&self) -> Vec<(IpAddr, u8)> {
        let mut r = permitted_ranges();
        if self.test_loopback {
            r.push(LOOPBACK);
        }
        r
    }
}

fn in_range(ip: &IpAddr, (net, prefix): &(IpAddr, u8)) -> bool {
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(n)) => u32::from(*a) >> (32 - *prefix as u32) == u32::from(*n) >> (32 - *prefix as u32),
        (IpAddr::V6(a), IpAddr::V6(n)) => u128::from(*a) >> (128 - *prefix as u32) == u128::from(*n) >> (128 - *prefix as u32),
        _ => false,
    }
}

/// Addresses the device may put in its certificate.
pub fn certifiable(ip: &IpAddr) -> bool {
    permitted_ranges().iter().any(|r| in_range(ip, r))
}

/// `host` is a single DNS label; the certified name is `<host>.local`.
pub fn valid_host(host: &str) -> bool {
    (1..=63).contains(&host.len())
        && host.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !host.starts_with('-')
        && !host.ends_with('-')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub ips: Vec<IpAddr>,
    pub ca_sha256: String,
    pub not_after_unix: u64,
}

impl Endpoint {
    pub fn fqdn(&self) -> String { format!("{}.local", self.host) }

    /// Preferred URL first (mDNS name), then address fallbacks.
    pub fn urls(&self) -> Vec<String> {
        let mut urls = vec![format!("https://{}:{}/", self.fqdn(), self.port)];
        for ip in &self.ips {
            match ip {
                IpAddr::V4(v4) => urls.push(format!("https://{v4}:{}/", self.port)),
                IpAddr::V6(v6) => urls.push(format!("https://[{v6}]:{}/", self.port)),
            }
        }
        urls
    }

    pub fn to_json(&self) -> Value {
        json!({
            "host": self.fqdn(), "port": self.port,
            "ips": self.ips.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
            "urls": self.urls(), "ca_sha256": self.ca_sha256, "not_after_unix": self.not_after_unix,
        })
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        let fqdn = v.get("host")?.as_str()?;
        let host = fqdn.strip_suffix(".local")?.to_owned();
        let port = u16::try_from(v.get("port")?.as_u64()?).ok()?;
        let ips = v.get("ips")?.as_array()?.iter().map(|i| i.as_str()?.parse().ok()).collect::<Option<Vec<IpAddr>>>()?;
        let ca_sha256 = v.get("ca_sha256")?.as_str()?.to_owned();
        let not_after_unix = v.get("not_after_unix")?.as_u64()?;
        if !valid_host(&host) || ca_sha256.len() != 64 || !ca_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(Self { host, port, ips, ca_sha256, not_after_unix })
    }

    /// Reads `endpoint.json`; anything missing or malformed is `None`
    /// (callers fail closed).
    pub fn load(dir: &Path) -> Option<Self> {
        let raw = std::fs::read(dir.join("endpoint.json")).ok()?;
        Self::from_json(&serde_json::from_slice(&raw).ok()?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ensure {
    Unchanged,
    /// A server certificate was (re)issued; the TLS server must reload.
    Issued,
}

fn dt(t: SystemTime) -> Result<OffsetDateTime, String> {
    let secs = t.duration_since(UNIX_EPOCH).map_err(|_| "clock before 1970".to_string())?.as_secs();
    OffsetDateTime::from_unix_timestamp(secs as i64).map_err(|e| e.to_string())
}

fn ca_params(host: &str, o: &Options) -> Result<CertificateParams, String> {
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, format!("Stephania device CA {host}"));
    p.distinguished_name = dn;
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let mut permitted = vec![GeneralSubtree::DnsName(format!("{host}.local"))];
    permitted.extend(o.ranges().into_iter().map(|(ip, prefix)| GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(ip, prefix))));
    // A permitted e-mail subtree that matches no real mailbox: with it, any
    // rfc822Name in a certificate below this CA is outside the constraints.
    permitted.push(GeneralSubtree::Rfc822Name("invalid".into()));
    p.name_constraints = Some(NameConstraints { permitted_subtrees: permitted, excluded_subtrees: vec![] });
    Ok(p)
}

fn random_serial() -> Result<SerialNumber, String> {
    let mut b = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).map_err(|_| "rng failure".to_string())?;
    b[0] &= 0x7f; // positive
    Ok(SerialNumber::from_slice(&b))
}

fn write_atomic(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(mode);
    }
    let mut f = opts.open(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.write_all(data).and_then(|_| f.sync_all()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn sha256_hex(data: &[u8]) -> String {
    crate::provisioning::hex(ring::digest::digest(&ring::digest::SHA256, data).as_ref())
}

fn pem_der(pem_text: &str) -> Option<Vec<u8>> {
    let body: String = pem_text.lines().filter(|l| !l.starts_with("-----")).collect();
    base64_decode(&body)
}

/// The PEM blocks of a certificate chain file, in order.
fn pem_blocks(text: &str) -> Vec<String> {
    const END: &str = "-----END CERTIFICATE-----";
    text.split(END).filter_map(|b| b.find("-----BEGIN CERTIFICATE-----").map(|i| format!("{}{END}\n", &b[i..]))).collect()
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let trimmed = bytes.iter().rposition(|&b| b != b'=').map(|i| &bytes[..=i]).unwrap_or(&[]);
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    for chunk in trimmed.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() { acc |= val(c)? << (18 - 6 * i); }
        let n = chunk.len();
        if n < 2 { return None; }
        out.push((acc >> 16) as u8);
        if n > 2 { out.push((acc >> 8) as u8); }
        if n > 3 { out.push(acc as u8); }
    }
    Some(out)
}

/// `(subject, subjectPublicKeyInfo)` of a CA certificate, as a TLS client
/// sees its trust anchor.
fn anchor_of(der: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut store = rustls::RootCertStore::empty();
    store.add(CertificateDer::from(der.to_vec())).map_err(|e| format!("not a usable CA certificate: {e}"))?;
    let a = &store.roots[0];
    Ok((a.subject.as_ref().to_vec(), a.subject_public_key_info.as_ref().to_vec()))
}

/// Verifies `chain` (leaf first) with the CA as the only anchor for every
/// name, the way the phone and stephctl do.
fn verify_chain(ca_der: &[u8], chain: &[CertificateDer<'static>], names: &[ServerName<'static>], now: SystemTime) -> Result<(), String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(ca_der.to_vec())).map_err(|e| e.to_string())?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let v = rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider).build().map_err(|e| e.to_string())?;
    let at = UnixTime::since_unix_epoch(now.duration_since(UNIX_EPOCH).map_err(|_| "clock before 1970")?);
    let (leaf, rest) = chain.split_first().ok_or("empty chain")?;
    for name in names {
        v.verify_server_cert(leaf, rest, name, &[], at).map_err(|e| format!("{name:?}: {e}"))?;
    }
    Ok(())
}

fn server_names(host: &str, ips: &[IpAddr]) -> Result<Vec<ServerName<'static>>, String> {
    let mut names = vec![ServerName::try_from(format!("{host}.local")).map_err(|e| e.to_string())?];
    names.extend(ips.iter().map(|ip| ServerName::IpAddress((*ip).into())));
    Ok(names)
}

fn read_opt(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn create_ca(dir: &Path, host: &str, o: &Options, now: SystemTime) -> Result<(KeyPair, String), String> {
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let mut p = ca_params(host, o)?;
    p.not_before = dt(now - Duration::from_secs(3600))?;
    p.not_after = dt(now + CA_VALIDITY)?;
    p.serial_number = Some(random_serial()?);
    let cert = p.self_signed(&key).map_err(|e| e.to_string())?;
    write_atomic(&dir.join("ca-key.pem"), key.serialize_pem().as_bytes(), 0o600)?;
    write_atomic(&dir.join("ca.pem"), cert.pem().as_bytes(), 0o644)?;
    eprintln!("stephania-pki: created a new device CA for {host}.local (SHA-256 {})", sha256_hex(cert.der()));
    Ok((key, cert.pem()))
}

enum Existing {
    Ok(Box<KeyPair>, String),
    /// A consistent CA, made for another host name.
    OtherHost,
    Broken(String),
}

fn existing_ca(dir: &Path, host: &str, o: &Options) -> Result<Existing, String> {
    let (key_text, cert_text) = (read_opt(&dir.join("ca-key.pem"))?, read_opt(&dir.join("ca.pem"))?);
    let key_text = match (key_text, &cert_text) {
        (None, None) => return Ok(Existing::Broken("there is no device CA".into())),
        (None, Some(_)) => return Ok(Existing::Broken("ca-key.pem is missing but ca.pem exists".into())),
        (Some(k), _) => k,
    };
    let Ok(key) = KeyPair::from_pem(&key_text) else { return Ok(Existing::Broken("ca-key.pem is not a readable key".into())) };
    // What a CA for this host made from this key looks like to a client.
    let probe = ca_params(host, o)?.self_signed(&key).map_err(|e| e.to_string())?;
    let (want_subject, want_spki) = anchor_of(probe.der())?;
    let cert_pem = match cert_text {
        Some(c) => c,
        None => {
            // server.pem ends with the CA; restoring it keeps the pin.
            let chain = read_opt(&dir.join("server.pem"))?.unwrap_or_default();
            let Some(last) = pem_blocks(&chain).pop() else { return Ok(Existing::Broken("ca.pem is missing".into())) };
            let matches = pem_der(&last).and_then(|d| anchor_of(&d).ok()).is_some_and(|(_, spki)| spki == want_spki);
            if !matches { return Ok(Existing::Broken("ca.pem is missing".into())); }
            write_atomic(&dir.join("ca.pem"), last.as_bytes(), 0o644)?;
            eprintln!("stephania-pki: ca.pem was missing; restored it from server.pem");
            last
        }
    };
    let Some(Ok((subject, spki))) = pem_der(&cert_pem).map(|d| anchor_of(&d)) else {
        return Ok(Existing::Broken("ca.pem is not a readable certificate".into()));
    };
    if spki != want_spki {
        return Ok(Existing::Broken("ca-key.pem does not match ca.pem".into()));
    }
    if subject != want_subject {
        return Ok(Existing::OtherHost);
    }
    Ok(Existing::Ok(Box::new(key), cert_pem))
}

/// The CA key and certificate, created when nothing can pin a CA yet.
/// A CA that clients may have pinned (an `endpoint.json` exists) is never
/// replaced because a file is missing or damaged: that is an error the
/// owner has to resolve. A host name change replaces it, loudly, because
/// its name constraint cannot cover the new name.
fn load_or_create_ca(dir: &Path, host: &str, o: &Options, now: SystemTime) -> Result<(KeyPair, String), String> {
    let endpoint_path = dir.join("endpoint.json");
    match existing_ca(dir, host, o)? {
        Existing::Ok(key, pem) => Ok((*key, pem)),
        Existing::OtherHost => {
            eprintln!("stephania-pki: the device CA was made for another host name; replacing it for {host}.local. \
                Phones and PCs that trusted the old CA must be set up again.");
            // Unpublish first, so an interruption cannot leave a pinned,
            // half-replaced CA behind.
            match std::fs::remove_file(&endpoint_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", endpoint_path.display())),
            }
            create_ca(dir, host, o, now)
        }
        Existing::Broken(why) if !endpoint_path.exists() => {
            if why != "there is no device CA" {
                eprintln!("stephania-pki: {why}; no client can have pinned this CA yet, creating a new one");
            }
            create_ca(dir, host, o, now)
        }
        Existing::Broken(why) => Err(format!(
            "{why}. Enrolled phones and PCs pin the device CA, so it is not replaced automatically: restore the file, \
             or remove {} to start over (every phone and PC must then be set up again)",
            dir.display()
        )),
    }
}

/// Whether `server.pem` is the CA-issued chain for these names right now.
fn chain_valid(dir: &Path, ca_der: &[u8], names: &[ServerName<'static>], now: SystemTime) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("server.pem")) else { return false };
    let Some(chain) = pem_blocks(&text).iter().map(|b| pem_der(b).map(CertificateDer::from)).collect::<Option<Vec<_>>>() else { return false };
    chain.len() == 2 && chain[1].as_ref() == ca_der && verify_chain(ca_der, &chain, names, now).is_ok()
}

/// Creates the CA if needed and (re)issues the server certificate when the
/// name, addresses or remaining validity require it. Idempotent.
pub fn ensure(dir: &Path, host: &str, ips: &[IpAddr], port: u16, now: SystemTime) -> Result<(Ensure, Endpoint), String> {
    ensure_with(dir, host, ips, port, now, &Options::default())
}

pub fn ensure_with(dir: &Path, host: &str, ips: &[IpAddr], port: u16, now: SystemTime, o: &Options) -> Result<(Ensure, Endpoint), String> {
    if !valid_host(host) {
        return Err(format!("invalid host label {host:?}"));
    }
    let ranges = o.ranges();
    let mut ips: Vec<IpAddr> = ips.iter().copied().filter(|ip| ranges.iter().any(|r| in_range(ip, r))).collect();
    ips.sort();
    ips.dedup();
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let (ca_key, ca_pem) = load_or_create_ca(dir, host, o, now)?;
    let ca_der = pem_der(&ca_pem).ok_or("ca.pem is not PEM")?;
    let ca_sha256 = sha256_hex(&ca_der);
    let names = server_names(host, &ips)?;

    let current = Endpoint::load(dir);
    if let Some(pinned) = current.as_ref().map(|e| &e.ca_sha256).filter(|p| **p != ca_sha256) {
        eprintln!("stephania-pki: the device CA changed (SHA-256 {pinned} -> {ca_sha256}); enrolled clients must be set up again");
    }
    let now_unix = now.duration_since(UNIX_EPOCH).map_err(|_| "clock before 1970")?.as_secs();
    let fresh = current.as_ref().is_some_and(|e| {
        e.host == host && e.port == port && e.ips == ips && e.ca_sha256 == ca_sha256
            && e.not_after_unix > now_unix + RENEW_BEFORE.as_secs()
    }) && dir.join("server-key.pem").is_file() && chain_valid(dir, &ca_der, &names, now);
    if fresh {
        return Ok((Ensure::Unchanged, current.expect("checked")));
    }

    let issuer = Issuer::new(ca_params(host, o)?, ca_key);
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let fqdn = format!("{host}.local");
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, fqdn.clone());
    p.distinguished_name = dn;
    p.subject_alt_names = std::iter::once(SanType::DnsName(fqdn.try_into().map_err(|e| format!("{e:?}"))?))
        .chain(ips.iter().map(|ip| SanType::IpAddress(*ip)))
        .collect();
    p.is_ca = IsCa::ExplicitNoCa;
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    p.use_authority_key_identifier_extension = true;
    p.not_before = dt(now - Duration::from_secs(3600))?;
    let not_after = now + SERVER_VALIDITY;
    p.not_after = dt(not_after)?;
    p.serial_number = Some(random_serial()?);
    let cert = p.signed_by(&key, &issuer).map_err(|e| e.to_string())?;
    // Never publish a chain clients would reject (e.g. an expired CA).
    let chain = [CertificateDer::from(cert.der().to_vec()), CertificateDer::from(ca_der.clone())];
    verify_chain(&ca_der, &chain, &names, now).map_err(|e| format!("the new certificate does not verify against ca.pem: {e}"))?;
    write_atomic(&dir.join("server-key.pem"), key.serialize_pem().as_bytes(), 0o640)?;
    // Chain: leaf then CA, so clients that only pin the CA can build it.
    write_atomic(&dir.join("server.pem"), format!("{}{}", cert.pem(), ca_pem).as_bytes(), 0o644)?;
    let endpoint = Endpoint {
        host: host.to_owned(), port, ips, ca_sha256,
        not_after_unix: not_after.duration_since(UNIX_EPOCH).map_err(|_| "clock")?.as_secs(),
    };
    write_atomic(&dir.join("endpoint.json"), endpoint.to_json().to_string().as_bytes(), 0o644)?;
    Ok((Ensure::Issued, endpoint))
}

/// The addresses to certify: those up now, plus those seen within
/// [`ADDRESS_GRACE`] (at most [`MAX_ADDRESSES`], most recently seen first),
/// so a short outage does not reissue the certificate twice. New addresses
/// are certified immediately. The last-seen times are kept in
/// `addresses.json`; if it cannot be written only `current` is used.
pub fn stable_addresses(dir: &Path, current: &[IpAddr], now: SystemTime, o: &Options) -> Vec<IpAddr> {
    let ranges = o.ranges();
    let now_unix = now.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = dir.join("addresses.json");
    let mut seen: Vec<(IpAddr, u64)> = std::fs::read(&path).ok()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .and_then(|v| v.get("seen").and_then(Value::as_object).cloned())
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_u64()?))).collect())
        .unwrap_or_default();
    let mut current: Vec<IpAddr> = current.iter().copied().filter(|ip| ranges.iter().any(|r| in_range(ip, r))).collect();
    current.sort();
    current.dedup();
    // A clock that went backwards counts as "just seen".
    seen.retain(|(ip, t)| !current.contains(ip) && ranges.iter().any(|r| in_range(ip, r)) && now_unix.saturating_sub(*t) <= ADDRESS_GRACE.as_secs());
    seen.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    seen.truncate(MAX_ADDRESSES.saturating_sub(current.len()));
    let mut out: Vec<(IpAddr, u64)> = current.iter().map(|ip| (*ip, now_unix)).collect();
    out.extend(seen);
    out.truncate(MAX_ADDRESSES);
    let state = json!({"seen": out.iter().map(|(ip, t)| (ip.to_string(), json!(t))).collect::<serde_json::Map<_, _>>()});
    let written = std::fs::create_dir_all(dir).map_err(|e| e.to_string())
        .and_then(|_| write_atomic(&path, state.to_string().as_bytes(), 0o644));
    if let Err(e) = written {
        eprintln!("stephania-pki: {e}; certifying only the current addresses");
        current.truncate(MAX_ADDRESSES);
        return current;
    }
    let mut ips: Vec<IpAddr> = out.into_iter().map(|(ip, _)| ip).collect();
    ips.sort();
    ips
}

/// Non-loopback addresses of up interfaces (Linux/Unix `getifaddrs`).
#[cfg(unix)]
pub fn interface_addresses() -> Vec<IpAddr> {
    let mut out = Vec::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `head` with a list we free below; every
    // pointer is checked before use and sockaddr casts follow sa_family.
    unsafe {
        if libc::getifaddrs(&mut head) != 0 {
            return out;
        }
        let mut cur = head;
        while !cur.is_null() {
            let ifa = &*cur;
            let up = ifa.ifa_flags & libc::IFF_UP as u32 != 0 && ifa.ifa_flags & libc::IFF_LOOPBACK as u32 == 0;
            if up && !ifa.ifa_addr.is_null() {
                match (*ifa.ifa_addr).sa_family as i32 {
                    libc::AF_INET => {
                        let sa = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                        out.push(IpAddr::V4(Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr))));
                    }
                    libc::AF_INET6 => {
                        let sa = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                        out.push(IpAddr::V6(Ipv6Addr::from(sa.sin6_addr.s6_addr)));
                    }
                    _ => {}
                }
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(head);
    }
    out
}

pub fn default_dir() -> PathBuf {
    std::env::var_os("STEPHANIA_PKI_DIR").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| "/data/stephania/pki".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("stephania-pki-{}-{:x}", std::process::id(), rand_u64()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn rand_u64() -> u64 {
        let mut b = [0u8; 8];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).unwrap();
        u64::from_le_bytes(b)
    }

    fn chain(dir: &Path) -> Vec<CertificateDer<'static>> {
        let text = std::fs::read_to_string(dir.join("server.pem")).unwrap();
        text.split("-----END CERTIFICATE-----").filter(|b| b.contains("BEGIN")).map(|b| CertificateDer::from(pem_der(b).unwrap())).collect()
    }

    fn verify(dir: &Path, name: &str, chain: &[CertificateDer<'static>], now: SystemTime) -> Result<(), rustls::Error> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(pem_der(&std::fs::read_to_string(dir.join("ca.pem")).unwrap()).unwrap())).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let v = rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider).build().unwrap();
        let at = UnixTime::since_unix_epoch(now.duration_since(UNIX_EPOCH).unwrap());
        v.verify_server_cert(&chain[0], &chain[1..], &ServerName::try_from(name.to_owned()).unwrap(), &[], at).map(|_| ())
    }

    #[test]
    fn issues_a_chain_rustls_accepts_for_name_and_addresses() {
        let dir = tmp();
        let now = SystemTime::now();
        let ip: IpAddr = "192.168.1.20".parse().unwrap();
        let public: IpAddr = "8.8.8.8".parse().unwrap();
        let (r, e) = ensure(&dir, "stephania-ab12", &[ip, public], 9443, now).unwrap();
        assert_eq!(r, Ensure::Issued);
        assert_eq!(e.ips, vec![ip], "public addresses are never certified");
        assert_eq!(e.urls(), vec!["https://stephania-ab12.local:9443/", "https://192.168.1.20:9443/"]);
        let c = chain(&dir);
        assert_eq!(c.len(), 2);
        verify(&dir, "stephania-ab12.local", &c, now).unwrap();
        verify(&dir, "192.168.1.20", &c, now).unwrap();
        assert!(verify(&dir, "other.local", &c, now).is_err());
        assert!(verify(&dir, "stephania-ab12.local", &c, now + SERVER_VALIDITY + Duration::from_secs(86_400)).is_err());
        assert_eq!(Endpoint::load(&dir), Some(e.clone()));
        assert_eq!(e.ca_sha256, sha256_hex(&c[1]));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("ca-key.pem")).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(dir.join("server-key.pem")).unwrap().permissions().mode() & 0o777, 0o640);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn idempotent_reissues_on_change_and_keeps_the_ca() {
        let dir = tmp();
        let now = SystemTime::now();
        let a: IpAddr = "10.0.0.5".parse().unwrap();
        let (_, first) = ensure(&dir, "stephania", &[a], 9443, now).unwrap();
        let leaf = std::fs::read(dir.join("server.pem")).unwrap();
        assert_eq!(ensure(&dir, "stephania", &[a], 9443, now).unwrap().0, Ensure::Unchanged);
        assert_eq!(std::fs::read(dir.join("server.pem")).unwrap(), leaf);
        let b: IpAddr = "10.0.0.6".parse().unwrap();
        let (r, second) = ensure(&dir, "stephania", &[b], 9443, now).unwrap();
        assert_eq!(r, Ensure::Issued);
        assert_eq!(second.ca_sha256, first.ca_sha256, "a new address must not change the pinned CA");
        verify(&dir, "10.0.0.6", &chain(&dir), now).unwrap();
        // Renewal window.
        let later = now + SERVER_VALIDITY - RENEW_BEFORE + Duration::from_secs(60);
        assert_eq!(ensure(&dir, "stephania", &[b], 9443, later).unwrap().0, Ensure::Issued);
        verify(&dir, "stephania.local", &chain(&dir), later).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn name_constraints_stop_the_ca_certifying_other_sites() {
        let dir = tmp();
        let now = SystemTime::now();
        ensure(&dir, "stephania", &[], 9443, now).unwrap();
        // Forge a leaf with the real CA key for a foreign name.
        let ca_key = KeyPair::from_pem(&std::fs::read_to_string(dir.join("ca-key.pem")).unwrap()).unwrap();
        let issuer = Issuer::new(ca_params("stephania", &Options::default()).unwrap(), ca_key);
        for name in ["bank.example", "8.8.8.8"] {
            let key = KeyPair::generate().unwrap();
            let mut p = CertificateParams::new(vec![name.to_owned()]).unwrap();
            p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
            p.use_authority_key_identifier_extension = true;
            let leaf = CertificateDer::from(p.signed_by(&key, &issuer).unwrap().der().to_vec());
            let ca = chain(&dir).pop().unwrap();
            let err = verify(&dir, name, &[leaf, ca], now).unwrap_err();
            assert!(format!("{err:?}").contains("NameConstraint"), "{name}: expected a name-constraint violation, got {err:?}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_bad_hosts_and_malformed_endpoint_files() {
        assert!(!valid_host("Stephania"));
        assert!(!valid_host("a.b"));
        assert!(!valid_host("-x"));
        assert!(valid_host("stephania-8f21"));
        assert!(ensure(&tmp(), "a.b", &[], 1, SystemTime::now()).is_err());
        assert_eq!(Endpoint::from_json(&json!({"host":"x.local","port":1,"ips":[],"ca_sha256":"zz","not_after_unix":1})), None);
        assert!(certifiable(&"fd12::1".parse().unwrap()));
        assert!(!certifiable(&"2001:db8::1".parse().unwrap()));
        assert!(!certifiable(&"172.32.0.1".parse().unwrap()));
        assert!(!certifiable(&"127.0.0.1".parse().unwrap()), "loopback is never a LAN address");
        assert!(!certifiable(&"fe80::1".parse().unwrap()), "IPv6 link-local needs a zone; not certified");
    }

    fn forge(dir: &Path, host: &str, names: Vec<String>, eku: ExtendedKeyUsagePurpose) -> String {
        let ca_key = KeyPair::from_pem(&std::fs::read_to_string(dir.join("ca-key.pem")).unwrap()).unwrap();
        let issuer = Issuer::new(ca_params(host, &Options::default()).unwrap(), ca_key);
        let mut p = CertificateParams::new(names).unwrap();
        p.extended_key_usages = vec![eku];
        p.use_authority_key_identifier_extension = true;
        p.signed_by(&KeyPair::generate().unwrap(), &issuer).unwrap().pem()
    }

    fn openssl(args: &[&str]) -> Option<(bool, String)> {
        let out = std::process::Command::new("openssl").args(args).output().ok()?;
        Some((out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))))
    }

    #[test]
    fn ca_is_limited_to_tls_servers_with_openssl() {
        let dir = tmp();
        ensure(&dir, "stephania", &["192.168.1.20".parse().unwrap()], 9443, SystemTime::now()).unwrap();
        let ca = dir.join("ca.pem");
        let ca = ca.to_str().unwrap();
        let Some((_, text)) = openssl(&["x509", "-in", ca, "-noout", "-text"]) else {
            eprintln!("openssl not installed; skipping");
            return;
        };
        assert!(text.contains("CA:TRUE, pathlen:0"), "{text}");
        assert!(text.contains("Certificate Sign, CRL Sign"), "{text}");
        assert!(text.contains("X509v3 Extended Key Usage") && text.contains("TLS Web Server Authentication"), "{text}");
        assert!(text.contains("DNS:stephania.local") && text.contains("email:invalid"), "{text}");
        assert!(!text.contains("127.0.0.0") && !text.contains("FE80"), "{text}");
        let leaf = dir.join("leaf.pem");
        let leaf_s = leaf.to_str().unwrap();
        let server = std::fs::read_to_string(dir.join("server.pem")).unwrap();
        std::fs::write(&leaf, &pem_blocks(&server)[0]).unwrap();
        let ok = |extra: &[&str]| openssl(&[&["verify", "-CAfile", ca][..], extra, &[leaf_s]].concat()).unwrap();
        assert!(ok(&["-purpose", "sslserver", "-verify_hostname", "stephania.local"]).0);
        assert!(ok(&["-purpose", "sslserver", "-verify_ip", "192.168.1.20"]).0);
        // OpenSSL has no code-signing purpose; Windows and macOS refuse it
        // because the CA's EKU lists server authentication only.
        assert!(!text.contains("Code Signing") && !text.contains("Client Authentication"), "{text}");
        // Forged with the CA key: S/MIME, client auth, loopback.
        for (names, eku, purpose) in [
            (vec!["stephania.local".to_owned()], ExtendedKeyUsagePurpose::EmailProtection, "smimesign"),
            (vec!["stephania.local".to_owned()], ExtendedKeyUsagePurpose::ClientAuth, "sslclient"),
            (vec!["127.0.0.1".to_owned()], ExtendedKeyUsagePurpose::ServerAuth, "sslserver"),
        ] {
            std::fs::write(&leaf, forge(&dir, "stephania", names.clone(), eku.clone())).unwrap();
            let (pass, out) = ok(&["-purpose", purpose, "-x509_strict"]);
            assert!(!pass, "{names:?} {eku:?} must not verify for {purpose}: {out}");
        }
        // An e-mail address in a server certificate is outside the constraints.
        let ca_key = KeyPair::from_pem(&std::fs::read_to_string(dir.join("ca-key.pem")).unwrap()).unwrap();
        let issuer = Issuer::new(ca_params("stephania", &Options::default()).unwrap(), ca_key);
        let mut p = CertificateParams::new(vec!["stephania.local".to_owned()]).unwrap();
        p.subject_alt_names.push(SanType::Rfc822Name("ceo@bank.example".try_into().unwrap()));
        p.use_authority_key_identifier_extension = true;
        std::fs::write(&leaf, p.signed_by(&KeyPair::generate().unwrap(), &issuer).unwrap().pem()).unwrap();
        let (pass, out) = ok(&[]);
        assert!(!pass && out.contains("subtree violation"), "{out}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn host_name_change_reissues_a_consistent_chain() {
        let dir = tmp();
        let now = SystemTime::now();
        let ip: IpAddr = "10.0.0.5".parse().unwrap();
        let (_, old) = ensure(&dir, "stephania", &[ip], 9443, now).unwrap();
        let (r, new) = ensure(&dir, "stephania-ab12", &[ip], 9443, now).unwrap();
        assert_eq!(r, Ensure::Issued);
        assert_ne!(new.ca_sha256, old.ca_sha256, "a CA constrained to the old name cannot certify the new one");
        let c = chain(&dir);
        assert_eq!(new.ca_sha256, sha256_hex(&c[1]));
        verify(&dir, "stephania-ab12.local", &c, now).unwrap();
        verify(&dir, "10.0.0.5", &c, now).unwrap();
        assert_eq!(ensure(&dir, "stephania-ab12", &[ip], 9443, now).unwrap().0, Ensure::Unchanged);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_leaf_that_no_longer_verifies_is_reissued() {
        let dir = tmp();
        let now = SystemTime::now();
        ensure(&dir, "stephania", &[], 9443, now).unwrap();
        // Replace server.pem with a chain from an unrelated CA.
        let other = tmp();
        ensure(&other, "stephania", &[], 9443, now).unwrap();
        std::fs::copy(other.join("server.pem"), dir.join("server.pem")).unwrap();
        assert_eq!(ensure(&dir, "stephania", &[], 9443, now).unwrap().0, Ensure::Issued);
        verify(&dir, "stephania.local", &chain(&dir), now).unwrap();
        for d in [dir, other] { std::fs::remove_dir_all(d).unwrap(); }
    }

    #[test]
    fn partial_ca_state_never_silently_rotates_the_pin() {
        let dir = tmp();
        let now = SystemTime::now();
        let (_, e) = ensure(&dir, "stephania", &[], 9443, now).unwrap();
        let key = std::fs::read(dir.join("ca-key.pem")).unwrap();
        // ca.pem lost: restored from the chain in server.pem, same pin.
        std::fs::remove_file(dir.join("ca.pem")).unwrap();
        let (r, again) = ensure(&dir, "stephania", &[], 9443, now).unwrap();
        assert_eq!((r, &again.ca_sha256), (Ensure::Unchanged, &e.ca_sha256));
        // ca-key.pem lost, unreadable or not matching: an error, files untouched.
        let ca = std::fs::read(dir.join("ca.pem")).unwrap();
        let other = KeyPair::generate().unwrap().serialize_pem();
        for broken in [None, Some(b"garbage".to_vec()), Some(other.into_bytes())] {
            match &broken {
                None => std::fs::remove_file(dir.join("ca-key.pem")).unwrap(),
                Some(b) => std::fs::write(dir.join("ca-key.pem"), b).unwrap(),
            }
            let err = ensure(&dir, "stephania", &[], 9443, now).unwrap_err();
            assert!(err.contains("not replaced automatically"), "{err}");
            assert_eq!(std::fs::read(dir.join("ca.pem")).unwrap(), ca);
            assert_eq!(Endpoint::load(&dir).unwrap().ca_sha256, e.ca_sha256);
        }
        // Both files lost while clients pin the CA: also an error.
        std::fs::remove_file(dir.join("ca-key.pem")).unwrap();
        std::fs::remove_file(dir.join("ca.pem")).unwrap();
        assert!(ensure(&dir, "stephania", &[], 9443, now).is_err());
        // Restoring the key recovers deterministically (pin kept).
        std::fs::write(dir.join("ca-key.pem"), &key).unwrap();
        assert_eq!(ensure(&dir, "stephania", &[], 9443, now).unwrap().1.ca_sha256, e.ca_sha256);
        // Nothing published yet (interrupted first run): a new CA is fine.
        std::fs::remove_file(dir.join("endpoint.json")).unwrap();
        std::fs::remove_file(dir.join("ca.pem")).unwrap();
        std::fs::remove_file(dir.join("server.pem")).unwrap();
        let (r, fresh) = ensure(&dir, "stephania", &[], 9443, now).unwrap();
        assert_eq!(r, Ensure::Issued);
        assert_ne!(fresh.ca_sha256, e.ca_sha256);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loopback_only_for_test_cas() {
        let dir = tmp();
        let now = SystemTime::now();
        let lo: IpAddr = "127.0.0.1".parse().unwrap();
        let o = Options { test_loopback: true };
        let (_, e) = ensure_with(&dir, "stephania-ci", &[lo], 9443, now, &o).unwrap();
        assert_eq!(e.ips, vec![lo]);
        verify(&dir, "127.0.0.1", &chain(&dir), now).unwrap();
        let plain = tmp();
        assert_eq!(ensure(&plain, "stephania", &[lo], 9443, now).unwrap().1.ips, Vec::<IpAddr>::new());
        for d in [dir, plain] { std::fs::remove_dir_all(d).unwrap(); }
    }

    #[test]
    fn address_flaps_do_not_reissue() {
        let dir = tmp();
        let o = Options::default();
        let t0 = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let wifi: IpAddr = "10.0.0.5".parse().unwrap();
        let eth: IpAddr = "192.168.1.20".parse().unwrap();
        assert_eq!(stable_addresses(&dir, &[wifi, eth, "8.8.8.8".parse().unwrap()], t0, &o), vec![wifi, eth]);
        // Wi-Fi drops for a few minutes: still certified.
        let t1 = t0 + Duration::from_secs(300);
        assert_eq!(stable_addresses(&dir, &[eth], t1, &o), vec![wifi, eth]);
        assert_eq!(stable_addresses(&dir, &[eth, wifi], t1 + Duration::from_secs(300), &o), vec![wifi, eth]);
        // Gone for longer than the grace period: dropped.
        let t2 = t1 + Duration::from_secs(300) + ADDRESS_GRACE + Duration::from_secs(1);
        assert_eq!(stable_addresses(&dir, &[eth], t2, &o), vec![eth]);
        // Bounded, current addresses always kept.
        let many: Vec<IpAddr> = (1..=20).map(|i| IpAddr::V4(Ipv4Addr::new(10, 1, 0, i))).collect();
        for (i, ip) in many.iter().enumerate() {
            stable_addresses(&dir, &[*ip], t2 + Duration::from_secs(i as u64), &o);
        }
        let last = stable_addresses(&dir, &[eth], t2 + Duration::from_secs(30), &o);
        assert_eq!(last.len(), MAX_ADDRESSES);
        assert!(last.contains(&eth) && last.contains(&many[19]) && !last.contains(&many[0]));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
