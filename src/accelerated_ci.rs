//! Event-count simulation: no sleeps, no full-resolution pixel allocations.
//! Exercises actual audio clock arithmetic, queue implementation and retry
//! policy. Network/device availability and service rate are modeled inputs.
use crate::{av_clock::AudioClock, media_core::{BoundedVideoQueue, VideoFrame, PixelFormat, AudioFrame}, reconnect};
use std::{io, time::Instant};

const SECONDS: u64 = 10800;
const PERIOD: u64 = 1024;
const RATE: u64 = 48000;
const VIDEO_FRAMES: u64 = SECONDS * 30;

#[test]
fn accelerated_three_hour_clock_and_framing() {
    let start = Instant::now();
    let periods = SECONDS * RATE / PERIOD;
    for ppm in [-900i64, -500, 0, 500, 900] {
        let mut clock = AudioClock::default();
        let mut emitted = 0u64;
        let mut previous = 0u64;
        for i in 0..periods {
            let observed = (i as i64 * PERIOD as i64 * (1_000_000 + ppm) / RATE as i64) as u64;
            let (ts, n) = clock.advance_period(observed, PERIOD as usize).unwrap();
            let sample = ts * RATE / 1_000_000;
            assert_eq!(sample, previous);
            assert!(ts.abs_diff(observed) <= 25_000, "ppm={ppm}, period={i}");
            previous += n as u64;
            emitted += n as u64;
        }
        let target = (periods * PERIOD) as i64 * (1_000_000 + ppm) / 1_000_000;
        assert!((emitted as i64 - target).abs() <= 1200);
        println!("3h audio ppm={ppm} periods={periods} samples={emitted}");
    }
    println!("clock simulation elapsed {:.3}s", start.elapsed().as_secs_f64());
}

#[test]
fn accelerated_transition_matches_real_pcm_path() {
    let mut actual = AudioClock::default();
    let mut scalar = AudioClock::default();
    let mut framer = crate::pcm_framer::PcmFramer::new(48000, 2).unwrap();
    let mut next_block = 0u64;
    for i in 0..4000u64 {
        let ts = i * 1024 * 1_000_800 / 48000;
        let (expected_ts, expected_n) = scalar.advance_period(ts, 1024).unwrap();
        let f = actual.correct(&AudioFrame { timestamp_us: ts, sample_rate: 48000, channels: 2,
            samples: vec![123; 2048] }).unwrap();
        assert_eq!((f.timestamp_us, f.samples.len()/2), (expected_ts, expected_n));
        for block in framer.push(&f).unwrap() {
            assert_eq!(block.pts_samples, next_block);
            next_block += 1024;
        }
    }
    assert_eq!(actual.inserted, scalar.inserted);
    assert_eq!(actual.removed, scalar.removed);
    assert_eq!(framer.stats().dropped_or_missing_samples, 0);
}

fn random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn accelerated_venue_fault_scenarios() {
    let start = Instant::now();
    let mut outages = 0u64;
    let mut dropped = 0u64;
    let mut recovered = 0u64;
    // 16 reproducible seeds, each visits every 30fps frame over three hours.
    for seed in 1..=16u64 {
        let mut rng = seed;
        let mut q = BoundedVideoQueue::new(4);
        let mut last = None;
        let mut retry_at = 0;
        let mut link_up = 0;
        let mut failures = 0u32;
        let mut online = true;
        let mut seen_outage = false;
        for i in 0..VIDEO_FRAMES {
            let r = random(&mut rng);
            if online && r.is_multiple_of(9000) {
                online = false;
                seen_outage = true;
                outages += 1;
                link_up = i + 30 + r % 600;
                failures = 1;
                retry_at = i + reconnect::retry_delay(failures).as_secs()*30;
                while q.pop().is_some() {}
            }
            if !online && i >= retry_at {
                if i >= link_up {
                    online = true;
                    recovered += 1;
                    failures = 0;
                } else {
                    failures += 1;
                    retry_at = i + reconnect::retry_delay(failures).as_secs()*30;
                    assert!(retry_at-i <= 960);
                }
            }
            if !online { continue; }
            q.push(VideoFrame { timestamp_us: i*1_000_000/30, sequence: i,
                width: 1920, height: 1080, row_stride_bytes: 3840,
                format: PixelFormat::Yuyv422, bytes: Vec::new() });
            assert!(q.len() <= 4);
            // Simulated service stalls; no claim of measured CPU throughput.
            if r % 100 < 85 {
                let frame = q.pop().unwrap();
                assert!(last.is_none_or(|p| frame.sequence > p));
                assert!(i-frame.sequence <= 4 + 100); // finite service lag in seeded cases
                last = Some(frame.sequence);
            }
        }
        dropped += q.dropped();
        assert!(seen_outage);
    }
    assert!(outages > 100 && recovered > 100 && dropped > 0);
    assert!(!reconnect::retryable(&io::Error::from(io::ErrorKind::PermissionDenied)));
    println!("{} modeled video events; {outages} outages, {recovered} recoveries, {dropped} queue drops; {:.3}s",
        16*VIDEO_FRAMES, start.elapsed().as_secs_f64());
}
