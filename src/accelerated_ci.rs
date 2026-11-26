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
        let mut seed_outages = 0;
        let mut seed_recoveries = 0;
        for i in 0..VIDEO_FRAMES {
            let r = random(&mut rng);
            if online && i + 2000 < VIDEO_FRAMES && r.is_multiple_of(9000) {
                online = false;
                seen_outage = true;
                outages += 1;
                seed_outages += 1;
                link_up = i + 30 + r % 600;
                failures = 1;
                retry_at = i + reconnect::retry_delay(failures).as_secs()*30;
                while q.pop().is_some() {}
            }
            if !online && i >= retry_at {
                if i >= link_up {
                    online = true;
                    recovered += 1;
                    seed_recoveries += 1;
                    assert!(i-link_up <= 960);
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
                assert!(i-frame.sequence <= 4); // newest bounded queue under per-frame arrivals
                last = Some(frame.sequence);
            }
        }
        dropped += q.dropped();
        assert!(seen_outage);
        assert!(online);
        assert_eq!(seed_outages, seed_recoveries);
    }
    assert!(outages > 100 && dropped > 0);
    assert_eq!(outages, recovered);
    assert!(!reconnect::retryable(&io::Error::from(io::ErrorKind::PermissionDenied)));
    println!("{} modeled video events; {outages} outages, {recovered} recoveries, {dropped} queue drops; {:.3}s",
        16*VIDEO_FRAMES, start.elapsed().as_secs_f64());
}


/// Every audio/video packet is serialized and parsed using production RTMP
/// components. Payloads are synthetic, not encoded media; no TLS or real I/O.
#[test]
fn accelerated_three_hour_rtmp_roundtrip() {
    use crate::{rtmp::RtmpChunkWriter, rtmp_read::ChunkReader};
    let start = Instant::now();
    let audio_packets = SECONDS * RATE / PERIOD;
    let mut a = 0u64;
    let mut v = 0u64;
    let mut bytes = Vec::with_capacity(1024);
    let mut reader = ChunkReader::new();
    let mut last = 0u32;
    let mut wire_bytes = 0u64;
    while a < audio_packets || v < VIDEO_FRAMES {
        let audio = a < audio_packets && (v == VIDEO_FRAMES || a * PERIOD * 30 <= v * RATE);
        let (kind, csid, ts, seq, size) = if audio {
            let seq = a;
            a += 1;
            (8, 4, (seq * PERIOD * 1000 / RATE) as u32, seq, 64usize)
        } else {
            let seq = v;
            v += 1;
            (9, 6, (seq * 1000 / 30) as u32, seq, if seq.is_multiple_of(60) { 513 } else { 129 })
        };
        let mut payload = [0x5au8; 513];
        payload[..8].copy_from_slice(&seq.to_be_bytes());
        assert!(ts >= last);
        last = ts;
        bytes.clear();
        let mut writer = RtmpChunkWriter::new(&mut bytes, 128).unwrap();
        writer.message(csid, kind, 1, ts, &payload[..size]).unwrap();
        wire_bytes += bytes.len() as u64;
        let mut input = bytes.as_slice();
        let msg = reader.read_message(&mut input).unwrap();
        assert_eq!((msg.type_id, msg.stream_id, msg.timestamp_ms), (kind, 1, ts));
        assert_eq!(msg.payload.as_slice(), &payload[..size]);
        assert!(input.is_empty());
        // Exercise session resets with fresh reader state at 10-minute intervals.
        if v > 0 && v.is_multiple_of(18_000) && !audio {
            reader = ChunkReader::new();
        }
    }
    assert_eq!(v, 324_000);
    assert_eq!(a, 506_250);
    assert!(last >= 10_799_960 && last < 10_800_000);
    println!("3h RTMP roundtrip: {v} video, {a} audio, {wire_bytes} wire bytes; {:.3}s", start.elapsed().as_secs_f64());
}

/// Replay 3 hours of 1 Hz TCP observations with the actual rate controller.
/// Each 180-second cycle has congestion, recovery, and a reconnect.
#[test]
fn accelerated_three_hour_rate_recovery() {
    use crate::rate_control::{Observation, RateController};
    let start = Instant::now();
    let mut c = RateController::new(6000);
    let mut retrans = 0;
    for cycle in 0..60 {
        assert_eq!(c.current(), 6000);
        for tick in 0..180 {
            if tick == 10 { c.reconnect(); }
            if tick == 20 { c.missing_sample(); }
            let congested = tick < 8;
            if congested { retrans += 1; }
            c.observe(Observation {
                not_sent_bytes: if congested { 1_000_000 } else { 0 },
                rtt_us: if congested { 200_000 } else { 40_000 },
                retrans_total: retrans,
            });
            assert!((1500..=6000).contains(&c.current()), "cycle {cycle}, tick {tick}");
            if tick == 7 { assert_eq!(c.current(), 1500); }
        }
        assert_eq!(c.current(), 6000);
    }
    println!("3h rate recovery: 10800 observations, 60 congestion/reconnect cycles; {:.3}s", start.elapsed().as_secs_f64());
}
