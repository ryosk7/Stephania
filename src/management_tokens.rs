//! Owner-issued management-plane access tokens.
//!
//! The provisioning service issues a token to the authenticated owner over
//! BLE (`management.enroll`, sealed) and stores only its SHA-256. The
//! control plane reads the same file and accepts `Authorization: Bearer
//! <token>` for LAN (TLS) requests. A factory reset clears every token.
//!
//! File: `{"tokens":[{"id","sha256","created_unix"}]}`, mode 0640 in a
//! setgid directory shared with the control group. At most MAX_TOKENS;
//! issuing beyond that drops the oldest.
use crate::provisioning::hex;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;

pub const MAX_TOKENS: usize = 16;
pub const TOKEN_PREFIX: &str = "t3b_";

#[derive(Debug, Clone)]
pub struct TokenStore {
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issued {
    pub id: String,
    pub token: String,
}

impl TokenStore {
    fn read(&self) -> Vec<Value> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
            .and_then(|v| v.get("tokens").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    }

    fn write(&self, tokens: &[Value]) -> std::io::Result<()> {
        let tmp = self.path.with_extension("tmp");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o640);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(json!({"tokens": tokens}).to_string().as_bytes())?;
        f.sync_all()?;
        // Explicit: the service runs with UMask=0077, and the control group
        // must be able to read the hashes.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o640))?;
        }
        std::fs::rename(&tmp, &self.path)
    }

    pub fn count(&self) -> usize { self.read().len() }

    /// Issues a new 256-bit token; persisted (hash only) before returning.
    pub fn issue(&self, now_unix: u64) -> std::io::Result<Issued> {
        let rng = SystemRandom::new();
        let mut secret = [0u8; 32];
        let mut id = [0u8; 4];
        rng.fill(&mut secret).and_then(|_| rng.fill(&mut id)).map_err(|_| std::io::Error::other("random source unavailable"))?;
        let token = format!("{TOKEN_PREFIX}{}", hex(&secret));
        let id = hex(&id);
        let mut tokens = self.read();
        tokens.push(json!({"id": id, "sha256": crate::pki::sha256_hex(token.as_bytes()), "created_unix": now_unix}));
        let excess = tokens.len().saturating_sub(MAX_TOKENS);
        tokens.drain(..excess);
        self.write(&tokens)?;
        Ok(Issued { id, token })
    }

    pub fn clear(&self) -> std::io::Result<()> {
        match std::fs::metadata(&self.path) {
            Ok(_) => self.write(&[]),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issues_hash_only_bounded_and_clears() {
        let dir = std::env::temp_dir().join(format!("t3box-tokens-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = TokenStore { path: dir.join("tokens.json") };
        let first = store.issue(1).unwrap();
        assert!(first.token.starts_with(TOKEN_PREFIX) && first.token.len() == 4 + 64);
        let raw = std::fs::read_to_string(&store.path).unwrap();
        assert!(!raw.contains(&first.token[4..]), "plaintext token must never be stored");
        assert!(raw.contains(&crate::pki::sha256_hex(first.token.as_bytes())));
        for i in 0..MAX_TOKENS { store.issue(2 + i as u64).unwrap(); }
        assert_eq!(store.count(), MAX_TOKENS);
        let ids: Vec<String> = store.read().iter().map(|t| t["id"].as_str().unwrap().to_owned()).collect();
        assert!(!ids.contains(&first.id), "oldest token evicted");
        store.clear().unwrap();
        assert_eq!(store.count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
