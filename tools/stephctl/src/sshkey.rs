//! A fresh Ed25519 key for one maintenance login: the public line the
//! device certifies and the `openssh-key-v1` private key the system `ssh`
//! reads (PROTOCOL.key, unencrypted). Same encoding as the device's
//! `src/sshcert.rs`.
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair};

const KEY_TYPE: &str = "ssh-ed25519";

pub struct Key {
    seed: [u8; 32],
    public: [u8; 32],
}

impl Drop for Key {
    fn drop(&mut self) {
        self.seed.fill(0);
    }
}

impl Key {
    pub fn generate() -> Result<Self, String> {
        let mut seed = [0u8; 32];
        SystemRandom::new().fill(&mut seed).map_err(|_| "random source failed".to_string())?;
        let pair = Ed25519KeyPair::from_seed_unchecked(&seed).map_err(|_| "invalid Ed25519 seed".to_string())?;
        let mut public = [0u8; 32];
        public.copy_from_slice(pair.public_key().as_ref());
        Ok(Self { seed, public })
    }

    fn public_blob(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_string(&mut out, KEY_TYPE.as_bytes());
        put_string(&mut out, &self.public);
        out
    }

    /// `ssh-ed25519 AAAA… comment`.
    pub fn public_line(&self, comment: &str) -> String {
        format!("{KEY_TYPE} {} {comment}", base64(&self.public_blob()))
    }

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
        both.fill(0);
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
        private.fill(0);
        let encoded = base64(&body);
        body.fill(0);
        let mut pem = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
        for chunk in encoded.as_bytes().chunks(70) {
            pem.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
            pem.push('\n');
        }
        pem.push_str("-----END OPENSSH PRIVATE KEY-----\n");
        Ok(pem)
    }
}

fn put_string(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        let v = ALPHABET.iter().position(|&a| a == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            assert_eq!(base64_decode(&base64(data)).unwrap(), data);
        }
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn private_key_has_the_openssh_shape() {
        let key = Key::generate().unwrap();
        let pem = key.private_pem("t").unwrap();
        assert!(pem.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n"));
        let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        assert!(base64_decode(&body).unwrap().starts_with(b"openssh-key-v1\0"));
        assert!(key.public_line("t").starts_with("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5"));
    }
}
