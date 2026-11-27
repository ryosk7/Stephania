//! Program audio mix (docs/AUDIO.md): up to two PCM inputs (HDMI audio and
//! a USB microphone) on the publisher's monotonic 48 kHz sample timeline.
//!
//! - Each input has its own `AudioClock` drift/discontinuity correction and
//!   a bounded queue (8192 frames); a missing or stopped input contributes
//!   silence, so program audio and its PTS stay continuous.
//! - Gain and mute changes are ramped across one block (no clicks).
//! - A soft limiter keeps the sum below −1 dBFS: instant attack within the
//!   block, gradual release, so overload does not pump block to block.
//! - When the publisher falls behind (blocked network write), due blocks are
//!   produced to catch up; beyond `RESYNC_SAMPLES` the timeline jumps forward
//!   instead of replaying stale audio, and the session keeps running.
use crate::{av_clock::AudioClock, media_core::AudioFrame, pcm_framer::PcmBlock};
use std::{collections::VecDeque, io};

const RATE: u64 = 48000;
pub const BLOCK: usize = 1024;
const CAPACITY: usize = 8192;
/// Session timelines start at +100 ms (matches the video PTS offset).
const OFFSET: u64 = 4800;
/// Behind by more than one second: resynchronise rather than catch up.
pub const RESYNC_SAMPLES: u64 = RATE;
/// Upper bound on blocks produced per poll while catching up.
const MAX_BLOCKS_PER_POLL: usize = 48;
const CEILING: f64 = 29204.0; // ≈ −1 dBFS
/// Limiter release per block (≈ +0.5 dB per 21 ms).
const RELEASE: f64 = 1.059;

fn sample_time(us: u64) -> u64 {
    (u128::from(us) * u128::from(RATE) / 1_000_000) as u64
}

struct Input {
    clock: AudioClock,
    queue: VecDeque<[i16; 2]>,
    start: u64,
    /// Linear gain applied at the end of the previous block.
    gain: f64,
}

impl Input {
    fn new() -> Self {
        Self { clock: AudioClock::default(), queue: VecDeque::with_capacity(CAPACITY), start: 0, gain: 0.0 }
    }

    fn reset(&mut self) {
        self.clock = AudioClock::default();
        self.queue.clear();
    }

    fn ingest(&mut self, frame: &AudioFrame) -> io::Result<()> {
        if frame.sample_rate != 48000 || !matches!(frame.channels, 1 | 2)
            || !frame.samples.len().is_multiple_of(usize::from(frame.channels)) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid mix input"));
        }
        let stereo = if frame.channels == 1 {
            AudioFrame { channels: 2, samples: frame.samples.iter().flat_map(|&x| [x, x]).collect(), ..frame.clone() }
        } else {
            frame.clone()
        };
        // One-frame short reads cannot be passed to the existing interpolator.
        if stereo.samples.len() < 4 { return Ok(()); }
        let corrected = match self.clock.correct(&stereo) {
            Ok(f) => f,
            Err(_) => { self.reset(); self.clock.correct(&stereo)? }
        };
        let start = sample_time(corrected.timestamp_us);
        if self.queue.is_empty() || self.start + self.queue.len() as u64 != start {
            self.queue.clear();
            self.start = start;
        }
        for pair in corrected.samples.as_chunks::<2>().0 {
            if self.queue.len() == CAPACITY {
                self.queue.pop_front();
                self.start += 1;
            }
            self.queue.push_back(*pair);
        }
        Ok(())
    }

    fn at(&mut self, pts: u64) -> [i16; 2] {
        while !self.queue.is_empty() && self.start < pts {
            self.queue.pop_front();
            self.start += 1;
        }
        if self.start == pts {
            if let Some(pair) = self.queue.pop_front() {
                self.start += 1;
                return pair;
            }
        }
        [0, 0]
    }
}

/// Levels of one block for metering: (peak, sum of squares, samples).
pub type Level = (f64, f64, usize);

pub struct MixedBlock {
    pub block: PcmBlock,
    /// Post-gain levels per input (before the limiter).
    pub inputs: [Level; 2],
    /// Program output after the limiter.
    pub master: Level,
    /// The unlimited sum exceeded full scale.
    pub clipped: bool,
    /// The limiter reduced gain in this block.
    pub limited: bool,
}

pub struct AudioMixer {
    inputs: [Input; 2],
    next: u64,
    delay: u64,
    limiter: f64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Poll {
    pub resynced: bool,
}

impl AudioMixer {
    pub fn new(latency_ms: u32) -> Self {
        Self { inputs: [Input::new(), Input::new()], next: OFFSET, delay: u64::from(latency_ms) * 48, limiter: 1.0 }
    }

    pub fn reset_input(&mut self, input: usize) { self.inputs[input].reset(); }

    pub fn ingest(&mut self, input: usize, frame: &AudioFrame) -> io::Result<()> {
        self.inputs[input].ingest(frame)
    }

    /// Produces every block due at `now_us` (session-relative, without the
    /// PTS offset), mixing with `targets` (linear gain per input; 0 = muted
    /// or disabled), ramped from the previous gains.
    pub fn poll(&mut self, now_us: u64, targets: [f64; 2], out: &mut Vec<MixedBlock>) -> Poll {
        let now = sample_time(now_us).saturating_add(OFFSET);
        let mut result = Poll::default();
        let delay = self.delay;
        let due = move |next: u64| next + BLOCK as u64 + delay;
        if now > due(self.next) + RESYNC_SAMPLES {
            // Far behind: skip to the present (whole blocks), keep running.
            let behind = now - due(self.next);
            self.next += behind / BLOCK as u64 * BLOCK as u64;
            result.resynced = true;
        }
        let mut produced = 0;
        while now >= due(self.next) && produced < MAX_BLOCKS_PER_POLL {
            out.push(self.mix_block(targets));
            produced += 1;
        }
        result
    }

    fn mix_block(&mut self, targets: [f64; 2]) -> MixedBlock {
        let mut sum = [0f64; BLOCK * 2];
        let mut inputs = [(0.0, 0.0, 0usize); 2];
        let start_gains = [self.inputs[0].gain, self.inputs[1].gain];
        for (k, input) in self.inputs.iter_mut().enumerate() {
            let (g0, g1) = (start_gains[k], targets[k]);
            let (mut peak, mut sq) = (0f64, 0f64);
            for (i, pair) in sum.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let s = input.at(self.next + i as u64);
                let g = g0 + (g1 - g0) * (i as f64 + 1.0) / BLOCK as f64;
                for ch in 0..2 {
                    let v = f64::from(s[ch]) * g;
                    pair[ch] += v;
                    peak = peak.max(v.abs());
                    sq += v * v;
                }
            }
            input.gain = g1;
            inputs[k] = (peak, sq, BLOCK * 2);
        }
        let raw_peak = sum.iter().fold(0f64, |m, &v| m.max(v.abs()));
        let wanted = if raw_peak > CEILING { CEILING / raw_peak } else { 1.0 };
        let start = self.limiter;
        // Attack: reach `wanted` within this block; release: slowly toward 1.
        let end = if wanted < start { wanted } else { (start * RELEASE).min(wanted).min(1.0) };
        let mut interleaved = Vec::with_capacity(BLOCK * 2);
        let (mut mpeak, mut msq) = (0f64, 0f64);
        for (i, pair) in sum.as_chunks::<2>().0.iter().enumerate() {
            // Attack applies from the first sample so no sample exceeds the ceiling.
            let g = if wanted < start { wanted } else { start + (end - start) * (i as f64 + 1.0) / BLOCK as f64 };
            for &v in pair {
                let y = (v * g).clamp(-32768.0, 32767.0);
                mpeak = mpeak.max(y.abs());
                msq += y * y;
                interleaved.push(y.round() as i16);
            }
        }
        self.limiter = end;
        let pts_samples = self.next;
        self.next += BLOCK as u64;
        MixedBlock {
            block: PcmBlock { pts_samples, sample_rate: 48000, channels: 2, interleaved },
            inputs,
            master: (mpeak, msq, BLOCK * 2),
            clipped: raw_peak > 32767.0,
            limited: wanted < 1.0 || start < 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(channels: u8, value: i16) -> AudioFrame {
        AudioFrame { timestamp_us: 100000, sample_rate: 48000, channels, samples: vec![value; BLOCK * usize::from(channels)] }
    }

    fn poll(m: &mut AudioMixer, now: u64, g: [f64; 2]) -> (Poll, Vec<MixedBlock>) {
        let mut out = Vec::new();
        let p = m.poll(now, g, &mut out);
        (p, out)
    }

    #[test]
    fn mixes_mono_and_stereo_with_continuous_pts() {
        let mut m = AudioMixer::new(40);
        m.ingest(0, &frame(2, 1000)).unwrap();
        m.ingest(1, &frame(1, 2000)).unwrap();
        // Prime gains so this block is not a ramp.
        m.inputs[0].gain = 0.5;
        m.inputs[1].gain = 0.5;
        assert!(poll(&mut m, 60000, [0.5, 0.5]).1.is_empty());
        let (_, a) = poll(&mut m, 62000, [0.5, 0.5]);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].block.pts_samples, 4800);
        assert!(a[0].block.interleaved.iter().all(|&x| x == 1500));
        m.reset_input(1);
        let (_, b) = poll(&mut m, 84000, [0.5, 0.5]);
        assert_eq!(b[0].block.pts_samples, 4800 + 1024);
        assert!(b[0].block.interleaved.iter().all(|&x| x == 0), "missing input is silence");
    }

    #[test]
    fn single_input_at_unity_is_bit_exact() {
        let mut m = AudioMixer::new(40);
        let samples: Vec<i16> = (0..BLOCK as i16 * 2).map(|i| (i % 109 - 54) * 200).collect();
        m.ingest(0, &AudioFrame { timestamp_us: 100000, sample_rate: 48000, channels: 2, samples: samples.clone() }).unwrap();
        m.inputs[0].gain = 1.0;
        let (_, out) = poll(&mut m, 62000, [1.0, 0.0]);
        assert_eq!(out[0].block.interleaved, samples);
        assert!(!out[0].limited);
    }

    #[test]
    fn gain_changes_are_ramped() {
        let mut m = AudioMixer::new(40);
        m.ingest(0, &frame(2, 10000)).unwrap();
        m.inputs[0].gain = 1.0;
        let (_, out) = poll(&mut m, 62000, [0.0, 0.0]);
        let s = &out[0].block.interleaved;
        assert!(s[0] > 9900 && s[s.len() - 1] == 0, "ramp from unity to mute across the block");
        assert!(s.windows(2).all(|w| w[1] <= w[0]), "monotonic, no step");
    }

    #[test]
    fn limiter_holds_ceiling_without_clipping_and_releases_gradually() {
        let mut m = AudioMixer::new(40);
        let loud = AudioFrame { timestamp_us: 100000, sample_rate: 48000, channels: 2, samples: vec![32767; BLOCK * 2 * 3] };
        m.ingest(0, &loud).unwrap();
        m.ingest(1, &AudioFrame { channels: 1, samples: vec![32767; BLOCK * 3], ..loud.clone() }).unwrap();
        m.inputs[0].gain = 1.0;
        m.inputs[1].gain = 1.0;
        let (_, out) = poll(&mut m, 62000, [1.0, 1.0]);
        let b = &out[0];
        assert!(b.clipped && b.limited);
        assert!(b.block.interleaved.iter().all(|&x| f64::from(x) <= CEILING + 1.0), "never above the ceiling");
        // Quiet again: limiter releases over several blocks, not at once.
        let mut quiet = AudioMixer::new(40);
        quiet.limiter = 0.5;
        quiet.ingest(0, &frame(2, 1000)).unwrap();
        quiet.inputs[0].gain = 1.0;
        let (_, q) = poll(&mut quiet, 62000, [1.0, 0.0]);
        assert!(q[0].block.interleaved[BLOCK * 2 - 1] < 1000, "release is gradual");
        assert!(quiet.limiter > 0.5 && quiet.limiter < 0.6);
    }

    #[test]
    fn catches_up_then_resynchronises_instead_of_failing() {
        let mut m = AudioMixer::new(40);
        // 0.5 s behind: produce the backlog (bounded per poll), no error.
        let (p, out) = poll(&mut m, 500_000 + 62000, [1.0, 0.0]);
        assert!(!p.resynced);
        assert!(out.len() > 20 && out.len() <= MAX_BLOCKS_PER_POLL);
        assert!(out.windows(2).all(|w| w[1].block.pts_samples == w[0].block.pts_samples + BLOCK as u64));
        // 5 s stall: jump forward, PTS stays block-aligned and increasing.
        let last = out.last().unwrap().block.pts_samples;
        let (p, out) = poll(&mut m, 5_500_000, [1.0, 0.0]);
        assert!(p.resynced);
        assert!(out[0].block.pts_samples > last + RESYNC_SAMPLES);
        assert_eq!((out[0].block.pts_samples - 4800) % BLOCK as u64, 0);
    }
}
