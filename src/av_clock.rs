//! Shared monotonic capture timeline and bounded PCM rate correction.
//! Arrival timestamps are noisy proxies for hardware timestamps; this is not
//! a calibrated hardware clock estimator.
use crate::media_core::AudioFrame;
use std::io;

const RATE: u64 = 48000;
const DEAD_BAND: i128 = 480; // 10 ms
const MAX_GAP: i128 = 12000; // 250 ms

#[derive(Default)]
pub struct AudioClock {
    next_sample: Option<u64>,
    correction_credit: usize,
    pub inserted: u64,
    pub removed: u64,
}

impl AudioClock {
    /// Input timestamp is the beginning of the PCM period, relative to the
    /// same session origin as video. Smooth small clock errors by interpolating
    /// at most one sample frame per 1000 input frames (1000 ppm).
    pub fn correct(&mut self, frame: &AudioFrame) -> io::Result<AudioFrame> {
        let n = frame.samples.len() / 2;
        if frame.sample_rate != RATE as u32 || frame.channels != 2 ||
            !(2..=8192).contains(&n) || frame.samples.len() != n * 2 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid stereo PCM period"));
        }
        let observed = u64::try_from(u128::from(frame.timestamp_us) * u128::from(RATE) / 1_000_000)
            .map_err(|_| io::Error::other("audio clock overflow"))?;
        let next = self.next_sample.unwrap_or(observed);
        let error = i128::from(observed) - i128::from(next);
        if error.abs() > MAX_GAP {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "audio capture discontinuity"));
        }
        self.correction_credit += n;
        let allowance = self.correction_credit / 1000;
        self.correction_credit %= 1000;
        let delta = if error > DEAD_BAND { allowance as isize }
            else if error < -DEAD_BAND { -(allowance as isize) } else { 0 };
        let count = n.checked_add_signed(delta).ok_or_else(|| io::Error::other("PCM size overflow"))?;
        let samples = if delta == 0 { frame.samples.clone() } else {
            let mut out = Vec::with_capacity(count * 2);
            for i in 0..count {
                let position = i * (n - 1);
                let index = position / (count - 1);
                let remainder = position % (count - 1);
                let right = (index + 1).min(n - 1);
                for ch in 0..2 {
                    let a = i64::from(frame.samples[index * 2 + ch]);
                    let b = i64::from(frame.samples[right * 2 + ch]);
                    out.push((a + (b - a) * remainder as i64 / (count - 1) as i64) as i16);
                }
            }
            out
        };
        if delta > 0 { self.inserted += delta as u64; }
        if delta < 0 { self.removed += (-delta) as u64; }
        self.next_sample = Some(next.checked_add(count as u64).ok_or_else(|| io::Error::other("audio clock overflow"))?);
        // Round UP so PCM framer's conversion back to samples is exact.
        let timestamp_us = u64::try_from((u128::from(next) * 1_000_000).div_ceil(u128::from(RATE)))
            .map_err(|_| io::Error::other("audio timestamp overflow"))?;
        Ok(AudioFrame { timestamp_us, sample_rate: frame.sample_rate, channels: 2, samples })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrects_fast_and_slow_clocks_without_timestamp_jumps() {
        for ppm in [-500i64, 500] {
            let mut clock = AudioClock::default();
            let mut expected = 0;
            for i in 0..6000i64 {
                let ts = i * 1000 * (1_000_000 + ppm) / 48000;
                let frame = AudioFrame { timestamp_us: ts as u64, sample_rate: 48000,
                    channels: 2, samples: vec![0; 2000] };
                let corrected = clock.correct(&frame).unwrap();
                assert_eq!(corrected.timestamp_us * RATE / 1_000_000, expected);
                expected += (corrected.samples.len() / 2) as u64;
            }
            if ppm > 0 { assert!(clock.inserted > 0); } else { assert!(clock.removed > 0); }
            let target = 6_000_000i64 * (1_000_000 + ppm) / 1_000_000;
            assert!((expected as i64 - target).abs() < 600);
        }
    }
    #[test]
    fn rejects_large_gaps() {
        let mut clock = AudioClock::default();
        let mut f = AudioFrame { timestamp_us: 0, sample_rate: 48000, channels: 2, samples: vec![0; 2048] };
        clock.correct(&f).unwrap();
        f.timestamp_us = 1_000_000;
        assert!(clock.correct(&f).is_err());
    }
}
