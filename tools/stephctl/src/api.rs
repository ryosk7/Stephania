//! HTTPS to the device's control plane (port 9443), trusting only the
//! pinned device CA: no system trust store, no fallback. The one exception
//! is the first fetch of `/ca.pem`, whose fingerprint the user confirms.
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use ureq::tls::{Certificate, RootCerts, TlsConfig};

pub const COOKIE: &str = "stephania_session";

pub struct Response {
    pub status: u16,
    pub body: Value,
    pub set_cookie: Option<String>,
    pub retry_after: Option<u64>,
}

impl Response {
    pub fn error_code(&self) -> &str { self.body.get("error").and_then(|e| e.get("code")).and_then(Value::as_str).unwrap_or("") }
}

/// `https://<address>:<port>`, with IPv6 literals in brackets.
pub fn base_url(address: &str, port: u16) -> String {
    if address.contains(':') && !address.starts_with('[') { format!("https://[{address}]:{port}") } else { format!("https://{address}:{port}") }
}

fn agent(tls: TlsConfig) -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(4)))
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into()
}

/// First contact only: download the CA certificate without verifying the
/// server, so its fingerprint can be shown and compared with Connect.
pub fn fetch_ca_unverified(address: &str, port: u16) -> Result<String, String> {
    let agent = agent(TlsConfig::builder().disable_verification(true).build());
    let mut resp = agent.get(&format!("{}/ca.pem", base_url(address, port))).call().map_err(|e| e.to_string())?;
    if resp.status().as_u16() != 200 {
        return Err(format!("{} answered {} for /ca.pem", address, resp.status().as_u16()));
    }
    let pem = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    pem_to_der(&pem).ok_or("the device sent no certificate")?;
    Ok(pem)
}

pub fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    let start = pem.find("-----BEGIN CERTIFICATE-----")? + "-----BEGIN CERTIFICATE-----".len();
    let end = pem[start..].find("-----END CERTIFICATE-----")? + start;
    crate::sshkey::base64_decode(&pem[start..end]).filter(|d| !d.is_empty())
}

pub fn sha256_hex(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The first 16 bytes as 8 groups of 4 upper-case hex digits, as Connect
/// shows it (`shortFingerprint` in packages/provisioning-client).
pub fn short_fingerprint(sha256_hex: &str) -> String {
    sha256_hex.get(..32).unwrap_or(sha256_hex).to_uppercase().as_bytes().chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or("")).collect::<Vec<_>>().join(" ")
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
}

impl Client {
    pub fn new(ca_pem: &str, address: &str, port: u16) -> Result<Self, String> {
        let der = pem_to_der(ca_pem).ok_or("pinned CA is not a PEM certificate")?;
        let roots = RootCerts::Specific(Arc::new(vec![Certificate::from_der(&der).to_owned()]));
        Ok(Self { agent: agent(TlsConfig::builder().root_certs(roots).build()), base: base_url(address, port) })
    }

    /// One request. `session` adds the cookie, and the CSRF token on
    /// state-changing calls.
    pub fn call(&self, method: &str, path: &str, body: Option<&Value>, session: Option<(&str, &str)>) -> Result<Response, ureq::Error> {
        let url = format!("{}{path}", self.base);
        let cookie = session.map(|(c, _)| format!("{COOKIE}={c}"));
        let csrf = session.map(|(_, t)| t.to_string());
        let mut resp = match method {
            "GET" => {
                let mut r = self.agent.get(&url);
                if let Some(c) = &cookie { r = r.header("Cookie", c); }
                r.call()?
            }
            "DELETE" => {
                let mut r = self.agent.delete(&url);
                if let Some(c) = &cookie { r = r.header("Cookie", c); }
                if let Some(t) = &csrf { r = r.header("X-CSRF-Token", t); }
                r.call()?
            }
            _ => {
                let mut r = self.agent.post(&url).header("Content-Type", "application/json");
                if let Some(c) = &cookie { r = r.header("Cookie", c); }
                if let Some(t) = &csrf { r = r.header("X-CSRF-Token", t); }
                r.send(serde_json::to_vec(body.unwrap_or(&Value::Null)).unwrap_or_default())?
            }
        };
        let status = resp.status().as_u16();
        let set_cookie = resp.headers().get_all("set-cookie").iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| v.strip_prefix(&format!("{COOKIE}=")).map(|rest| rest.split(';').next().unwrap_or("").to_string()));
        let retry_after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let body = if text.is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) };
        Ok(Response { status, body, set_cookie, retry_after })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_fingerprint_matches_connect() {
        assert_eq!(short_fingerprint(&"ab".repeat(32)), "ABAB ABAB ABAB ABAB ABAB ABAB ABAB ABAB");
    }

    #[test]
    fn ipv6_addresses_are_bracketed() {
        assert_eq!(base_url("fe80::1", 9443), "https://[fe80::1]:9443");
        assert_eq!(base_url("192.168.1.2", 9443), "https://192.168.1.2:9443");
    }
}
