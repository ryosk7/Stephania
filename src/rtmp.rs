//! RTMP chunk serializer. TCP/TLS, handshake, AMF commands and reconnection
//! are intentionally out of scope. Message format follows RTMP v1 fmt0/fmt3.
use std::io::{self, Write};

pub struct RtmpChunkWriter<W: Write> {
    out: W,
    chunk_size: usize,
}

impl<W: Write> RtmpChunkWriter<W> {
    pub fn new(out: W, chunk_size: usize) -> io::Result<Self> {
        if chunk_size == 0 || chunk_size > 0x7fff_ffff {
            return Err(invalid("chunk size must be positive and RTMP-compatible"));
        }
        Ok(Self { out, chunk_size })
    }

    pub fn into_inner(self) -> W { self.out }

    /// Serializes one complete RTMP message using 1-byte basic headers.
    ///
    /// Chunk stream ID is 2..=63; message stream ID is little-endian in RTMP.
    /// Length is capped at 24 bits by RTMP. Extended timestamp is used when needed.
    /// The caller is responsible for establishing/maintaining the RTMP connection,
    /// including TLS, handshake, AMF commands and negotiated chunk size.
    pub fn message(
        &mut self,
        chunk_stream_id: u8,
        message_type: u8,
        message_stream_id: u32,
        timestamp_ms: u32,
        body: &[u8],
    ) -> io::Result<()> {
        if !(2..=63).contains(&chunk_stream_id) {
            return Err(invalid("unsupported chunk stream ID"));
        }
        if body.len() > 0x00ff_ffff { return Err(invalid("RTMP message larger than 24 bits")); }

        let extended = timestamp_ms >= 0x00ff_ffff;
        let ts24 = if extended { 0x00ff_ffff } else { timestamp_ms };
        let size = body.len() as u32;

        // fmt=0 basic header: 0b00xxxxxx.
        self.out.write_all(&[chunk_stream_id])?;
        self.out.write_all(&[(ts24 >> 16) as u8, (ts24 >> 8) as u8, ts24 as u8])?;
        self.out.write_all(&[(size >> 16) as u8, (size >> 8) as u8, size as u8])?;
        self.out.write_all(&[message_type])?;
        self.out.write_all(&message_stream_id.to_le_bytes())?;
        if extended { self.out.write_all(&timestamp_ms.to_be_bytes())?; }

        for (i, part) in body.chunks(self.chunk_size).enumerate() {
            if i > 0 {
                // fmt=3 (same message header as previous chunk).
                self.out.write_all(&[0xc0 | chunk_stream_id])?;
                if extended { self.out.write_all(&timestamp_ms.to_be_bytes())?; }
            }
            self.out.write_all(part)?;
        }
        Ok(())
    }
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_boundaries_and_stream_id_endianness() {
        let mut w=RtmpChunkWriter::new(Vec::new(), 3).unwrap();
        w.message(6, 9, 0x01020304, 1000, &[1,2,3,4,5,6,7]).unwrap();
        let b=w.into_inner();
        assert_eq!(&b[..12], &[6,0,3,232,0,0,7,9,4,3,2,1]);
        assert_eq!(&b[12..], &[1,2,3,0xc6,4,5,6,0xc6,7]);
    }
    #[test]
    fn extended_timestamp_repeated_on_continuation() {
        let mut w=RtmpChunkWriter::new(Vec::new(),2).unwrap();
        w.message(4,8,1,0x01234567,&[9,10,11]).unwrap();
        let b=w.into_inner();
        assert_eq!(&b[1..4], &[0xff,0xff,0xff]);
        assert_eq!(&b[12..16], &[0x01,0x23,0x45,0x67]);
        assert_eq!(&b[16..], &[9,10,0xc4,0x01,0x23,0x45,0x67,11]);
    }
    #[test]
    fn invalid_params_are_rejected() {
        assert!(RtmpChunkWriter::new(Vec::new(),0).is_err());
        assert!(RtmpChunkWriter::new(Vec::new(),0x8000_0000).is_err());
        let mut w=RtmpChunkWriter::new(Vec::new(),128).unwrap();
        assert!(w.message(1,8,1,0,&[1]).is_err());
        assert!(w.message(64,9,1,0,&[1]).is_err());
    }
    #[test]
    fn empty_message_has_only_header() {
        let mut w=RtmpChunkWriter::new(Vec::new(),128).unwrap();
        w.message(2,1,0,0,&[]).unwrap();
        assert_eq!(w.into_inner().len(),12);
    }
}
