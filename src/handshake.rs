//! RTMP version 3 simple client handshake on an established byte stream.
//! RTMPS requires independently authenticated TLS before calling this.
use std::io::{self, Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

pub const HANDSHAKE_SIZE: usize = 1536;
pub const RTMP_VERSION: u8 = 3;

/// Deterministic timestamp and injected entropy keep the protocol testable.
/// Production MUST use cryptographically secure random entropy.
pub fn client_handshake<T: Read + Write>(
    transport: &mut T,
    timestamp_ms: u32,
    entropy: &[u8; HANDSHAKE_SIZE - 8],
) -> io::Result<()> {
    let mut c1 = [0u8; HANDSHAKE_SIZE];
    c1[..4].copy_from_slice(&timestamp_ms.to_be_bytes());
    c1[8..].copy_from_slice(entropy);
    transport.write_all(&[RTMP_VERSION])?;
    transport.write_all(&c1)?;
    transport.flush()?;
    let mut reply = [0u8; 1 + HANDSHAKE_SIZE * 2];
    transport.read_exact(&mut reply)?;
    if reply[0] != RTMP_VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "RTMP version mismatch"));
    }
    transport.write_all(&reply[1..1 + HANDSHAKE_SIZE])?;
    transport.flush()?;
    Ok(())
}

/// RTMP simple handshake on Linux. The transport should already be TLS-authenticated.
pub fn handshake_linux<T: Read + Write>(transport: &mut T) -> io::Result<()> {
    let mut entropy = [0u8; HANDSHAKE_SIZE - 8];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut entropy)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(io::Error::other)?;
    client_handshake(transport, now.as_millis() as u32, &entropy)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Duplex {
        incoming: std::io::Cursor<Vec<u8>>,
        written: Vec<u8>,
    }
    impl Read for Duplex {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.incoming.read(buf) }
    }
    impl Write for Duplex {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(buf); Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    #[test]
    fn handshake_emits_c0_c1_and_echoes_s1() {
        let mut incoming = vec![RTMP_VERSION];
        let s1 = vec![0x77; HANDSHAKE_SIZE];
        incoming.extend_from_slice(&s1);
        incoming.extend_from_slice(&[0x42; HANDSHAKE_SIZE]);
        let mut io = Duplex { incoming: std::io::Cursor::new(incoming), written: vec![] };
        client_handshake(&mut io, 0x01020304, &[0x12; HANDSHAKE_SIZE-8]).unwrap();
        assert_eq!(io.written.len(), 1 + HANDSHAKE_SIZE*2);
        assert_eq!(io.written[0], 3);
        assert_eq!(&io.written[1..5], &[1,2,3,4]);
        assert_eq!(&io.written[9..1+HANDSHAKE_SIZE], &[0x12; HANDSHAKE_SIZE-8]);
        assert_eq!(&io.written[1+HANDSHAKE_SIZE..], s1.as_slice());
    }
    #[test]
    fn rejects_bad_protocol_version() {
        let mut io = Duplex { incoming: std::io::Cursor::new(vec![4; 1+HANDSHAKE_SIZE*2]), written: vec![] };
        assert_eq!(client_handshake(&mut io, 0, &[0; HANDSHAKE_SIZE-8]).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
    #[test]
    fn rejects_short_server_handshake() {
        let mut io = Duplex { incoming: std::io::Cursor::new(vec![3;4]), written: vec![] };
        assert_eq!(client_handshake(&mut io, 0, &[0; HANDSHAKE_SIZE-8]).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }
}
