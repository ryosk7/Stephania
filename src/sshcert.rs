//! OpenSSH Ed25519 keys and certificates (PROTOCOL.certkeys,
//! PROTOCOL.key), for maintenance access (docs/MAINTENANCE.md).
//!
//! Only what the device needs: generate Ed25519 keys, write a private key
//! sshd can read (`openssh-key-v1`, unencrypted), parse an
//! `ssh-ed25519` public key line, and sign user and host certificates.
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};

pub const KEY_TYPE: &str = "ssh-ed25519";
pub const CERT_TYPE: &str = "ssh-ed25519-cert-v01@openssh.com";
pub const USER_CERT: u32 = 1;
pub const HOST_CERT: u32 = 2;

/// An Ed25519 key kept as its 32-byte seed.
#[derive(Clone)]
pub struct Key {
    seed: [u8; 32],
    public: [u8; 32],
}

impl Key {
    pub fn generate() -> Result<Self, String> {
        let mut seed = [0u8; 32];
        SystemRandom::new().fill(&mut seed).map_err(|_| "random source failed".to_string())?;
        Self::from_seed(seed)
    }

    pub fn from_seed(seed: [u8; 32]) -> Result<Self, String> {
        let pair = Ed25519KeyPair::from_seed_unchecked(&seed).map_err(|_| "invalid Ed25519 seed".to_string())?;
        let mut public = [0u8; 32];
        public.copy_from_slice(pair.public_key().as_ref());
        Ok(Self { seed, public })
    }

    pub fn seed(&self) -> &[u8; 32] { &self.seed }
    pub fn public(&self) -> &[u8; 32] { &self.public }

    /// The SSH wire encoding of the public key.
    pub fn public_blob(&self) -> Vec<u8> { public_blob(&self.public) }

    /// `ssh-ed25519 AAAA… comment`, as in authorized_keys or `TrustedUserCAKeys`.
    pub fn public_line(&self, comment: &str) -> String {
        let line = format!("{KEY_TYPE} {}", base64_encode(&self.public_blob()));
        if comment.is_empty() { line } else { format!("{line} {comment}") }
    }

    fn sign(&self, data: &[u8]) -> Vec<u8> {
        let pair = Ed25519KeyPair::from_seed_unchecked(&self.seed).expect("seed validated at construction");
        let mut out = Vec::new();
        put_string(&mut out, KEY_TYPE.as_bytes());
        put_string(&mut out, pair.sign(data).as_ref());
        out
    }

    /// Unencrypted `openssh-key-v1` private key (what `ssh-keygen` writes).
    pub fn private_pem(&self, comment: &str) -> Result<String, String> {
        let mut check = [0u8; 4];
        SystemRandom::new().fill(&mut check).map_err(|_| "random source failed".to_string())?;
        let mut private = Vec::new();
        private.extend_from_slice(&check);
        private.extend_from_slice(&check);
        put_string(&mut private, KEY_TYPE.as_bytes());
        put_string(&mut private, &self.public);
        let mut both = self.seed.to_vec();
        both.extend_from_slice(&self.public);
        put_string(&mut private, &both);
        put_string(&mut private, comment.as_bytes());
        let mut pad = 1u8;
        while private.len() % 8 != 0 {
            private.push(pad);
            pad += 1;
        }
        let mut body = b"openssh-key-v1\0".to_vec();
        put_string(&mut body, b"none");
        put_string(&mut body, b"none");
        put_string(&mut body, b"");
        body.extend_from_slice(&1u32.to_be_bytes());
        put_string(&mut body, &self.public_blob());
        put_string(&mut body, &private);
        let encoded = base64_encode(&body);
        let mut pem = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
        for chunk in encoded.as_bytes().chunks(70) {
            pem.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
            pem.push('\n');
        }
        pem.push_str("-----END OPENSSH PRIVATE KEY-----\n");
        Ok(pem)
    }
}

pub fn public_blob(public: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::new();
    put_string(&mut out, KEY_TYPE.as_bytes());
    put_string(&mut out, public);
    out
}

/// Parses an `ssh-ed25519 <base64> [comment]` line. Other key types are refused.
pub fn parse_public_line(line: &str) -> Result<[u8; 32], String> {
    let mut parts = line.split_ascii_whitespace();
    if parts.next() != Some(KEY_TYPE) { return Err("only ssh-ed25519 keys are accepted".into()); }
    let blob = parts.next().and_then(base64_decode).ok_or("malformed public key")?;
    let mut r = Reader(&blob);
    if r.string()? != KEY_TYPE.as_bytes() { return Err("key type mismatch".into()); }
    let key = r.string()?;
    if key.len() != 32 || !r.0.is_empty() { return Err("malformed public key".into()); }
    let mut out = [0u8; 32];
    out.copy_from_slice(key);
    Ok(out)
}

/// What goes into a certificate besides the key and the signer.
pub struct CertSpec<'a> {
    pub kind: u32,
    pub serial: u64,
    pub key_id: &'a str,
    pub principals: &'a [String],
    pub valid_after: u64,
    pub valid_before: u64,
    /// `(name, value)`; sorted by name when encoded.
    pub critical_options: &'a [(&'a str, String)],
    /// Extension names (empty data); sorted when encoded.
    pub extensions: &'a [&'a str],
}

/// Signs `public` with `ca` and returns the certificate line
/// (`ssh-ed25519-cert-v01@openssh.com AAAA… comment`).
pub fn sign_certificate(ca: &Key, public: &[u8; 32], spec: &CertSpec, comment: &str) -> Result<String, String> {
    let mut nonce = [0u8; 32];
    SystemRandom::new().fill(&mut nonce).map_err(|_| "random source failed".to_string())?;
    let mut body = Vec::new();
    put_string(&mut body, CERT_TYPE.as_bytes());
    put_string(&mut body, &nonce);
    put_string(&mut body, public);
    body.extend_from_slice(&spec.serial.to_be_bytes());
    body.extend_from_slice(&spec.kind.to_be_bytes());
    put_string(&mut body, spec.key_id.as_bytes());
    let mut principals = Vec::new();
    for p in spec.principals { put_string(&mut principals, p.as_bytes()); }
    put_string(&mut body, &principals);
    body.extend_from_slice(&spec.valid_after.to_be_bytes());
    body.extend_from_slice(&spec.valid_before.to_be_bytes());
    let mut options: Vec<_> = spec.critical_options.to_vec();
    options.sort_by(|a, b| a.0.cmp(b.0));
    let mut critical = Vec::new();
    for (name, value) in options {
        put_string(&mut critical, name.as_bytes());
        let mut data = Vec::new();
        put_string(&mut data, value.as_bytes());
        put_string(&mut critical, &data);
    }
    put_string(&mut body, &critical);
    let mut names: Vec<_> = spec.extensions.to_vec();
    names.sort_unstable();
    let mut extensions = Vec::new();
    for name in names {
        put_string(&mut extensions, name.as_bytes());
        put_string(&mut extensions, b"");
    }
    put_string(&mut body, &extensions);
    put_string(&mut body, b"");
    put_string(&mut body, &ca.public_blob());
    let signature = ca.sign(&body);
    put_string(&mut body, &signature);
    let line = format!("{CERT_TYPE} {}", base64_encode(&body));
    Ok(if comment.is_empty() { line } else { format!("{line} {comment}") })
}

/// Fields of a certificate, read back and with its signature checked
/// against the embedded signer. For tests and `steph whoami`.
#[derive(Debug, PartialEq, Eq)]
pub struct Certificate {
    pub public: [u8; 32],
    pub serial: u64,
    pub kind: u32,
    pub key_id: String,
    pub principals: Vec<String>,
    pub valid_after: u64,
    pub valid_before: u64,
    pub critical_options: Vec<(String, String)>,
    pub extensions: Vec<String>,
    pub signer: [u8; 32],
}

pub fn parse_certificate(line: &str) -> Result<Certificate, String> {
    let mut parts = line.split_ascii_whitespace();
    if parts.next() != Some(CERT_TYPE) { return Err("not an ssh-ed25519 certificate".into()); }
    let blob = parts.next().and_then(base64_decode).ok_or("malformed certificate")?;
    let mut r = Reader(&blob);
    if r.string()? != CERT_TYPE.as_bytes() { return Err("certificate type mismatch".into()); }
    r.string()?; // nonce
    let public = key32(r.string()?)?;
    let serial = r.u64()?;
    let kind = r.u32()?;
    let key_id = utf8(r.string()?)?;
    let mut principals = Vec::new();
    let mut pr = Reader(r.string()?);
    while !pr.0.is_empty() { principals.push(utf8(pr.string()?)?); }
    let valid_after = r.u64()?;
    let valid_before = r.u64()?;
    let mut critical_options = Vec::new();
    let mut cr = Reader(r.string()?);
    while !cr.0.is_empty() {
        let name = utf8(cr.string()?)?;
        let value = utf8(Reader(cr.string()?).string()?)?;
        critical_options.push((name, value));
    }
    let mut extensions = Vec::new();
    let mut er = Reader(r.string()?);
    while !er.0.is_empty() {
        extensions.push(utf8(er.string()?)?);
        er.string()?;
    }
    r.string()?; // reserved
    let mut sr = Reader(r.string()?);
    if sr.string()? != KEY_TYPE.as_bytes() { return Err("signer is not Ed25519".into()); }
    let signer = key32(sr.string()?)?;
    let signed_len = blob.len() - r.0.len();
    let mut sig = Reader(r.string()?);
    if sig.string()? != KEY_TYPE.as_bytes() { return Err("signature is not Ed25519".into()); }
    let signature = sig.string()?;
    if !r.0.is_empty() { return Err("trailing data".into()); }
    UnparsedPublicKey::new(&ED25519, signer)
        .verify(&blob[..signed_len], signature)
        .map_err(|_| "bad certificate signature".to_string())?;
    Ok(Certificate { public, serial, kind, key_id, principals, valid_after, valid_before, critical_options, extensions, signer })
}

fn key32(b: &[u8]) -> Result<[u8; 32], String> {
    b.try_into().map_err(|_| "key is not 32 bytes".to_string())
}

fn utf8(b: &[u8]) -> Result<String, String> {
    String::from_utf8(b.to_vec()).map_err(|_| "not UTF-8".to_string())
}

fn put_string(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n { return Err("truncated".into()); }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn u32(&mut self) -> Result<u32, String> { Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes"))) }
    fn u64(&mut self) -> Result<u64, String> { Ok(u64::from_be_bytes(self.take(8)?.try_into().expect("8 bytes"))) }
    fn string(&mut self) -> Result<&'a [u8], String> {
        let n = self.u32()? as usize;
        self.take(n)
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() { out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char); } else { out.push('='); }
        }
    }
    out
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = B64.iter().position(|&b| b == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec<'a>(principals: &'a [String], options: &'a [(&'a str, String)]) -> CertSpec<'a> {
        CertSpec {
            kind: USER_CERT,
            serial: 7,
            key_id: "alice@w1",
            principals,
            valid_after: 1_000,
            valid_before: 1_300,
            critical_options: options,
            extensions: &["permit-pty"],
        }
    }

    #[test]
    fn base64_round_trips() {
        for n in 0..40 {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data);
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert!(base64_decode("Zm9v!").is_none());
    }

    #[test]
    fn public_line_round_trips_and_other_types_are_refused() {
        let key = Key::generate().unwrap();
        assert_eq!(&parse_public_line(&key.public_line("me@laptop")).unwrap(), key.public());
        assert!(parse_public_line("ssh-rsa AAAAB3NzaC1yc2E= x").is_err());
        assert!(parse_public_line("ssh-ed25519 !!!").is_err());
    }

    #[test]
    fn certificate_fields_and_signature_verify() {
        let ca = Key::generate().unwrap();
        let user = Key::generate().unwrap();
        let principals = vec!["maint".to_string()];
        let options = [("source-address", "192.168.1.20/32".to_string())];
        let line = sign_certificate(&ca, user.public(), &spec(&principals, &options), "c").unwrap();
        let cert = parse_certificate(&line).unwrap();
        assert_eq!(cert.public, *user.public());
        assert_eq!(cert.signer, *ca.public());
        assert_eq!((cert.serial, cert.kind, cert.key_id.as_str()), (7, USER_CERT, "alice@w1"));
        assert_eq!(cert.principals, principals);
        assert_eq!((cert.valid_after, cert.valid_before), (1_000, 1_300));
        assert_eq!(cert.critical_options, vec![("source-address".into(), "192.168.1.20/32".into())]);
        assert_eq!(cert.extensions, vec!["permit-pty".to_string()]);
    }

    #[test]
    fn tampered_certificate_is_rejected() {
        let ca = Key::generate().unwrap();
        let user = Key::generate().unwrap();
        let principals = vec!["maint".to_string()];
        let line = sign_certificate(&ca, user.public(), &spec(&principals, &[]), "").unwrap();
        let mut blob = base64_decode(line.split(' ').nth(1).unwrap()).unwrap();
        let at = blob.len() / 2;
        blob[at] ^= 1;
        assert!(parse_certificate(&format!("{CERT_TYPE} {}", base64_encode(&blob))).is_err());
    }

    /// The real OpenSSH tools accept what we write (skipped where absent).
    #[test]
    fn ssh_keygen_reads_our_private_key_and_certificate() {
        if std::process::Command::new("ssh-keygen").arg("-?").output().is_err() { return; }
        let dir = std::env::temp_dir().join(format!("stephania-sshcert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = Key::generate().unwrap();
        let path = dir.join("id");
        std::fs::write(&path, key.private_pem("test").unwrap()).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let out = std::process::Command::new("ssh-keygen").arg("-y").arg("-f").arg(&path).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).split_whitespace().nth(1), key.public_line("").split(' ').nth(1));
        let ca = Key::generate().unwrap();
        let principals = vec!["maint".to_string()];
        let options = [("source-address", "10.0.0.5/32".to_string())];
        let cert = dir.join("id-cert.pub");
        std::fs::write(&cert, sign_certificate(&ca, key.public(), &spec(&principals, &options), "").unwrap() + "\n").unwrap();
        let out = std::process::Command::new("ssh-keygen").arg("-L").arg("-f").arg(&cert).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(text.contains("user certificate") && text.contains("Key ID: \"alice@w1\"") && text.contains("Serial: 7"), "{text}");
        assert!(text.contains("source-address 10.0.0.5/32") && text.contains("permit-pty"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
