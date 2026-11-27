//! Two independent PCM inputs on the publisher's monotonic sample timeline.
use crate::{av_clock::AudioClock, media_core::AudioFrame, pcm_framer::PcmBlock};
use std::{collections::VecDeque, io};

const RATE: u64 = 48000;
const BLOCK: usize = 1024;
const CAPACITY: usize = 8192;
const OFFSET: u64 = 4800;

fn sample_time(us: u64) -> u64 {
    (u128::from(us) * u128::from(RATE) / 1_000_000) as u64
}

struct Input {
    clock: AudioClock,
    queue: VecDeque<[i16; 2]>,
    start: u64,
    gain: i32,
}
impl Input {
    fn new(gain: i32) -> Self {
        Self { clock: AudioClock::default(), queue: VecDeque::with_capacity(CAPACITY),
            start: 0, gain }
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
            AudioFrame { channels: 2, samples: frame.samples.iter().flat_map(|&x| [x,x]).collect(),
                ..frame.clone() }
        } else { frame.clone() };
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
            self.queue.push_back([pair[0],pair[1]]);
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
        [0,0]
    }
}

pub struct AudioMixer {
    inputs: [Input; 2],
    next: u64,
    delay: u64,
}
impl AudioMixer {
    pub fn new(gains: [i32; 2], latency_ms: u32) -> Self {
        Self { inputs: gains.map(Input::new), next: OFFSET,
            delay: u64::from(latency_ms) * 48 }
    }
    pub fn reset_input(&mut self, input: usize) { self.inputs[input].reset(); }
    pub fn ingest(&mut self, input: usize, frame: &AudioFrame) -> io::Result<()> {
        self.inputs[input].ingest(frame)
    }
    /// now_us is relative to the current publishing session, without PTS offset.
    pub fn poll(&mut self, now_us: u64) -> io::Result<Option<PcmBlock>> {
        let now = sample_time(now_us).saturating_add(OFFSET);
        let due = self.next + BLOCK as u64 + self.delay;
        if now < due { return Ok(None); }
        if now - due > RATE / 2 {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "audio mixer deadline exceeded"));
        }
        let mut wide = [0i64; BLOCK * 2];
        let mut peak = 1i64;
        for (i, pair) in wide.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let p = self.next + i as u64;
            let a = self.inputs[0].at(p);
            let b = self.inputs[1].at(p);
            for ch in 0..2 {
                pair[ch] = (i64::from(a[ch]) * i64::from(self.inputs[0].gain)
                    + i64::from(b[ch]) * i64::from(self.inputs[1].gain)) >> 15;
                peak = peak.max(pair[ch].abs());
            }
        }
        let ceiling = 29204i64; // approximately -1 dBFS
        let interleaved = wide.iter().map(|&x| {
            if peak > ceiling { (x * ceiling / peak) as i16 } else { x as i16 }
        }).collect();
        let pts_samples = self.next;
        self.next += BLOCK as u64;
        Ok(Some(PcmBlock { pts_samples, sample_rate: 48000, channels: 2, interleaved }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(channels: u8, value: i16) -> AudioFrame {
        AudioFrame { timestamp_us: 100000, sample_rate: 48000, channels,
            samples: vec![value; BLOCK * usize::from(channels)] }
    }
    #[test]
    fn mixes_mono_and_stereo_and_preserves_pts_through_loss() {
        let mut m = AudioMixer::new([16384,16384], 40);
        m.ingest(0,&frame(2,1000)).unwrap();
        m.ingest(1,&frame(1,2000)).unwrap();
        assert!(m.poll(60000).unwrap().is_none());
        let a = m.poll(62000).unwrap().unwrap();
        assert_eq!(a.pts_samples,4800);
        assert!(a.interleaved.iter().all(|&x| x == 1500));
        m.reset_input(1);
        let b = m.poll(84000).unwrap().unwrap();
        assert_eq!(b.pts_samples,a.pts_samples+1024);
        assert!(b.interleaved.iter().all(|&x| x == 0));
    }
    #[test]
    fn limits_full_scale_mix() {
        let mut m = AudioMixer::new([32768,32768],40);
        m.ingest(0,&frame(2,32767)).unwrap();
        m.ingest(1,&frame(1,32767)).unwrap();
        assert!(m.poll(62000).unwrap().unwrap().interleaved.iter().all(|&x| x == 29204));
    }
    #[test]
    fn rejects_stale_output_deadline() {
        assert!(AudioMixer::new([0,0],40).poll(1_000_000).is_err());
    }
}
