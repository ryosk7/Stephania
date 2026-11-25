//! Safe, streaming FLV demultiplexing of already-encoded AVC/AAC packets.
//!
//! A FLV file header and PreviousTagSize fields are framing. On RTMP, only
//! the tag's payload and timestamp should be sent as message types 8, 9, 18.
use std::io::{self, Read};

const MAX_HEADER: u32 = 4096;
const MAX_TAG: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Tag {
    pub kind: u8,
    pub timestamp_ms: u32,
    pub payload: Vec<u8>,
}

pub struct FlvReader<R: Read> {
    source: R,
}

impl<R: Read> FlvReader<R> {
    pub fn new(mut source: R) -> io::Result<Self> {
        let mut header = [0u8; 9];
        source.read_exact(&mut header)?;
        if &header[..3] != b"FLV" || header[3] != 1 || header[4] & !0x05 != 0 {
            return Err(invalid("invalid FLV header"));
        }
        let offset = u32::from_be_bytes(header[5..9].try_into().unwrap());
        if !(9..=MAX_HEADER).contains(&offset) {
            return Err(invalid("invalid FLV header offset"));
        }
        let mut extra = vec![0u8; (offset - 9) as usize];
        source.read_exact(&mut extra)?;
        let mut previous = [0u8; 4];
        source.read_exact(&mut previous)?;
        if previous != [0; 4] {
            return Err(invalid("FLV initial PreviousTagSize must be zero"));
        }
        Ok(Self { source })
    }

    pub fn next_tag(&mut self) -> io::Result<Option<Tag>> {
        let mut header = [0u8; 11];
        match self.source.read(&mut header[..1])? {
            0 => return Ok(None),
            1 => {}
            _ => unreachable!(),
        }
        self.source.read_exact(&mut header[1..])?;
        let kind = header[0];
        if ![8, 9, 18].contains(&kind) {
            return Err(invalid("unsupported FLV tag type"));
        }
        // The FLV StreamID field is reserved and must remain zero.
        if header[8..11] != [0, 0, 0] {
            return Err(invalid("nonzero FLV StreamID"));
        }
        let size = read24(&header[1..4]) as usize;
        if size == 0 || size > MAX_TAG {
            return Err(invalid("invalid or oversized FLV tag length"));
        }
        let timestamp_ms = read24(&header[4..7]) | (u32::from(header[7]) << 24);
        let mut payload = vec![0u8; size];
        self.source.read_exact(&mut payload)?;
        let mut previous = [0u8; 4];
        self.source.read_exact(&mut previous)?;
        if u32::from_be_bytes(previous) != size as u32 + 11 {
            return Err(invalid("incorrect FLV PreviousTagSize"));
        }
        if (kind == 8 && payload[0] >> 4 != 10)
            || (kind == 9 && payload[0] & 0x0f != 7) {
            return Err(invalid("only AAC and AVC media are supported"));
        }
        Ok(Some(Tag { kind, timestamp_ms, payload }))
    }
}

fn read24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2])
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flv::FlvMuxer;

    #[test]
    fn round_trips_avc_and_aac_tag_bodies() {
        let mut mux = FlvMuxer::new(Vec::new()).unwrap();
        mux.avc_configuration(0, &[0x67, 0x42, 0, 0x1f], &[0x68, 1]).unwrap();
        mux.aac_configuration(0, &[0x11, 0x90]).unwrap();
        mux.aac_frame(21, &[1, 2, 3]).unwrap();
        mux.avc_frame(33, 0, true, &[&[0x65, 0xaa]]).unwrap();
        let bytes = mux.into_inner();
        let mut reader = FlvReader::new(bytes.as_slice()).unwrap();
        let mut tags = Vec::new();
        while let Some(tag) = reader.next_tag().unwrap() { tags.push(tag); }
        assert_eq!(tags.len(), 4);
        assert_eq!((tags[0].kind, tags[0].timestamp_ms), (9, 0));
        assert_eq!((tags[1].kind, tags[1].timestamp_ms), (8, 0));
        assert_eq!((tags[2].kind, tags[2].timestamp_ms), (8, 21));
        assert_eq!(tags[3].payload[0], 0x17);
        assert_eq!(tags[3].timestamp_ms, 33);
    }

    #[test]
    fn rejects_bad_file_header_and_truncated_tag() {
        assert!(FlvReader::new(b"notflv".as_slice()).is_err());
        assert!(FlvReader::new(b"FLV\x01\x05\0\0\0\x09\0\0\0\x01".as_slice()).is_err());
        let mut reader = FlvReader::new(b"FLV\x01\x05\0\0\0\x09\0\0\0\0\x09\0".as_slice()).unwrap();
        assert_eq!(reader.next_tag().unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn rejects_wrong_previous_tag_size_and_video_codec() {
        let mut mux = FlvMuxer::new(Vec::new()).unwrap();
        mux.aac_frame(0, &[0x01]).unwrap();
        let mut bytes = mux.into_inner();
        *bytes.last_mut().unwrap() = 0;
        assert!(FlvReader::new(bytes.as_slice()).unwrap().next_tag().is_err());
    }

    #[test]
    fn handles_script_metadata_tags() {
        let mut flv = b"FLV\x01\x05\0\0\0\x09\0\0\0\0".to_vec();
        flv.extend_from_slice(&[18, 0, 0, 3, 0, 0, 1, 0, 0, 0, 0, 0x02, 0, 0]);
        flv.extend_from_slice(&14u32.to_be_bytes());
        let mut reader = FlvReader::new(flv.as_slice()).unwrap();
        let tag = reader.next_tag().unwrap().unwrap();
        assert_eq!(tag.kind, 18);
        assert_eq!(tag.timestamp_ms, 1);
        assert_eq!(tag.payload.len(), 3);
        assert!(reader.next_tag().unwrap().is_none());
    }
}
