//! Device-independent media contract used by capture, encoding and transport.
//! All timestamps are microseconds on a shared monotonic clock. Sources whose
//! clock origins differ must be normalized before combining audio and video.
use std::collections::VecDeque;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Yuyv422,
    Mjpeg,
    Nv12,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFrame {
    pub timestamp_us: u64,
    pub sequence: u64,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioFrame {
    pub timestamp_us: u64,
    pub sample_rate: u32,
    pub channels: u8,
    pub samples: Vec<i16>,
}

pub trait VideoSource {
    fn next_frame(&mut self) -> io::Result<VideoFrame>;
}
pub trait AudioSource {
    fn next_frame(&mut self) -> io::Result<AudioFrame>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec { H264, Aac }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedPacket {
    pub codec: Codec,
    pub dts_us: u64,
    pub pts_us: u64,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

/// Swap codecs without changing capture, buffering, clock sync or RTMP layers.
pub trait VideoEncoder {
    fn encode(&mut self, frame: &VideoFrame) -> io::Result<Vec<EncodedPacket>>;
    fn flush(&mut self) -> io::Result<Vec<EncodedPacket>>;
}
pub trait AudioEncoder {
    fn encode(&mut self, frame: &AudioFrame) -> io::Result<Vec<EncodedPacket>>;
    fn flush(&mut self) -> io::Result<Vec<EncodedPacket>>;
}

pub struct BoundedVideoQueue {
    queue: VecDeque<VideoFrame>,
    max_frames: usize,
    dropped: u64,
}
impl BoundedVideoQueue {
    pub fn new(max_frames: usize) -> Self {
        assert!(max_frames > 0);
        Self { queue: VecDeque::with_capacity(max_frames), max_frames, dropped: 0 }
    }
    pub fn push(&mut self, frame: VideoFrame) {
        if self.queue.len() == self.max_frames {
            self.queue.pop_front();
            self.dropped += 1;
        }
        self.queue.push_back(frame);
    }
    pub fn pop(&mut self) -> Option<VideoFrame> { self.queue.pop_front() }
    pub fn dropped(&self) -> u64 { self.dropped }
    pub fn len(&self) -> usize { self.queue.len() }
    pub fn is_empty(&self) -> bool { self.queue.is_empty() }
}

/// Device timestamps are related to the same Instant origin by calibration.
/// An offset is calculated once per device from a capture timestamp and a
/// clock reference; input timestamp jitter is clamped, but this is NOT an
/// audio resampler or hardware oscillator drift correction.
#[derive(Debug, Default)]
pub struct TimestampNormalizer {
    offset_us: Option<i128>,
    last_output: Option<u64>,
}
impl TimestampNormalizer {
    pub fn observe(&mut self, device_us: u64, monotonic_us: u64) -> u64 {
        let offset = *self.offset_us.get_or_insert(
            i128::from(monotonic_us) - i128::from(device_us)
        );
        let translated = (i128::from(device_us) + offset).clamp(0, i128::from(u64::MAX)) as u64;
        let result = translated.max(self.last_output.unwrap_or(0));
        self.last_output = Some(result);
        result
    }
    pub fn reset(&mut self) { self.offset_us = None; self.last_output = None; }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioLevel {
    pub peak_dbfs: f32,
    pub rms_dbfs: f32,
    pub clipped_samples: usize,
}
pub fn measure_audio(samples: &[i16]) -> AudioLevel {
    if samples.is_empty() {
        return AudioLevel { peak_dbfs: f32::NEG_INFINITY, rms_dbfs: f32::NEG_INFINITY, clipped_samples: 0 };
    }
    let mut peak = 0f64;
    let mut squares = 0f64;
    let mut clipped = 0;
    for &sample in samples {
        let abs = f64::from(sample).abs() / 32768.0;
        peak = peak.max(abs);
        squares += abs * abs;
        if i32::from(sample).abs() >= 32760 { clipped += 1; }
    }
    let db = |n: f64| if n == 0.0 { f32::NEG_INFINITY } else { (20.0 * n.log10()) as f32 };
    AudioLevel { peak_dbfs: db(peak), rms_dbfs: db((squares / samples.len() as f64).sqrt()), clipped_samples: clipped }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QualityProfile {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
}
pub const HD1080_30: QualityProfile = QualityProfile { width: 1920, height: 1080, fps: 30, bitrate_kbps: 6_000 };
pub const HD720_30: QualityProfile = QualityProfile { width: 1280, height: 720, fps: 30, bitrate_kbps: 3_000 };

/// Advisory decision only. Switching profiles requires an actual encoder
/// restart/reconfiguration and an active, validated capture mode.
pub fn suggested_profile(dropped_frames_percent: f32, cpu_temp_c: f32, sustained_upload_kbps: u32) -> QualityProfile {
    if dropped_frames_percent > 3.0 || cpu_temp_c >= 78.0 || sustained_upload_kbps < 7_500 {
        HD720_30
    } else {
        HD1080_30
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(ts: u64) -> VideoFrame {
        VideoFrame { timestamp_us: ts, sequence: ts, width: 2, height: 2, format: PixelFormat::Yuyv422, bytes: vec![0;8] }
    }
    #[test]
    fn queue_drops_stale_video_not_newest() {
        let mut q=BoundedVideoQueue::new(2);
        q.push(frame(1)); q.push(frame(2)); q.push(frame(3));
        assert_eq!(q.dropped(),1);
        assert_eq!(q.pop().unwrap().sequence,2);
        assert_eq!(q.pop().unwrap().sequence,3);
        assert!(q.is_empty());
    }
    #[test]
    fn timestamp_origin_and_clamping() {
        let mut n=TimestampNormalizer::default();
        assert_eq!(n.observe(12_000,500_000),500_000);
        assert_eq!(n.observe(13_000,501_000),501_000);
        assert_eq!(n.observe(12_500,502_000),501_000);
        n.reset();
        assert_eq!(n.observe(900_000,25),25);
    }
    #[test]
    fn audio_level_is_bounded_and_detects_clipping() {
        let m=measure_audio(&[0, 16384, -32768]);
        assert!((m.peak_dbfs-0.0).abs() < 0.01);
        assert_eq!(m.clipped_samples,1);
        assert!(m.rms_dbfs < 0.0);
        assert!(measure_audio(&[]).rms_dbfs.is_infinite());
    }
    #[test]
    fn degraded_thermal_upload_or_fps_selects_720p() {
        assert_eq!(suggested_profile(0.0,60.0,15_000),HD1080_30);
        assert_eq!(suggested_profile(4.0,60.0,15_000),HD720_30);
        assert_eq!(suggested_profile(0.0,79.0,15_000),HD720_30);
        assert_eq!(suggested_profile(0.0,60.0,5_000),HD720_30);
    }
}
