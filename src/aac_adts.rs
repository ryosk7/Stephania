//! AAC-LC ADTS framing for feeding raw AAC access units to RTMP.
use std::io::{self, Read};

const RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050,
    16000, 12000, 11025, 8000, 7350,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AacConfig {
    rate_index: u8,
    channels: u8,
}

impl AacConfig {
    pub fn sample_rate(self) -> u32 {
        RATES[usize::from(self.rate_index)]
    }

    pub fn channels(self) -> u8 {
        self.channels
    }

    pub fn audio_specific_config(self) -> [u8; 2] {
        // Audio object type 2 (AAC-LC), frequency index, channel configuration.
        [0x10 | (self.rate_index >> 1),
         ((self.rate_index & 1) << 7) | (self.channels << 3)]
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct AacAccessUnit {
    pub config: AacConfig,
    pub pts_samples: u64,
    pub raw: Vec<u8>,
}

impl AacAccessUnit {
    pub fn timestamp_ms(&self) -> io::Result<u32> {
        u32::try_from(u128::from(self.pts_samples) * 1000 /
            u128::from(self.config.sample_rate()))
            .map_err(|_| invalid("AAC RTMP timestamp overflow"))
    }
}

/// Reads one AAC-LC access unit at a time. A clean EOF is distinct from a
/// truncated frame. Configuration changes require a new reader/session.
/// Only mono/stereo, one raw block per ADTS frame, and no CRC are supported.
pub struct AdtsReader<R> {
    input: R,
    config: Option<AacConfig>,
    next_pts: u64,
}

impl<R: Read> AdtsReader<R> {
    pub fn new(input: R) -> Self {
        Self { input, config: None, next_pts: 0 }
    }

    pub fn read_frame(&mut self) -> io::Result<Option<AacAccessUnit>> {
        let mut h = [0u8; 7];
        loop {
            match self.input.read(&mut h[..1]) {
                Ok(0) => return Ok(None),
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        self.input.read_exact(&mut h[1..])?;
        if h[0] != 0xff || h[1] & 0xf6 != 0xf0 || h[1] & 1 == 0 {
            return Err(invalid("invalid ADTS sync/layer or unsupported CRC"));
        }
        let rate_index = (h[2] >> 2) & 0x0f;
        let channels = ((h[2] & 1) << 2) | (h[3] >> 6);
        if h[2] >> 6 != 1 || usize::from(rate_index) >= RATES.len()
            || !(1..=2).contains(&channels) || h[6] & 3 != 0 {
            return Err(invalid("unsupported ADTS profile, rate, channels or blocks"));
        }
        let config = AacConfig { rate_index, channels };
        if self.config.is_some_and(|previous| previous != config) {
            return Err(invalid("AAC configuration changed"));
        }
        let length = (usize::from(h[3] & 3) << 11)
            | (usize::from(h[4]) << 3) | usize::from(h[5] >> 5);
        if length <= h.len() {
            return Err(invalid("empty or invalid ADTS payload"));
        }
        let mut raw = vec![0; length - h.len()];
        self.input.read_exact(&mut raw)?;
        let pts_samples = self.next_pts;
        self.next_pts = self.next_pts.checked_add(1024)
            .ok_or_else(|| invalid("AAC sample clock overflow"))?;
        self.config = Some(config);
        Ok(Some(AacAccessUnit { config, pts_samples, raw }))
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(rate_index: u8, raw: &[u8]) -> Vec<u8> {
        let n = raw.len() + 7;
        let mut bytes = vec![0xff, 0xf1, 0x40 | (rate_index << 2),
            0x80 | ((n >> 11) as u8), (n >> 3) as u8,
            ((n & 7) << 5) as u8 | 0x1f, 0xfc];
        bytes.extend_from_slice(raw);
        bytes
    }

    #[test]
    fn strips_headers_and_preserves_sample_clock() {
        let bytes = frame(3, &[1, 2, 3]).repeat(100);
        let mut reader = AdtsReader::new(bytes.as_slice());
        for i in 0..100u64 {
            let unit = reader.read_frame().unwrap().unwrap();
            assert_eq!(unit.raw, [1, 2, 3]);
            assert_eq!(unit.config.audio_specific_config(), [0x11, 0x90]);
            assert_eq!(unit.config.sample_rate(), 48000);
            assert_eq!(unit.pts_samples, i * 1024);
            assert_eq!(unit.timestamp_ms().unwrap(), (i * 1024 / 48) as u32);
        }
        assert!(reader.read_frame().unwrap().is_none());
    }

    #[test]
    fn rejects_truncated_frames() {
        let bytes = frame(3, &[1, 2, 3]);
        for end in 1..bytes.len() {
            let err = AdtsReader::new(&bytes[..end]).read_frame().unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        }
    }

    #[test]
    fn rejects_unsupported_or_changed_configurations() {
        let mut bytes = frame(3, &[1]);
        bytes.extend(frame(4, &[2]));
        let mut reader = AdtsReader::new(bytes.as_slice());
        reader.read_frame().unwrap();
        assert!(reader.read_frame().is_err());
        for (index, mask) in [(0, 1), (1, 1), (2, 0x40), (6, 1)] {
            let mut bytes = frame(3, &[1]);
            bytes[index] ^= mask;
            assert!(AdtsReader::new(bytes.as_slice()).read_frame().is_err());
        }
        assert!(AdtsReader::new(frame(3, &[]).as_slice()).read_frame().is_err());
    }
}
