//! Minimal FLV muxer for pre-encoded H.264/AVC + AAC.
//!
//! This module writes FLV tags directly without FFmpeg/GStreamer. It does not
//! encode H.264, AAC, synchronize independent device clocks, or implement RTMPS.
//! Caller must pass H.264 NAL units without Annex-B start codes.
use std::io::{self, Write};

const MAX_TAG_PAYLOAD: usize = 0x00ff_ffff;

pub struct FlvMuxer<W: Write> {
    output: W,
    last_timestamp: Option<u32>,
}

impl<W: Write> FlvMuxer<W> {
    /// Construct an FLV stream containing AAC audio and AVC video.
    pub fn new(mut output: W) -> io::Result<Self> {
        // "FLV", version 1, audio+video flags, 9-byte header, previous-tag-size = 0.
        output.write_all(b"FLV\x01\x05\0\0\0\x09\0\0\0\0")?;
        Ok(Self { output, last_timestamp: None })
    }

    pub fn into_inner(self) -> W { self.output }

    /// Write the AVC sequence header using raw SPS and PPS NAL units.
    pub fn avc_configuration(&mut self, timestamp_ms: u32, sps: &[u8], pps: &[u8]) -> io::Result<()> {
        if sps.len() < 4 || pps.is_empty() || sps.len() > u16::MAX as usize || pps.len() > u16::MAX as usize {
            return Err(invalid("invalid SPS/PPS size"));
        }
        if (sps[0] & 0x1f) != 7 || (pps[0] & 0x1f) != 8 {
            return Err(invalid("expected SPS (type 7) and PPS (type 8)"));
        }
        let mut p = Vec::with_capacity(18 + sps.len() + pps.len());
        // FrameType=keyframe, CodecID=AVC, AVCPacketType=0, composition time=0.
        p.extend_from_slice(&[0x17, 0, 0, 0, 0]);
        // AVCDecoderConfigurationRecord (ISO/IEC 14496-15).
        p.extend_from_slice(&[1, sps[1], sps[2], sps[3], 0xff, 0xe1]);
        p.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        p.extend_from_slice(sps);
        p.push(1); // one PPS
        p.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        p.extend_from_slice(pps);
        self.write_tag(9, timestamp_ms, &p)
    }

    /// Write a frame containing one or more length-prefixed AVC NAL units.
    /// Composition time (PTS-DTS) is signed 24-bit milliseconds.
    pub fn avc_frame(&mut self, dts_ms: u32, cts_ms: i32, keyframe: bool, nals: &[&[u8]]) -> io::Result<()> {
        if !(-8_388_608..=8_388_607).contains(&cts_ms) {
            return Err(invalid("composition offset exceeds signed 24-bit range"));
        }
        if nals.is_empty() { return Err(invalid("empty AVC access unit")); }
        let mut p = Vec::new();
        p.extend_from_slice(&[if keyframe { 0x17 } else { 0x27 }, 1]);
        let bytes = cts_ms.to_be_bytes();
        p.extend_from_slice(&bytes[1..4]);
        for nal in nals {
            if nal.is_empty() || nal.len() > u32::MAX as usize {
                return Err(invalid("invalid AVC NAL size"));
            }
            // A 3- or 4-byte Annex-B start code is not allowed here.
            if nal.starts_with(&[0,0,1]) || nal.starts_with(&[0,0,0,1]) {
                return Err(invalid("AVC NAL contains Annex-B start code"));
            }
            p.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            p.extend_from_slice(nal);
            if p.len() > MAX_TAG_PAYLOAD { return Err(invalid("AVC frame too large for FLV")); }
        }
        self.write_tag(9, dts_ms, &p)
    }

    /// AAC AudioSpecificConfig: supplied as ASC bytes, not ADTS headers.
    pub fn aac_configuration(&mut self, timestamp_ms: u32, asc: &[u8]) -> io::Result<()> {
        if !(2..=16).contains(&asc.len()) { return Err(invalid("invalid AudioSpecificConfig size")); }
        let mut p = vec![0xaf, 0]; // AAC stereo, sequence header
        p.extend_from_slice(asc);
        self.write_tag(8, timestamp_ms, &p)
    }

    /// AAC raw frame, without ADTS header.
    pub fn aac_frame(&mut self, timestamp_ms: u32, raw: &[u8]) -> io::Result<()> {
        if raw.is_empty() { return Err(invalid("empty AAC frame")); }
        let mut p = Vec::with_capacity(2 + raw.len());
        p.extend_from_slice(&[0xaf, 1]);
        p.extend_from_slice(raw);
        self.write_tag(8, timestamp_ms, &p)
    }

    fn write_tag(&mut self, kind: u8, timestamp_ms: u32, payload: &[u8]) -> io::Result<()> {
        if payload.len() > MAX_TAG_PAYLOAD { return Err(invalid("FLV tag too large")); }
        if self.last_timestamp.is_some_and(|last| timestamp_ms < last) {
            return Err(invalid("FLV tag timestamps must be monotonically nondecreasing"));
        }
        let len = payload.len() as u32;
        let mut header = [0u8; 11];
        header[0] = kind;
        header[1] = (len >> 16) as u8;
        header[2] = (len >> 8) as u8;
        header[3] = len as u8;
        header[4] = (timestamp_ms >> 16) as u8;
        header[5] = (timestamp_ms >> 8) as u8;
        header[6] = timestamp_ms as u8;
        header[7] = (timestamp_ms >> 24) as u8;
        // StreamID stays zero.
        self.output.write_all(&header)?;
        self.output.write_all(payload)?;
        self.output.write_all(&(11 + len).to_be_bytes())?;
        self.last_timestamp = Some(timestamp_ms);
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
    fn emits_correct_flv_header_and_tag_sizes() {
        let mut mux = FlvMuxer::new(Vec::new()).unwrap();
        mux.aac_configuration(0, &[0x11, 0x90]).unwrap();
        mux.aac_frame(21, &[1, 2, 3]).unwrap();
        let bytes = mux.into_inner();
        assert_eq!(&bytes[..13], b"FLV\x01\x05\0\0\0\x09\0\0\0\0");
        assert_eq!(bytes[13], 8);
        assert_eq!(&bytes[17..21], &[0, 0, 0, 0]); // timestamp
        assert_eq!(&bytes[24..28], &[0xaf, 0, 0x11, 0x90]);
        assert_eq!(&bytes[28..32], &15u32.to_be_bytes());
        assert_eq!(bytes[32], 8);
        assert_eq!(bytes[39], 0); // stream-id
    }

    #[test]
    fn avc_configuration_and_access_unit() {
        let mut mux = FlvMuxer::new(Vec::new()).unwrap();
        let sps = [0x67, 0x42, 0x00, 0x1f, 0xe5];
        let pps = [0x68, 0xce, 0x06, 0xe2];
        mux.avc_configuration(0, &sps, &pps).unwrap();
        mux.avc_frame(20, -1, true, &[&[0x65, 0xaa, 0xbb]]).unwrap();
        let bytes = mux.into_inner();
        assert!(bytes.windows(4).any(|s|s==[1, 0x42, 0, 0x1f]));
        assert!(bytes.windows(7).any(|s|s==[0x17, 1, 0xff, 0xff, 0xff, 0, 0]));
    }

    #[test]
    fn bad_inputs_rejected_without_timestamp_advance() {
        let mut mux=FlvMuxer::new(Vec::new()).unwrap();
        assert!(mux.avc_frame(0, 0, true, &[]).is_err());
        assert!(mux.aac_configuration(0, &[]).is_err());
        mux.aac_frame(33, &[1]).unwrap();
        assert!(mux.aac_frame(32, &[2]).is_err());
        assert!(mux.avc_frame(33, 8_388_608, true, &[&[0x65, 1]]).is_err());
        assert!(mux.avc_frame(33, 0, true, &[&[0,0,0,1,0x65]]).is_err());
        assert!(mux.aac_frame(34, &[]).is_err());
    }

    #[test]
    fn upper_timestamp_byte_is_stored() {
        let mut mux=FlvMuxer::new(Vec::new()).unwrap();
        mux.aac_frame(0x12345678, &[0]).unwrap();
        let bytes=mux.into_inner();
        assert_eq!(&bytes[17..21], &[0x34, 0x56, 0x78, 0x12]);
    }
}
