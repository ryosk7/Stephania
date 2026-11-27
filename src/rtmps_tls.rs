//! Verified TLS transport and RTMPS URL parser.
//!
//! TLS uses rustls with WebPKI trust anchors and hostname verification.
//! The RTMP session logic remains implemented in Stephania Rust.
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::{pki_types::ServerName, ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use crate::{handshake, session::PublishSession};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const IO_TIMEOUT: Duration = Duration::from_secs(12);

/// Deliberately no Debug: stream_name is usually a private live key.
pub struct Target {
    host: String,
    port: u16,
    app: String,
    stream_name: String,
}

impl Target {
    /// Accept a DNS-host RTMPS URL with exactly two path segments.
    pub fn parse(url: &str) -> io::Result<Self> {
        let body = url.strip_prefix("rtmps://").ok_or_else(invalid)?;
        if body.contains(['?', '#', '@', '\\', '\0', '\n', '\r']) {
            return Err(invalid());
        }
        let (authority, path) = body.split_once('/').ok_or_else(invalid)?;
        let (app, stream_name) = path.split_once('/').ok_or_else(invalid)?;
        if app.is_empty() || stream_name.is_empty() ||
            stream_name.contains('/') || app.len() > 128 || stream_name.len() > 256 ||
            app.contains('%') || stream_name.contains('%') {
            return Err(invalid());
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                let port = port.parse::<u16>().map_err(|_| invalid())?;
                if port == 0 { return Err(invalid()); }
                (host, port)
            }
            None => (authority, 443),
        };
        if !valid_hostname(host) { return Err(invalid()); }
        Ok(Self {
            host: host.into(), port, app: app.into(), stream_name: stream_name.into()
        })
    }
    pub fn server_address(&self) -> String { format!("{}:{}", self.host, self.port) }
}

fn valid_hostname(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 || !host.contains('.') ||
        host.parse::<std::net::Ipv4Addr>().is_ok() { return false; }
    host.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63 &&
        !label.starts_with('-') && !label.ends_with('-') &&
        label.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    })
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid or unsupported RTMPS URL")
}

/// Open TLS with read/write deadlines; never bypass certificate verification.
pub fn tls_connect(target: &Target) -> io::Result<StreamOwned<ClientConnection, TcpStream>> {
    let addr = target.server_address();
    let mut last = None;
    let mut tcp = None;
    for socket in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&socket, CONNECT_TIMEOUT) {
            Ok(stream) => { tcp = Some(stream); break; }
            Err(e) => last = Some(e),
        }
    }
    let stream = tcp.ok_or_else(|| last.unwrap_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "no RTMPS address")))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_nodelay(true)?;

    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    #[cfg(feature = "ci-virtual")]
    let roots = {
        let mut roots = roots;
        if target.host == "stephania-ci.test" {
            if let Some(path) = std::env::var_os("STEPHANIA_CI_CA_DER") {
                roots.add(rustls::pki_types::CertificateDer::from(std::fs::read(path)?))
                    .map_err(io::Error::other)?;
            }
        }
        roots
    };
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server = ServerName::try_from(target.host.clone()).map_err(|_| invalid())?;
    let connection = ClientConnection::new(Arc::new(config), server).map_err(io::Error::other)?;
    Ok(StreamOwned::new(connection, stream))
}

/// Establish authenticated TLS, RTMP simple handshake, connect/createStream,
/// and wait for publish confirmation. No media packets are sent yet.
pub fn begin_publish(url: &str) -> io::Result<PublishSession<StreamOwned<ClientConnection, TcpStream>>> {
    let target = Target::parse(url)?;
    let mut tls = tls_connect(&target)?;
    handshake::handshake_linux(&mut tls)?;
    let app_url = format!("rtmps://{}:{}/{}", target.host, target.port, target.app);
    let mut session = PublishSession::new(tls);
    session.connect(&target.app, &app_url)?;
    session.create_stream()?;
    session.publish(&target.stream_name)?;
    Ok(session)
}

#[cfg(target_os = "linux")]
impl PublishSession<StreamOwned<ClientConnection, TcpStream>> {
    /// Duplicate the socket handle for read-only telemetry, not TLS I/O.
    /// The monitor must be dropped with the session; it shares socket lifetime.
    pub fn tcp_monitor(&self) -> io::Result<TcpStream> {
        self.transport().sock.try_clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_youtube_url_without_exposing_secret() {
        let t = Target::parse("rtmps://a.rtmps.youtube.com/live2/PRIVATE-STREAM-KEY").unwrap();
        assert_eq!(t.server_address(), "a.rtmps.youtube.com:443");
        let t = Target::parse("rtmps://ingest.example.net:443/live2/test").unwrap();
        assert_eq!(t.server_address(), "ingest.example.net:443");
    }
    #[test]
    fn rejects_non_tls_and_malformed_urls() {
        for url in [
            "rtmp://a.rtmps.youtube.com/live2/key",
            "rtmps://127.0.0.1/live2/key",
            "rtmps://host/live2/key",
            "rtmps://user@server.test/live2/key",
            "rtmps://server.test/live2/",
            "rtmps://server.test/live2/key/extra",
            "rtmps://server.test/live2/key?token=bad",
            "rtmps://server.test:0/live2/key",
            "rtmps://server.test/live2/key%2fvalue",
        ] {
            assert!(Target::parse(url).is_err(), "unexpected URL accepted");
        }
    }
}

