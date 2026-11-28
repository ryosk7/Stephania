//! mDNS: devices publish `_stephania._tcp` on the control port (avahi on
//! the device). Used by `login` without a device and to find a device whose
//! address changed.
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Found {
    /// `<host>.local`
    pub host: String,
    pub addresses: Vec<String>,
    pub port: u16,
}

pub fn browse(wait: Duration) -> Vec<Found> {
    let Ok(daemon) = ServiceDaemon::new() else { return Vec::new() };
    let Ok(rx) = daemon.browse("_stephania._tcp.local.") else { return Vec::new() };
    let deadline = Instant::now() + wait;
    let mut found: Vec<Found> = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let host = info.get_hostname().trim_end_matches('.').to_string();
                let mut addresses: Vec<String> = info.get_addresses().iter().map(|a| a.to_string()).collect();
                // IPv4 first: link-local IPv6 needs a zone the URL cannot carry.
                addresses.sort_by_key(|a| a.contains(':'));
                if let Some(f) = found.iter_mut().find(|f| f.host == host) {
                    for a in addresses { if !f.addresses.contains(&a) { f.addresses.push(a); } }
                } else {
                    found.push(Found { host, addresses, port: info.get_port() });
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.shutdown();
    found
}

/// Addresses for `<host>.local`, from a short browse.
pub fn resolve(host: &str) -> Vec<String> {
    browse(Duration::from_secs(3)).into_iter().find(|f| f.host.eq_ignore_ascii_case(host)).map(|f| f.addresses).unwrap_or_default()
}
