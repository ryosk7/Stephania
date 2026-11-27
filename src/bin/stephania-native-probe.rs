//! Standalone Pi 5 hardware probe with direct V4L2 and ALSA sources.
#![cfg(feature = "native-capture")]
use std::{env, io, thread, time::Duration};
use stephania::media_core::{AudioSource, VideoSource, measure_audio};
use stephania::native_capture::{CaptureSettings, V4lVideoSource, AlsaAudioSource};

fn main() {
    if let Err(err) = run() {
        eprintln!("native probe: {err}");
        std::process::exit(1);
    }
}
fn run() -> io::Result<()> {
    let mut settings = CaptureSettings::default();
    if let Ok(path) = env::var("STEPHANIA_VIDEO") { settings.video_path = path; }
    if let Ok(device) = env::var("STEPHANIA_AUDIO") { settings.audio_device = device; }
    if let Ok(width) = env::var("STEPHANIA_WIDTH") { settings.width = width.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput,"invalid width"))?; }
    if let Ok(height) = env::var("STEPHANIA_HEIGHT") { settings.height = height.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput,"invalid height"))?; }
    eprintln!("Native capture diagnostics: {} / {}", settings.video_path, settings.audio_device);
    let mut video = V4lVideoSource::open(settings.clone())?;
    let mut audio = AlsaAudioSource::open(settings)?;
    let video_worker = thread::spawn(move || -> io::Result<()> {
        for _ in 0..20 {
            let frame = video.next_frame()?;
            println!("video seq={} time={}us {}x{} format={:?} bytes={}",
                frame.sequence, frame.timestamp_us, frame.width, frame.height, frame.format, frame.bytes.len());
        }
        Ok(())
    });
    let audio_worker = thread::spawn(move || -> io::Result<()> {
        for _ in 0..40 {
            let frame = audio.next_frame()?;
            let level = measure_audio(&frame.samples);
            println!("audio time={}us rate={} channels={} samples={} peak={:.1}dBFS rms={:.1}dBFS clipped={}",
                frame.timestamp_us, frame.sample_rate, frame.channels,
                frame.samples.len(), level.peak_dbfs, level.rms_dbfs, level.clipped_samples);
        }
        Ok(())
    });
    let video_result = video_worker.join().map_err(|_| io::Error::other("video capture worker panicked"))?;
    let audio_result = audio_worker.join().map_err(|_| io::Error::other("audio capture worker panicked"))?;
    thread::sleep(Duration::from_millis(10));
    video_result?;
    audio_result
}
