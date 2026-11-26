//! Experimental native combined A/V publisher. No FFmpeg subprocess.
use std::{env, io, thread, time::{Duration, Instant}};
use t3box::{
    av_clock::AudioClock, media_core::PixelFormat,
    native_aac::{AacBackend, AacPacket},
    native_capture::{timestamp_us, AlsaAudioSource, CaptureSettings, V4lVideoSource},
    native_video::NativeVideoPipeline, pcm_framer::PcmFramer, reconnect, rtmps_tls,
};

fn number(key: &str, default: u32) -> io::Result<u32> {
    env::var(key).ok().map(|s| s.parse().map_err(|_| invalid("invalid numeric setting")))
        .transpose().map(|v| v.unwrap_or(default))
}
fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}
// Only fixed stage names are logged; never include peer responses or URLs.
fn stage<T>(result: io::Result<T>, name: &'static str) -> io::Result<T> {
    result.inspect_err(|error| {
        eprintln!("native A/V stage {name} failed ({:?})", error.kind());
    })
}
fn send_audio<T: io::Read + io::Write>(pipeline: &mut NativeVideoPipeline<T>, packets: Vec<AacPacket>) -> io::Result<()> {
    for packet in packets {
        // The common 100 ms startup offset covers AAC encoder priming.
        let ms = u32::try_from(i128::from(packet.pts_samples) * 1000 / 48000)
            .map_err(|_| invalid("AAC timestamp out of range"))?;
        pipeline.push_aac(ms, &packet.raw)?;
    }
    Ok(())
}

struct Settings {
    capture: CaptureSettings,
    fps: u32,
    video_bitrate: u32,
    audio_bitrate: u32,
    max_frames: u32,
}

enum Sources {
    Hardware(V4lVideoSource, AlsaAudioSource),
    #[cfg(feature = "ci-virtual")]
    Virtual(VirtualCapture),
}
impl Sources {
    fn open(s: &Settings) -> io::Result<Self> {
        #[cfg(feature = "ci-virtual")]
        if env::var_os("T3BOX_CI_VIRTUAL").is_some() {
            return Ok(Self::Virtual(VirtualCapture {
                origin: timestamp_us(), video_index: 0, audio_index: 0,
                width: s.capture.width, height: s.capture.height, fps: s.fps,
            }));
        }
        Ok(Self::Hardware(V4lVideoSource::open(s.capture.clone())?, AlsaAudioSource::open(s.capture.clone())?))
    }
    fn video(&mut self) -> io::Result<Option<t3box::media_core::VideoFrame>> {
        match self {
            Self::Hardware(v, _) => v.try_next_frame(),
            #[cfg(feature = "ci-virtual")]
            Self::Virtual(v) => Ok(v.video()),
        }
    }
    fn audio(&mut self) -> io::Result<Option<t3box::media_core::AudioFrame>> {
        match self {
            Self::Hardware(_, a) => a.try_next_frame(),
            #[cfg(feature = "ci-virtual")]
            Self::Virtual(v) => Ok(v.audio()),
        }
    }
}
#[cfg(feature = "ci-virtual")]
struct VirtualCapture {
    origin: u64,
    video_index: u64,
    audio_index: u64,
    width: u32,
    height: u32,
    fps: u32,
}
#[cfg(feature = "ci-virtual")]
impl VirtualCapture {
    fn video(&mut self) -> Option<t3box::media_core::VideoFrame> {
        let now = timestamp_us();
        let available = (now.saturating_sub(self.origin)) * u64::from(self.fps) / 1_000_000;
        if self.video_index > available { return None; }
        self.video_index = available + 1;
        let timestamp_us = self.origin + available * 1_000_000 / u64::from(self.fps);
        let mut bytes = vec![0; self.width as usize * self.height as usize * 2];
        for (i, pixel) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let y = (16 + ((i / 32 + available as usize * 3) % 200)) as u8;
            pixel.copy_from_slice(&[y, 128, y, 128]);
        }
        Some(t3box::media_core::VideoFrame { timestamp_us, sequence: available,
            width: self.width, height: self.height, row_stride_bytes: self.width as usize * 2,
            format: PixelFormat::Yuyv422, bytes })
    }
    fn audio(&mut self) -> Option<t3box::media_core::AudioFrame> {
        let now = timestamp_us();
        let available = now.saturating_sub(self.origin) * 48000 / 1_000_000 / 1024;
        if self.audio_index >= available { return None; }
        // Bound backlog after a blocked network write to four periods.
        self.audio_index = self.audio_index.max(available.saturating_sub(4));
        let index = self.audio_index;
        self.audio_index += 1;
        let samples = (0..1024).flat_map(|j| {
            let v = ((((index * 1024 + j) % 109) as i16) - 54) * 200;
            [v,v]
        }).collect();
        Some(t3box::media_core::AudioFrame {
            timestamp_us: self.origin + (index * 1024 * 1_000_000).div_ceil(48000),
            sample_rate: 48000, channels: 2, samples,
        })
    }
}

fn publish(url: &str, s: &Settings, sources: &mut Sources) -> io::Result<()> {
    let mut aac = AacBackend::new(s.audio_bitrate)?;
    let session = stage(rtmps_tls::begin_publish(url), "publish negotiation")?;
    let mut pipeline = NativeVideoPipeline::new(session, s.capture.width as usize,
        s.capture.height as usize, s.fps, s.video_bitrate)?;
    pipeline.configure_audio(&aac.configuration()?)?;
    let origin = timestamp_us();
    let mut clock = AudioClock::default();
    let mut framer = PcmFramer::new(48000, 2)?;
    let mut frames = 0u32;
    let mut last_video = Instant::now();
    let mut last_audio = Instant::now();
    loop {
        let mut progress = false;
        if let Some(mut frame) = stage(sources.audio(), "audio capture")? {
            progress = true;
            if frame.timestamp_us >= origin {
                last_audio = Instant::now();
                frame.timestamp_us = frame.timestamp_us - origin + 100_000;
                let corrected = stage(clock.correct(&frame), "audio clock")?;
                for block in stage(framer.push(&corrected), "PCM framing")? {
                    stage(send_audio(&mut pipeline, stage(aac.encode(&block), "AAC encode")?), "audio send")?;
                }
            }
        }
        if let Some(mut frame) = stage(sources.video(), "video capture")? {
            progress = true;
            if frame.timestamp_us >= origin {
                last_video = Instant::now();
                frame.timestamp_us = frame.timestamp_us - origin + 100_000;
                stage(pipeline.push_frame(&frame), "video encode/send")?;
                frames = frames.saturating_add(1);
                if s.max_frames != 0 && frames >= s.max_frames { break; }
            }
        }
        if last_audio.elapsed() > Duration::from_secs(5) || last_video.elapsed() > Duration::from_secs(5) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "capture stalled"));
        }
        if !progress { thread::sleep(Duration::from_millis(2)); }
    }
    stage(send_audio(&mut pipeline, stage(aac.finish(), "AAC flush")?), "audio flush send")?;
    let (_, count) = stage(pipeline.finish(), "video flush send")?;
    eprintln!("native A/V session finished: {count} video access units");
    Ok(())
}

fn run() -> io::Result<()> {
    let url = env::var("T3BOX_OUTPUT").map_err(|_| invalid("T3BOX_OUTPUT required"))?;
    rtmps_tls::Target::parse(&url)?;
    let mut capture = CaptureSettings::default();
    if let Ok(path) = env::var("T3BOX_VIDEO") { capture.video_path = path; }
    if let Ok(device) = env::var("T3BOX_AUDIO") { capture.audio_device = device; }
    capture.width = number("T3BOX_WIDTH", 1280)?;
    capture.height = number("T3BOX_HEIGHT", 720)?;
    capture.preferred_format = match env::var("T3BOX_PIXEL_FORMAT").as_deref() {
        Ok("nv12") => PixelFormat::Nv12,
        Ok("yuyv") | Err(_) => PixelFormat::Yuyv422,
        _ => return Err(invalid("unsupported pixel format")),
    };
    let s = Settings { capture, fps: number("T3BOX_FPS", 30)?,
        video_bitrate: number("T3BOX_BITRATE_KBPS", 3000)?,
        audio_bitrate: number("T3BOX_AUDIO_BITRATE_KBPS", 128)?,
        max_frames: number("T3BOX_MAX_FRAMES", 0)? };
    // Validate codecs before opening devices or attempting network retries.
    AacBackend::new(s.audio_bitrate)?;
    t3box::native_x264::X264Backend::new(s.capture.width as usize,
        s.capture.height as usize, s.fps, s.video_bitrate, true)?;
    let mut sources = Sources::open(&s)?;
    let mut failures = 0u32;
    loop {
        let started = Instant::now();
        match publish(&url, &s, &mut sources) {
            Ok(()) => return Ok(()),
            Err(e) if !reconnect::retryable(&e) => return Err(e),
            Err(e) => {
                if started.elapsed() >= Duration::from_secs(60) { failures = 0; }
                failures = failures.saturating_add(1);
                let delay = reconnect::retry_delay(failures);
                // Do not print URLs or server responses containing stream keys.
                eprintln!("native A/V session failed ({:?}); reconnect in {}s", e.kind(), delay.as_secs());
                // Keep draining capture queues during backoff; never replay stale media.
                let until = Instant::now() + delay;
                while Instant::now() < until {
                    let _ = sources.video();
                    let _ = sources.audio();
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("native A/V stopped ({:?})", e.kind());
        std::process::exit(1);
    }
}
