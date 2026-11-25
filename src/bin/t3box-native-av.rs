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

fn publish(url: &str, s: &Settings, video: &V4lVideoSource, audio: &AlsaAudioSource) -> io::Result<()> {
    let mut aac = AacBackend::new(s.audio_bitrate)?;
    let session = rtmps_tls::begin_publish(url)?;
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
        if let Some(mut frame) = audio.try_next_frame()? {
            progress = true;
            if frame.timestamp_us >= origin {
                last_audio = Instant::now();
                frame.timestamp_us = frame.timestamp_us - origin + 100_000;
                let corrected = clock.correct(&frame)?;
                for block in framer.push(&corrected)? {
                    send_audio(&mut pipeline, aac.encode(&block)?)?;
                }
            }
        }
        if let Some(mut frame) = video.try_next_frame()? {
            progress = true;
            if frame.timestamp_us >= origin {
                last_video = Instant::now();
                frame.timestamp_us = frame.timestamp_us - origin + 100_000;
                pipeline.push_frame(&frame)?;
                frames = frames.saturating_add(1);
                if s.max_frames != 0 && frames >= s.max_frames { break; }
            }
        }
        if last_audio.elapsed() > Duration::from_secs(5) || last_video.elapsed() > Duration::from_secs(5) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "capture stalled"));
        }
        if !progress { thread::sleep(Duration::from_millis(2)); }
    }
    send_audio(&mut pipeline, aac.finish()?)?;
    let (_, count) = pipeline.finish()?;
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
    let video = V4lVideoSource::open(s.capture.clone())?;
    let audio = AlsaAudioSource::open(s.capture.clone())?;
    let mut failures = 0u32;
    loop {
        let started = Instant::now();
        match publish(&url, &s, &video, &audio) {
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
                    let _ = video.try_next_frame();
                    let _ = audio.try_next_frame();
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
