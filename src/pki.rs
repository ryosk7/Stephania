//! Device PKI for trusted LAN TLS (docs/MASTER-ARCHITECTURE.md §26).
//!
//! Each device creates its own CA on first use. The CA is name-constrained
//! to the device's own `<host>.local` name and private/link-local address
//! ranges, so even a CA a user chose to trust can never certify any other
//! site. The phone learns the CA's SHA-256 over authenticated BLE
//! (`management.endpoint`) and pins it; no public CA is involved and no
//! browser warning is clicked through.
//!
//! Files in `dir`:
//! - `ca-key.pem` (0600) and `ca.pem`: the CA, created once;
//! - `server-key.pem` (0640) and `server.pem`: the TLS server certificate,
//!   reissued when the host name or addresses change, or within 30 days of
//!   expiry (397-day validity, accepted by Apple platforms);
//! - `endpoint.json`: `{host, port, ips, urls, ca_sha256, not_after_unix}`,
//!   read by the provisioning service and the control plane.
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType, ExtendedKeyUsagePurpose, GeneralSubtree,
    IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints, SanType, SerialNumber,
};
use serde_json::{json, Value};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;

pub const SERVER_VALIDITY: Duration = Duration::from_secs(397 * 86_400);
pub const RENEW_BEFORE: Duration = Duration::from_secs(30 * 86_400);
const CA_VALIDITY: Duration = Duration::from_secs(20 * 365 * 86_400);

/// Ranges the device CA may certify (besides its own name).
pub fn permitted_ranges() -> Vec<(IpAddr, u8)> {
    vec![
        (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)), 8),
        (IpAddr::V4(Ipv4Addr::new(172, 16, 0, 0)), 12),
        (IpAddr::V4(Ipv4Addr::new(192, 168, 0, 0)), 16),
        (IpAddr::V4(Ipv4Addr::new(169, 254, 0, 0)), 16),
        (IpAddr::V4(Ipv4Addr::new(127, 0, 0, 0)), 8),
        (IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 0)), 8),
        (IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0)), 10),
    ]
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

fn ca_params(host: &str) -> Result<CertificateParams, String> {
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, format!("Stephania device CA {host}"));
    p.distinguished_name = dn;
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let mut permitted = vec![GeneralSubtree::DnsName(format!("{host}.local"))];
    permitted.extend(permitted_ranges().into_iter().map(|(ip, prefix)| GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(ip, prefix))));
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

/// Creates the CA if needed and (re)issues the server certificate when the
/// name, addresses or remaining validity require it. Idempotent.
pub fn ensure(dir: &Path, host: &str, ips: &[IpAddr], port: u16, now: SystemTime) -> Result<(Ensure, Endpoint), String> {
    if !valid_host(host) {
        return Err(format!("invalid host label {host:?}"));
    }
    let mut ips: Vec<IpAddr> = ips.iter().copied().filter(certifiable).collect();
    ips.sort();
    ips.dedup();
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let ca_key_path = dir.join("ca-key.pem");
    let ca_path = dir.join("ca.pem");
    let params = ca_params(host)?;
    let (ca_key, ca_pem) = match (std::fs::read_to_string(&ca_key_path), std::fs::read_to_string(&ca_path)) {
        (Ok(k), Ok(c)) => (KeyPair::from_pem(&k).map_err(|e| format!("ca key: {e}"))?, c),
        _ => {
            let key = KeyPair::generate().map_err(|e| e.to_string())?;
            let mut p = params.clone();
            p.not_before = dt(now - Duration::from_secs(3600))?;
            p.not_after = dt(now + CA_VALIDITY)?;
            p.serial_number = Some(random_serial()?);
            let cert = p.self_signed(&key).map_err(|e| e.to_string())?;
            write_atomic(&ca_key_path, key.serialize_pem().as_bytes(), 0o600)?;
            write_atomic(&ca_path, cert.pem().as_bytes(), 0o644)?;
            (key, cert.pem())
        }
    };
    let ca_der = pem_der(&ca_pem).ok_or("ca.pem is not PEM")?;
    let ca_sha256 = sha256_hex(&ca_der);

    let current = Endpoint::load(dir);
    let now_unix = now.duration_since(UNIX_EPOCH).map_err(|_| "clock before 1970")?.as_secs();
    let fresh = current.as_ref().is_some_and(|e| {
        e.host == host && e.port == port && e.ips == ips && e.ca_sha256 == ca_sha256
            && e.not_after_unix > now_unix + RENEW_BEFORE.as_secs()
    }) && dir.join("server.pem").is_file() && dir.join("server-key.pem").is_file();
    if fresh {
        return Ok((Ensure::Unchanged, current.expect("checked")));
    }

    let issuer = Issuer::new(params, ca_key);
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
    use rustls::client::danger::ServerCertVerifier;
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use std::sync::Arc;

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
        let issuer = Issuer::new(ca_params("stephania").unwrap(), ca_key);
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
    }
}
