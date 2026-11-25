//! Native Linux HDMI (V4L2 mmap) and USB audio (ALSA PCM) capture workers.
//!
//! Capture is on dedicated threads because V4L2 streams borrow the Device and
//! ALSA IO objects borrow the PCM. A bounded channel prevents unbounded RAM
//! growth if encoders fall behind. These workers DO NOT perform decoding,
//! colorspace conversion, resampling or any H.264/AAC compression.
use crate::media_core::{AudioFrame, AudioSource, PixelFormat, VideoFrame, VideoSource};
use std::io;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use alsa::{pcm::{Access, Format, HwParams, PCM}, Direction, ValueOr};
use v4l::{buffer::Type, io::{mmap::Stream, traits::CaptureStream}, video::Capture, Device, FourCC};

#[derive(Clone, Debug)]
pub struct CaptureSettings {
    pub video_path: String,
    pub audio_device: String,
    pub width: u32,
    pub height: u32,
    pub sample_rate: u32,
    pub audio_channels: u8,
    pub audio_period_frames: usize,
    pub frame_queue: usize,
    pub preferred_format: PixelFormat,
}
impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            video_path: "/dev/video0".into(),
            audio_device: "plughw:1,0".into(),
            width: 1280, height: 720,
            sample_rate: 48_000, audio_channels: 2,
            audio_period_frames: 1024, frame_queue: 4,
            preferred_format: PixelFormat::Yuyv422,
        }
    }
}
impl CaptureSettings {
    fn validate(&self) -> io::Result<()> {
        if self.width == 0 || self.height == 0 ||
            self.width > 3840 || self.height > 2160 ||
            self.audio_channels == 0 || self.audio_channels > 8 ||
            !(8_000..=192_000).contains(&self.sample_rate) ||
            !(64..=8192).contains(&self.audio_period_frames) ||
            !(1..=16).contains(&self.frame_queue) {
            return Err(invalid("capture configuration out of range"));
        }
        Ok(())
    }
}

pub struct V4lVideoSource {
    rx: Receiver<io::Result<VideoFrame>>,
}
pub struct AlsaAudioSource {
    rx: Receiver<io::Result<AudioFrame>>,
}

impl VideoSource for V4lVideoSource {
    fn next_frame(&mut self) -> io::Result<VideoFrame> {
        recv(&self.rx)
    }
}
impl AudioSource for AlsaAudioSource {
    fn next_frame(&mut self) -> io::Result<AudioFrame> {
        recv(&self.rx)
    }
}
fn recv<T>(rx: &Receiver<io::Result<T>>) -> io::Result<T> {
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|e| io::Error::new(io::ErrorKind::TimedOut, e))?
}

fn timestamp_us() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

fn invalid(text: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, text)
}

fn pixel_fourcc(format: PixelFormat) -> FourCC {
    match format {
        PixelFormat::Yuyv422 => FourCC::new(b"YUYV"),
        PixelFormat::Mjpeg => FourCC::new(b"MJPG"),
        PixelFormat::Nv12 => FourCC::new(b"NV12"),
    }
}

impl V4lVideoSource {
    pub fn open(settings: CaptureSettings) -> io::Result<Self> {
        settings.validate()?;
        let (tx, rx) = mpsc::sync_channel(settings.frame_queue);
        thread::Builder::new().name("t3box-v4l2".into()).spawn(move || {
            // Reopen and renegotiate when the UVC device disappears.
            while !capture_video(&settings, &tx) {
                thread::sleep(Duration::from_secs(2));
            }
        })?;
        Ok(Self { rx })
    }
}

// true means that the receiver has gone away and the worker can exit.
fn capture_video(settings: &CaptureSettings, tx: &SyncSender<io::Result<VideoFrame>>) -> bool {
    let result = capture_video_session(settings, tx);
    match result {
        Ok(()) => true,
        Err(error) => matches!(tx.try_send(Err(error)), Err(TrySendError::Disconnected(_))),
    }
}

fn capture_video_session(settings: &CaptureSettings, tx: &SyncSender<io::Result<VideoFrame>>) -> io::Result<()> {
    let device = Device::with_path(&settings.video_path)?;
    let mut format = device.format()?;
    format.width = settings.width;
    format.height = settings.height;
    format.fourcc = pixel_fourcc(settings.preferred_format);
    let actual = device.set_format(&format)?;
    if actual.width != settings.width || actual.height != settings.height ||
        actual.fourcc != format.fourcc {
        return Err(invalid("V4L2 driver did not accept requested capture format"));
    }
    let mut stream = Stream::with_buffers(&device, Type::VideoCapture, 4)?;
    loop {
        let (buffer, meta) = stream.next()?;
        let frame = VideoFrame {
            timestamp_us: timestamp_us(),
            sequence: u64::from(meta.sequence),
            width: actual.width,
            height: actual.height,
            format: settings.preferred_format,
            bytes: buffer.to_vec(),
        };
        // Overloaded pipelines may drop video, never wait for an encoder.
        match tx.try_send(Ok(frame)) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => return Ok(()),
        }
    }
}

impl AlsaAudioSource {
    pub fn open(settings: CaptureSettings) -> io::Result<Self> {
        settings.validate()?;
        let (tx, rx) = mpsc::sync_channel(settings.frame_queue);
        thread::Builder::new().name("t3box-alsa".into()).spawn(move || {
            while !capture_audio(&settings, &tx) {
                thread::sleep(Duration::from_secs(2));
            }
        })?;
        Ok(Self { rx })
    }
}

fn capture_audio(settings: &CaptureSettings, tx: &SyncSender<io::Result<AudioFrame>>) -> bool {
    let result = capture_audio_session(settings, tx);
    match result {
        Ok(()) => true,
        Err(error) => matches!(tx.try_send(Err(error)), Err(TrySendError::Disconnected(_))),
    }
}

fn capture_audio_session(settings: &CaptureSettings, tx: &SyncSender<io::Result<AudioFrame>>) -> io::Result<()> {
    let pcm = PCM::new(&settings.audio_device, Direction::Capture, false)
        .map_err(audio_error)?;
    {
        let hw = HwParams::any(&pcm).map_err(audio_error)?;
        hw.set_channels(u32::from(settings.audio_channels)).map_err(audio_error)?;
        hw.set_rate(settings.sample_rate, ValueOr::Nearest).map_err(audio_error)?;
        hw.set_format(Format::s16()).map_err(audio_error)?;
        hw.set_access(Access::RWInterleaved).map_err(audio_error)?;
        pcm.hw_params(&hw).map_err(audio_error)?;
    }
    let actual_rate = pcm.hw_params_current().map_err(audio_error)?
        .get_rate().map_err(audio_error)?;
    if actual_rate != settings.sample_rate {
        return Err(invalid("ALSA device did not support exact capture sample rate"));
    }
    let mut samples = vec![0i16; settings.audio_period_frames * usize::from(settings.audio_channels)];
    loop {
        let frames = match pcm.io_i16().map_err(audio_error)?.readi(&mut samples) {
            Ok(n) => n,
            Err(e) => {
                // ALSA EPIPE is an xrun. Prepare the device, then continue;
                // the following capture timestamp is recalibrated to now.
                if e.errno() == 32 {
                    pcm.prepare().map_err(audio_error)?;
                    continue;
                }
                return Err(audio_error(e));
            }
        };
        if frames == 0 { continue; }
        let len = frames * usize::from(settings.audio_channels);
        let frame = AudioFrame {
            timestamp_us: timestamp_us(),
            sample_rate: actual_rate,
            channels: settings.audio_channels,
            samples: samples[..len].to_vec(),
        };
        // Audio should not be silently dropped: errors are surfaced to the
        // owner when backpressure makes the capture timeline unreliable.
        match tx.try_send(Ok(frame)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(io::Error::new(io::ErrorKind::WouldBlock, "audio capture queue overflow"));
            }
            Err(TrySendError::Disconnected(_)) => return Ok(()),
        }
    }
}

fn audio_error(error: alsa::Error) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_settings_have_bounded_buffers() {
        let s=CaptureSettings::default();
        assert!(s.validate().is_ok());
        assert!(s.frame_queue <= 16);
    }
    #[test]
    fn invalid_capture_parameters_fail_before_device_access() {
        let invalid_queue = CaptureSettings { frame_queue: 0, ..CaptureSettings::default() };
        assert!(invalid_queue.validate().is_err());
        let invalid_rate = CaptureSettings { sample_rate: 1, ..CaptureSettings::default() };
        assert!(invalid_rate.validate().is_err());
    }
    #[test]
    fn fourcc_matches_video_source_formats() {
        assert_eq!(pixel_fourcc(PixelFormat::Yuyv422), FourCC::new(b"YUYV"));
        assert_eq!(pixel_fourcc(PixelFormat::Mjpeg), FourCC::new(b"MJPG"));
    }
}
