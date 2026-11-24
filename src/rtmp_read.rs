//! Bounded RTMP chunk reader. This is the inbound counterpart of rtmp.rs.
//! Supports RTMP chunk basic headers with 1-byte CSIDs (2..63), fmt 0..3,
//! chunk-size updates, interleaved streams and bounded reassembly.
//! Explicitly rejects extended CSID headers and unsupported oversized payloads.
use std::collections::HashMap;
use std::io::{self, Read};

const MAX_MESSAGE: usize = 1_048_576;
const MAX_CHUNK: usize = 65_536;
const MAX_STREAMS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub stream_id: u32,
    pub type_id: u8,
    pub timestamp_ms: u32,
    pub payload: Vec<u8>,
}

#[derive(Default)]
struct Partial {
    length: usize,
    type_id: u8,
    stream_id: u32,
    timestamp_ms: u32,
    delta: u32,
    extended: bool,
    payload: Vec<u8>,
    has_header: bool,
    in_progress: bool,
}

pub struct ChunkReader {
    chunk_size: usize,
    streams: HashMap<u8, Partial>,
}

impl Default for ChunkReader {
    fn default() -> Self {
        Self { chunk_size: 128, streams: HashMap::new() }
    }
}

impl ChunkReader {
    pub fn new() -> Self { Self::default() }
    pub fn chunk_size(&self) -> usize { self.chunk_size }

    pub fn read_message<R: Read>(&mut self, reader: &mut R) -> io::Result<Message> {
        // Skips RTMP Set Chunk Size messages as protocol controls, and limits
        // each individual message to 1 MiB (not unlimited peer-controlled memory).
        loop {
            let message = self.read_one(reader)?;
            if message.type_id != 1 { return Ok(message); }
            if message.payload.len() != 4 {
                return Err(invalid("invalid Set Chunk Size payload"));
            }
            let raw = u32::from_be_bytes(message.payload[..4].try_into().unwrap());
            if raw & 0x8000_0000 != 0 || raw == 0 || raw as usize > MAX_CHUNK {
                return Err(invalid("unsafe RTMP chunk size"));
            }
            self.chunk_size = raw as usize;
        }
    }

    fn read_one<R: Read>(&mut self, reader: &mut R) -> io::Result<Message> {
        loop {
            let mut basic = [0u8; 1];
            reader.read_exact(&mut basic)?;
            let fmt = basic[0] >> 6;
            let csid = basic[0] & 0x3f;
            if csid < 2 { return Err(invalid("extended chunk stream IDs not supported")); }
            if !self.streams.contains_key(&csid) && self.streams.len() >= MAX_STREAMS {
                return Err(invalid("too many RTMP chunk streams"));
            }
            let part = self.streams.entry(csid).or_default();
            let pending = part.in_progress;
            if pending && fmt != 3 {
                return Err(invalid("new RTMP message header before previous is complete"));
            }
            if !pending {
                part.payload.clear();
                part.in_progress = true;
            }

            let mut ts24 = 0u32;
            match fmt {
                0 => {
                    let mut header = [0u8; 11];
                    reader.read_exact(&mut header)?;
                    ts24 = read24(&header[..3]);
                    part.length = read24(&header[3..6]) as usize;
                    part.type_id = header[6];
                    part.stream_id = u32::from_le_bytes(header[7..11].try_into().unwrap());
                    part.delta = 0;
                    part.extended = ts24 == 0x00ff_ffff;
                    part.timestamp_ms = ts24;
                    part.has_header = true;
                }
                1 if part.has_header => {
                    let mut header = [0u8; 7];
                    reader.read_exact(&mut header)?;
                    ts24 = read24(&header[..3]);
                    part.length = read24(&header[3..6]) as usize;
                    part.type_id = header[6];
                    part.extended = ts24 == 0x00ff_ffff;
                    part.delta = ts24;
                }
                2 if part.has_header => {
                    let mut header = [0u8; 3];
                    reader.read_exact(&mut header)?;
                    ts24 = read24(&header);
                    part.delta = ts24;
                    part.extended = ts24 == 0x00ff_ffff;
                }
                3 if part.has_header => {}
                _ => return Err(invalid("missing prior RTMP chunk header")),
            }
            if part.length > MAX_MESSAGE { return Err(invalid("RTMP message exceeds 1 MiB")); }

            if part.extended {
                let mut extension = [0u8; 4];
                reader.read_exact(&mut extension)?;
                let extended_ts = u32::from_be_bytes(extension);
                if fmt == 0 { part.timestamp_ms = extended_ts; }
                if fmt == 1 || fmt == 2 { part.delta = extended_ts; }
            }

            if fmt == 1 || fmt == 2 || (fmt == 3 && !pending) {
                part.timestamp_ms = part.timestamp_ms.wrapping_add(part.delta);
            }

            let take = (part.length - part.payload.len()).min(self.chunk_size);
            if take > 0 {
                let before = part.payload.len();
                part.payload.resize(before + take, 0);
                reader.read_exact(&mut part.payload[before..])?;
            }
            if part.payload.len() == part.length {
                part.in_progress = false;
                return Ok(Message {
                    stream_id: part.stream_id,
                    type_id: part.type_id,
                    timestamp_ms: part.timestamp_ms,
                    payload: std::mem::take(&mut part.payload),
                });
            }
        }
    }
}

fn read24(bytes: &[u8]) -> u32 {
    (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2])
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtmp::RtmpChunkWriter;

    #[test]
    fn round_trip_video_with_short_chunks() {
        let mut writer = RtmpChunkWriter::new(Vec::new(), 3).unwrap();
        writer.message(6, 9, 1, 42, &[1, 2, 3, 4, 5, 6, 7]).unwrap();
        let bytes = writer.into_inner();
        let mut reader = ChunkReader::new();
        reader.chunk_size = 3;
        let m = reader.read_message(&mut bytes.as_slice()).unwrap();
        assert_eq!(m.stream_id, 1);
        assert_eq!(m.type_id, 9);
        assert_eq!(m.timestamp_ms, 42);
        assert_eq!(m.payload, vec![1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn set_chunk_size_is_applied() {
        let mut w = RtmpChunkWriter::new(Vec::new(), 128).unwrap();
        w.message(2, 1, 0, 0, &4u32.to_be_bytes()).unwrap();
        let first = w.into_inner();
        let mut w = RtmpChunkWriter::new(Vec::new(), 4).unwrap();
        w.message(5, 20, 0, 0, &[1, 2, 3, 4, 5, 6, 7]).unwrap();
        let mut all = first;
        all.extend(w.into_inner());
        let mut r = ChunkReader::new();
        let msg = r.read_message(&mut all.as_slice()).unwrap();
        assert_eq!(r.chunk_size(), 4);
        assert_eq!(msg.payload, vec![1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn supports_fmt1_delta_after_complete_message() {
        let mut w = RtmpChunkWriter::new(Vec::new(), 128).unwrap();
        w.message(3, 20, 0, 100, &[1]).unwrap();
        let mut bytes = w.into_inner();
        // fmt1, timestamp delta 40, size 1, type AMF0 command.
        bytes.extend_from_slice(&[0x43, 0, 0, 40, 0, 0, 1, 20, 2]);
        let mut r = ChunkReader::new();
        let mut stream = bytes.as_slice();
        assert_eq!(r.read_message(&mut stream).unwrap().timestamp_ms, 100);
        let m = r.read_message(&mut stream).unwrap();
        assert_eq!(m.timestamp_ms, 140);
        assert_eq!(m.payload, vec![2]);
    }

    #[test]
    fn rejects_invalid_chunk_size_and_truncated_payload() {
        let mut w = RtmpChunkWriter::new(Vec::new(), 128).unwrap();
        w.message(2, 1, 0, 0, &0u32.to_be_bytes()).unwrap();
        assert!(ChunkReader::new().read_message(&mut w.into_inner().as_slice()).is_err());
        let truncated = [6, 0, 0, 0, 0, 0, 4, 9, 1, 0, 0, 0, 1, 2];
        assert_eq!(
            ChunkReader::new().read_message(&mut truncated.as_slice()).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
