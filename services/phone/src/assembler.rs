//! RTP (RFC 6184, H.264) → Annex-B access units.
//!
//! Packets of one access unit share an RTP timestamp; the marker bit ends
//! it. Any sequence gap (loss or reordering beyond what the NACK
//! interceptor repaired) breaks the reference chain, so after a gap nothing
//! is forwarded until the next keyframe (IDR or SPS) and the caller is told
//! to request one. This trades a short freeze for never showing smeared
//! garbage on the program output.
use bytes::Bytes;
use rtc::rtp::codec::h264::H264Packet;
use rtc::rtp::packetizer::Depacketizer;

/// Largest access unit forwarded (matches the engine's bound).
pub const MAX_ACCESS_UNIT: usize = 2 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Output {
    /// Complete access unit: RTP timestamp (90 kHz) and Annex-B bytes.
    AccessUnit { timestamp: u32, keyframe: bool, data: Vec<u8> },
    /// Loss detected (or oversize): request a keyframe.
    NeedKeyframe,
}

pub struct Assembler {
    depacketizer: H264Packet,
    buffer: Vec<u8>,
    timestamp: Option<u32>,
    last_seq: Option<u16>,
    broken: bool,
    /// Waiting for a keyframe since start or since the last loss.
    awaiting_keyframe: bool,
}

impl Default for Assembler {
    fn default() -> Self {
        Self { depacketizer: H264Packet::default(), buffer: Vec::new(), timestamp: None, last_seq: None, broken: false, awaiting_keyframe: true }
    }
}

/// True when the Annex-B data contains an IDR slice or an SPS.
pub fn is_keyframe(annex_b: &[u8]) -> bool {
    let mut i = 0;
    while i + 3 < annex_b.len() {
        if annex_b[i] == 0 && annex_b[i + 1] == 0 && annex_b[i + 2] == 1 {
            let t = annex_b[i + 3] & 0x1f;
            if t == 5 || t == 7 { return true; }
            i += 3;
        } else {
            i += 1;
        }
    }
    false
}

impl Assembler {
    pub fn push(&mut self, sequence: u16, timestamp: u32, marker: bool, payload: &Bytes) -> Vec<Output> {
        let mut out = Vec::new();
        if let Some(last) = self.last_seq {
            let expected = last.wrapping_add(1);
            if sequence != expected {
                if sequence.wrapping_sub(expected) > 0x8000 {
                    return out; // late duplicate or old reordered packet: ignore
                }
                self.lose(&mut out);
            }
        }
        self.last_seq = Some(sequence);
        if self.timestamp.is_some_and(|t| t != timestamp) {
            // New timestamp without a marker on the previous AU: it was
            // incomplete (marker packet lost).
            self.lose(&mut out);
        }
        self.timestamp = Some(timestamp);
        if !self.broken {
            match self.depacketizer.depacketize(payload) {
                Ok(nal) => {
                    self.buffer.extend_from_slice(&nal);
                    if self.buffer.len() > MAX_ACCESS_UNIT { self.lose(&mut out); }
                }
                Err(_) => self.lose(&mut out),
            }
        }
        if marker {
            if !self.broken && !self.buffer.is_empty() {
                let data = std::mem::take(&mut self.buffer);
                let keyframe = is_keyframe(&data);
                if keyframe { self.awaiting_keyframe = false; }
                if !self.awaiting_keyframe {
                    out.push(Output::AccessUnit { timestamp, keyframe, data });
                }
            }
            self.buffer.clear();
            self.broken = false;
            self.timestamp = None;
        }
        out
    }

    fn lose(&mut self, out: &mut Vec<Output>) {
        self.buffer.clear();
        self.depacketizer = H264Packet::default();
        self.broken = true;
        if !out.contains(&Output::NeedKeyframe) { out.push(Output::NeedKeyframe); }
        self.awaiting_keyframe = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS: &[u8] = &[0x67, 0x42, 0xc0, 0x1f, 0xda];
    const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00, 0x11];
    const P: &[u8] = &[0x41, 0x9a, 0x22, 0x33];

    fn b(x: &[u8]) -> Bytes { Bytes::copy_from_slice(x) }

    /// FU-A fragments of one NAL (RFC 6184 §5.8).
    fn fu_a(nal: &[u8], parts: usize) -> Vec<Bytes> {
        let (hdr, body) = (nal[0], &nal[1..]);
        let chunk = body.len().div_ceil(parts);
        body.chunks(chunk).enumerate().map(|(i, c)| {
            let mut fu_header = hdr & 0x1f;
            if i == 0 { fu_header |= 0x80; }
            if (i + 1) * chunk >= body.len() { fu_header |= 0x40; }
            let mut p = vec![(hdr & 0xe0) | 28, fu_header];
            p.extend_from_slice(c);
            Bytes::from(p)
        }).collect()
    }

    fn annexb(nals: &[&[u8]]) -> Vec<u8> {
        nals.iter().flat_map(|n| [&[0u8, 0, 0, 1][..], n].concat()).collect()
    }

    #[test]
    fn assembles_single_and_fragmented_nals_into_access_units() {
        let mut a = Assembler::default();
        assert!(a.push(1, 1000, false, &b(SPS)).is_empty());
        assert!(a.push(2, 1000, false, &b(PPS)).is_empty());
        let frags = fu_a(IDR, 2);
        assert!(a.push(3, 1000, false, &frags[0]).is_empty());
        let out = a.push(4, 1000, true, &frags[1]);
        assert_eq!(out, vec![Output::AccessUnit { timestamp: 1000, keyframe: true, data: annexb(&[SPS, PPS, IDR]) }]);
        let out = a.push(5, 4000, true, &b(P));
        assert_eq!(out, vec![Output::AccessUnit { timestamp: 4000, keyframe: false, data: annexb(&[P]) }]);
    }

    #[test]
    fn nothing_before_the_first_keyframe() {
        let mut a = Assembler::default();
        assert!(a.push(10, 0, true, &b(P)).is_empty());
        assert!(matches!(a.push(11, 3000, true, &b(IDR))[..], [Output::AccessUnit { keyframe: true, .. }]));
    }

    #[test]
    fn loss_requests_a_keyframe_and_suppresses_until_one_arrives() {
        let mut a = Assembler::default();
        a.push(1, 0, true, &b(IDR));
        let frags = fu_a(P, 3);
        a.push(2, 3000, false, &frags[0]);
        // seq 3 lost
        let out = a.push(4, 3000, true, &frags[2]);
        assert_eq!(out, vec![Output::NeedKeyframe]);
        assert!(a.push(5, 6000, true, &b(P)).is_empty(), "P after loss is not forwarded");
        assert!(matches!(a.push(6, 9000, true, &b(IDR))[..], [Output::AccessUnit { keyframe: true, .. }]));
        assert!(matches!(a.push(7, 12000, true, &b(P))[..], [Output::AccessUnit { keyframe: false, .. }]));
    }

    #[test]
    fn lost_marker_packet_is_detected_by_timestamp_change() {
        let mut a = Assembler::default();
        a.push(1, 0, true, &b(IDR));
        a.push(2, 3000, false, &b(P)); // marker packet of this AU lost...
        let out = a.push(3, 6000, true, &b(P)); // ...but seq is contiguous
        assert_eq!(out, vec![Output::NeedKeyframe]);
    }

    #[test]
    fn duplicates_and_late_packets_are_ignored_and_seq_wraps() {
        let mut a = Assembler::default();
        a.push(65535, 0, true, &b(IDR));
        assert!(matches!(a.push(0, 3000, true, &b(P))[..], [Output::AccessUnit { .. }]), "sequence wraps");
        assert!(a.push(0, 3000, true, &b(P)).is_empty(), "duplicate");
        assert!(a.push(65530, 0, true, &b(P)).is_empty(), "late");
        assert!(matches!(a.push(1, 6000, true, &b(P))[..], [Output::AccessUnit { .. }]));
    }

    #[test]
    fn keyframe_detection() {
        assert!(is_keyframe(&annexb(&[SPS, PPS, IDR])));
        assert!(!is_keyframe(&annexb(&[P])));
    }
}
