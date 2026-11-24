//! RTMP simple handshake (C0/C1, S0/S1/S2, C2), not a TLS transport.
//!
//! Use only after an authenticated TLS connection has been established for
//! RTMPS. Complex Adobe handshake schemes are not supported.
use std::fs::File;
use std::io::{self, Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

const BLOCK: usize = 1536;
const RTMP_VERSION: u8 = 3;

pub fn client_handshake<T: Read + Write>(connection: &mut T) -> io::Result<()> {
    let mut c1 = [0u8; BLOCK];
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?.as_secs() as u32;
    c1[..4].copy_from_slice(&seconds.to_be_bytes());
    // C1 bytes 4..8 are zero for a simple handshake.
    File::open("/dev/urandom")?.read_exact(&mut c1[8..])?;

    connection.write_all(&[RTMP_VERSION])?;
    connection.write_all(&c1)?;
    connection.flush()?;

    let mut version = [0u8; 1];
    connection.read_exact(&mut version)?;
    if version[0] != RTMP_VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported RTMP version"));
    }

    let mut s1 = [0u8; BLOCK];
    let mut s2 = [0u8; BLOCK];
    connection.read_exact(&mut s1)?;
    connection.read_exact(&mut s2)?;

    // In a simple handshake, S2 should echo the client's random bytes
    // (the server can replace bytes 4..7 with the time at which C1 was received).
    if s2[..4] != c1[..4] || s2[8..] != c1[8..] {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "RTMP handshake echo mismatch"));
    }

    // C2 echoes S1. Bytes 4..7 (time2) are zero here.
    connection.write_all(&s1)?;
    connection.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Result};

    struct FakePeer {
        reads: Cursor<Vec<u8>>,
        writes: Vec<u8>,
    }
    impl Read for FakePeer {
        fn read(&mut self, out: &mut [u8]) -> Result<usize> { self.reads.read(out) }
    }
    impl Write for FakePeer {
        fn write(&mut self, bytes: &[u8]) -> Result<usize> {
            self.writes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> Result<()> {
            if self.writes.len() == BLOCK + 1 && self.reads.get_ref().is_empty() {
                let c1 = &self.writes[1..];
                let mut s1 = [0x42u8; BLOCK];
                s1[..4].copy_from_slice(&[0,0,0,1]);
                let mut response = vec![3];
                response.extend_from_slice(&s1);
                response.extend_from_slice(c1);
                self.reads = Cursor::new(response);
            }
            Ok(())
        }
    }

    #[test]
    fn simple_handshake_exchanges_complete_blocks() {
        let mut peer = FakePeer { reads: Cursor::new(Vec::new()), writes: Vec::new() };
        client_handshake(&mut peer).unwrap();
        assert_eq!(peer.writes[0], 3);
        assert_eq!(peer.writes.len(), 1 + BLOCK * 2);
        assert_eq!(&peer.writes[BLOCK+1..], &peer.reads.get_ref()[1..BLOCK+1]);
        assert_eq!(&peer.writes[BLOCK+1-1536+8..BLOCK+1-1536+1536].len(), &1528usize);
    }

    #[test]
    fn rejects_wrong_protocol_version() {
        let mut peer=FakePeer { reads: Cursor::new(vec![6]), writes: Vec::new() };
        assert_eq!(client_handshake(&mut peer).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_truncated_server_reply() {
        let mut peer=FakePeer { reads: Cursor::new(vec![3; 10]), writes: Vec::new() };
        assert_eq!(client_handshake(&mut peer).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }
}
