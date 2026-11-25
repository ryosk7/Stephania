//! Frame and meter ALSA PCM before handing it to a software AAC encoder.
//!
//! This module is independent of the actual AAC implementation. It preserves
//! sample-exact 1024-frame boundaries and reports discontinuities rather than
//! inventing or silently dropping captured audio samples.
use crate::media_core::{measure_audio, AudioFrame, AudioLevel};
use std::collections::VecDeque;
use std::io;

pub const AAC_LC_FRAME_SAMPLES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PcmBlock {
    pub pts_samples: u64,
    pub sample_rate: u32,
    pub channels: u8,
    pub interleaved: Vec<i16>,
}
impl PcmBlock {
    pub fn timestamp_us(&self) -> u64 {
        ((u128::from(self.pts_samples) * 1_000_000) / u128::from(self.sample_rate)) as u64
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioStats {
    pub frames_in: u64,
    pub blocks_out: u64,
    pub dropped_or_missing_samples: u64,
    pub last_level: AudioLevel,
}

pub struct PcmFramer {
    sample_rate: u32,
    channels: u8,
    samples: VecDeque<i16>,
    pending_pts_samples: Option<u64>,
    next_input_sample: Option<u64>,
    frames_in: u64,
    blocks_out: u64,
    discontinuities: u64,
    last_level: AudioLevel,
}

impl PcmFramer {
    pub fn new(sample_rate: u32, channels: u8) -> io::Result<Self> {
        if !(8_000..=192_000).contains(&sample_rate) || channels == 0 || channels > 8 {
            return Err(invalid("invalid PCM sampling configuration"));
        }
        Ok(Self {
            sample_rate, channels, samples: VecDeque::new(),
            pending_pts_samples: None, next_input_sample: None,
            frames_in: 0, blocks_out: 0, discontinuities: 0,
            last_level: measure_audio(&[]),
        })
    }

    /// Frames must use the negotiated sample rate/channel layout. When a source
    /// reconnects or a hole is detected, the partial AAC block is discarded
    /// rather than splicing unrelated timestamps into one encoded packet.
    pub fn push(&mut self, frame: &AudioFrame) -> io::Result<Vec<PcmBlock>> {
        if frame.sample_rate != self.sample_rate || frame.channels != self.channels ||
            frame.samples.is_empty() || !frame.samples.len().is_multiple_of(usize::from(self.channels)) {
            return Err(invalid("invalid PCM frame format or length"));
        }
        let clock = u64::try_from(
            (u128::from(frame.timestamp_us) * u128::from(self.sample_rate)) / 1_000_000
        ).map_err(|_| invalid("PCM timestamp overflow"))?;
        let sample_frames = (frame.samples.len() / usize::from(self.channels)) as u64;
        if let Some(expected) = self.next_input_sample {
            // Allow 3ms of capture-arrival timestamp jitter. True drift must
            // be corrected by a clock-domain resampler, not this packetizer.
            let tolerance = u64::from(self.sample_rate).div_ceil(333);
            if expected.abs_diff(clock) > tolerance {
                self.samples.clear();
                self.pending_pts_samples = None;
                self.discontinuities += expected.abs_diff(clock);
            }
        }
        if self.pending_pts_samples.is_none() { self.pending_pts_samples = Some(clock); }
        self.next_input_sample = Some(clock.saturating_add(sample_frames));
        self.frames_in += 1;
        self.last_level = measure_audio(&frame.samples);
        self.samples.extend(frame.samples.iter().copied());
        let mut result = Vec::new();
        let block_len = AAC_LC_FRAME_SAMPLES * usize::from(self.channels);
        while self.samples.len() >= block_len {
            let mut interleaved = Vec::with_capacity(block_len);
            for _ in 0..block_len {
                interleaved.push(self.samples.pop_front().expect("length checked"));
            }
            let pts = self.pending_pts_samples.expect("set before append");
            self.pending_pts_samples = Some(pts.saturating_add(AAC_LC_FRAME_SAMPLES as u64));
            self.blocks_out += 1;
            result.push(PcmBlock {
                pts_samples: pts, sample_rate: self.sample_rate,
                channels: self.channels, interleaved,
            });
        }
        Ok(result)
    }

    pub fn stats(&self) -> AudioStats {
        AudioStats {
            frames_in: self.frames_in,
            blocks_out: self.blocks_out,
            dropped_or_missing_samples: self.discontinuities,
            last_level: self.last_level,
        }
    }
    pub fn reset(&mut self) {
        self.samples.clear();
        self.pending_pts_samples = None;
        self.next_input_sample = None;
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(ts: u64, count: usize) -> AudioFrame {
        AudioFrame { timestamp_us: ts, sample_rate: 48_000,
            channels: 2, samples: vec![123;count*2] }
    }
    #[test]
    fn assembles_exact_1024_pcm_frames_across_reads() {
        let mut f=PcmFramer::new(48_000,2).unwrap();
        assert!(f.push(&frame(0,512)).unwrap().is_empty());
        let blocks=f.push(&frame(10_666,512)).unwrap();
        assert_eq!(blocks.len(),1);
        assert_eq!(blocks[0].interleaved.len(),2048);
        assert_eq!(blocks[0].pts_samples,0);
        assert_eq!(blocks[0].timestamp_us(),0);
        let next=f.push(&frame(21_333,1024)).unwrap();
        assert_eq!(next[0].pts_samples,1024);
        assert_eq!(next[0].timestamp_us(),21_333);
        assert_eq!(f.stats().blocks_out,2);
    }
    #[test]
    fn rejects_wrong_channels_and_truncated_interleaved_samples() {
        let mut f=PcmFramer::new(48_000,2).unwrap();
        let mut bad=frame(0,1024);
        bad.channels=1;
        assert!(f.push(&bad).is_err());
        bad.channels=2;
        bad.samples.pop();
        assert!(f.push(&bad).is_err());
    }
    #[test]
    fn discontinuity_resets_partial_block() {
        let mut f=PcmFramer::new(48_000,2).unwrap();
        f.push(&frame(0,300)).unwrap();
        assert!(f.push(&frame(2_000_000,300)).unwrap().is_empty());
        assert!(f.stats().dropped_or_missing_samples > 0);
        let output=f.push(&frame(2_006_250,724)).unwrap();
        assert_eq!(output.len(),1);
        assert!(output[0].pts_samples>=96_000);
    }
    #[test]
    fn rejects_invalid_sampling_setup() {
        assert!(PcmFramer::new(0,2).is_err());
        assert!(PcmFramer::new(48_000,0).is_err());
    }
}
