//! Shared native A/V publisher used by the CLI and controller.
use std::{env, io, thread, time::{Duration, Instant}};
use crate::{
    media_core::PixelFormat,
    native_aac::{AacBackend, AacPacket},
    native_capture::{timestamp_us, AlsaAudioSource, CaptureSettings, V4lVideoSource},
    native_video::{NativeVideoPipeline, NativeVideoEncoder, NativeMediaSink}, reconnect, rtmps_tls,
};

// Limit raw video age to three frame periods (100 ms at 30 fps).
fn video_max_age(fps: u32) -> Duration {
    Duration::from_micros(3_000_000 / u64::from(fps.max(1)))
}

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

pub struct Settings {
    pub capture: CaptureSettings,
    pub fps: u32,
    pub video_bitrate: u32,
    pub audio_bitrate: u32,
    pub max_frames: u32,
}

/// Video source plus the audio inputs of `crate::audio::control()`.
enum Sources {
    Hardware(V4lVideoSource, [Option<AlsaAudioSource>; 2]),
    #[cfg(feature = "ci-virtual")]
    Virtual(VirtualCapture),
}

/// ALSA settings for one mix input. Short periods and a 16-period queue
/// keep mix latency low without dropping audio under scheduling jitter.
fn input_settings(capture: &CaptureSettings, cfg: &crate::audio::InputConfig) -> CaptureSettings {
    let mut c = capture.clone();
    c.audio_device = cfg.device.clone();
    c.audio_channels = cfg.channels;
    c.audio_period_frames = 512;
    c.frame_queue = 16;
    c
}

impl Sources {
    fn open(s: &Settings) -> io::Result<Self> {
        #[cfg(feature = "ci-virtual")]
        if env::var_os("STEPHANIA_CI_VIRTUAL").is_some() {
            return Ok(Self::Virtual(VirtualCapture {
                origin: timestamp_us(), video_index: 0, audio_index: [0, 0],
                width: s.capture.width, height: s.capture.height, fps: s.fps,
                fixture: VirtualFixture::open(s)?,
            }));
        }
        Ok(Self::Hardware(V4lVideoSource::open(s.capture.clone())?, [None, None]))
    }

    /// Opens or closes inputs to match the configuration. A closed ALSA
    /// source stops its capture thread; an open one reopens the device on
    /// its own after errors (hotplug).
    fn apply_audio(&mut self, capture: &CaptureSettings, cfg: &[crate::audio::InputConfig; 2]) -> io::Result<()> {
        match self {
            Self::Hardware(_, inputs) => {
                for (slot, c) in inputs.iter_mut().zip(cfg) {
                    *slot = if c.enabled { Some(AlsaAudioSource::open(input_settings(capture, c))?) } else { None };
                }
            }
            #[cfg(feature = "ci-virtual")]
            Self::Virtual(v) => {
                let now = timestamp_us();
                for (i, c) in cfg.iter().enumerate() {
                    // Restart the generator at the current time when enabled.
                    v.audio_index[i] = if c.enabled { now.saturating_sub(v.origin) * 48000 / 1_000_000 / 1024 } else { u64::MAX };
                }
            }
        }
        Ok(())
    }

    fn video(&mut self, max_age: Duration) -> io::Result<Option<crate::media_core::VideoFrame>> {
        match self {
            Self::Hardware(v, _) => v.try_latest_frame(max_age),
            #[cfg(feature = "ci-virtual")]
            Self::Virtual(v) => v.video(),
        }
    }

    fn audio(&mut self, input: usize) -> io::Result<Option<crate::media_core::AudioFrame>> {
        match self {
            Self::Hardware(_, inputs) => match &inputs[input] {
                Some(a) => a.try_next_frame(),
                None => Ok(None),
            },
            #[cfg(feature = "ci-virtual")]
            Self::Virtual(v) => Ok(v.audio(input)),
        }
    }

    /// Discards queued audio (reconnect backoff): never replay stale media.
    fn drain_audio(&mut self) {
        for i in 0..2 {
            for _ in 0..64 { if !matches!(self.audio(i), Ok(Some(_))) { break; } }
        }
    }
}

/// Program audio: every input goes through the mixer (docs/AUDIO.md).
struct AudioPipeline {
    mixer: crate::audio_mixer::AudioMixer,
    generation: Option<u64>,
    mixed: Vec<crate::audio_mixer::MixedBlock>,
}

impl AudioPipeline {
    fn new() -> io::Result<Self> {
        let latency = number("STEPHANIA_AUDIO_LATENCY_MS", 40)?;
        if !(20..=200).contains(&latency) { return Err(invalid("STEPHANIA_AUDIO_LATENCY_MS must be 20..200")); }
        Ok(Self { mixer: crate::audio_mixer::AudioMixer::new(latency), generation: None, mixed: Vec::new() })
    }

    fn blocks(&mut self, sources: &mut Sources, capture: &CaptureSettings, origin: u64) -> io::Result<Vec<(crate::pcm_framer::PcmBlock, Instant)>> {
        let ctl = crate::audio::control();
        let (cfg, generation) = ctl.config();
        if self.generation != Some(generation) {
            sources.apply_audio(capture, &cfg)?;
            for (i, c) in cfg.iter().enumerate() {
                self.mixer.reset_input(i);
                if c.enabled { ctl.input_opened(i); }
            }
            self.generation = Some(generation);
        }
        let now = Instant::now();
        for (i, c) in cfg.iter().enumerate() {
            if !c.enabled { continue; }
            for _ in 0..16 {
                match sources.audio(i) {
                    Ok(Some(mut frame)) => {
                        ctl.input_frame(i, now);
                        if frame.timestamp_us < origin { continue; }
                        frame.timestamp_us = frame.timestamp_us - origin + 100_000;
                        if self.mixer.ingest(i, &frame).is_err() { self.mixer.reset_input(i); }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        ctl.input_error(i, &e.to_string());
                        self.mixer.reset_input(i);
                        break;
                    }
                }
            }
        }
        let targets = [0, 1].map(|i| if cfg[i].enabled && !cfg[i].muted { crate::audio::db_to_gain(cfg[i].gain_db) } else { 0.0 });
        let now_us = timestamp_us().saturating_sub(origin);
        self.mixed.clear();
        if self.mixer.poll(now_us, targets, &mut self.mixed).resynced {
            ctl.record_resync();
            eprintln!("audio mixer: publisher stalled for more than 1 s; audio timeline resynchronised");
        }
        let mut result = Vec::with_capacity(self.mixed.len());
        for m in self.mixed.drain(..) {
            ctl.record_block(m.inputs, m.master, m.clipped, m.limited);
            let capture_us = m.block.timestamp_us().saturating_sub(100_000);
            let captured = Instant::now().checked_sub(Duration::from_micros(now_us.saturating_sub(capture_us))).unwrap_or_else(Instant::now);
            result.push((m.block, captured));
        }
        Ok(result)
    }
}

/// Marks the audio subsystem running for the lifetime of a publisher run.
struct AudioRunning;
impl AudioRunning {
    fn start() -> Self { crate::audio::control().set_running(true); Self }
}
impl Drop for AudioRunning {
    fn drop(&mut self) { crate::audio::control().set_running(false); }
}
#[cfg(feature = "ci-virtual")]
struct VirtualCapture {
    origin: u64,
    video_index: u64,
    /// Next 1024-sample period per input; u64::MAX = input disabled.
    audio_index: [u64; 2],
    width: u32,
    height: u32,
    fps: u32,
    fixture: Option<VirtualFixture>,
}
#[cfg(feature = "ci-virtual")]
struct VirtualFixture {
    video: std::fs::File,
    frames: u64,
}
#[cfg(feature = "ci-virtual")]
impl VirtualFixture {
    fn open(s: &Settings) -> io::Result<Option<Self>> {
        let Some(path) = env::var_os("STEPHANIA_LAB_VIDEO") else { return Ok(None); };
        let video = std::fs::File::open(path)?;
        let frame_bytes = u64::from(s.capture.width)*u64::from(s.capture.height)*2;
        let len = video.metadata()?.len();
        if frame_bytes == 0 || len == 0 || !len.is_multiple_of(frame_bytes) {
            return Err(invalid("invalid lab fixture length"));
        }
        Ok(Some(Self { video, frames: len/frame_bytes }))
    }
    fn read(&mut self, index: u64, bytes: &mut [u8]) -> io::Result<()> {
        use std::io::{Read,Seek,SeekFrom};
        self.video.seek(SeekFrom::Start(index % self.frames * bytes.len() as u64))?;
        self.video.read_exact(bytes)
    }
}
#[cfg(feature = "ci-virtual")]
impl VirtualCapture {
    fn video(&mut self) -> io::Result<Option<crate::media_core::VideoFrame>> {
        let now = timestamp_us();
        let available = (now.saturating_sub(self.origin)) * u64::from(self.fps) / 1_000_000;
        if self.video_index > available { return Ok(None); }
        if self.video_index == 0 && env::var_os("STEPHANIA_LAB_MARKERS").is_some() {
            #[repr(C)]
            struct Timespec { sec: std::ffi::c_long, nsec: std::ffi::c_long }
            unsafe extern "C" { fn clock_gettime(clock: std::ffi::c_int, value: *mut Timespec) -> std::ffi::c_int; }
            let before = timestamp_us();
            let mut t = Timespec { sec: 0, nsec: 0 };
            let result = unsafe { clock_gettime(1, &mut t) };
            let after = timestamp_us();
            if result != 0 { return Err(io::Error::last_os_error()); }
            let epoch_ns = i128::from(t.sec)*1_000_000_000 + i128::from(t.nsec)
                - i128::from((before+after)/2-self.origin)*1000;
            eprintln!("lab source epoch_ns={epoch_ns} uncertainty_us={}", after-before);
        }
        self.video_index = available + 1;
        let timestamp_us = self.origin + available * 1_000_000 / u64::from(self.fps);
        let mut bytes = vec![0; self.width as usize * self.height as usize * 2];
        if let Some(fixture) = &mut self.fixture { fixture.read(available, &mut bytes)?; } else {
        for (i, pixel) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let y = (16 + ((i / 32 + available as usize * 3) % 200)) as u8;
            pixel.copy_from_slice(&[y, 128, y, 128]);
        }
        }
        // Optional source sequence marker survives lossy encoding. 32 bits, 16x16 blocks.
        if self.width >= 512 && self.height >= 16 && env::var_os("STEPHANIA_LAB_MARKERS").is_some() {
            for y in 0..16usize { for x in 0..512usize {
                let i = (y*self.width as usize+x)*2;
                bytes[i] = if (available >> (x/16)) & 1 == 1 {235} else {16};
                bytes[i+1]=128;
            }}
        }
        Ok(Some(crate::media_core::VideoFrame { timestamp_us, sequence: available,
            width: self.width, height: self.height, row_stride_bytes: self.width as usize * 2,
            format: PixelFormat::Yuyv422, bytes }))
    }
    /// Input 0 (`hdmi`): the deterministic lab pattern, or a 440 Hz tone with
    /// `STEPHANIA_CI_AUDIO_TONES`. Input 1 (`mic`): an 880 Hz mono tone. Tones
    /// at −12 dBFS let CI verify mixing, gain and mute in the decoded output.
    fn audio(&mut self, input: usize) -> Option<crate::media_core::AudioFrame> {
        if self.audio_index[input] == u64::MAX { return None; }
        let now = timestamp_us();
        let available = now.saturating_sub(self.origin) * 48000 / 1_000_000 / 1024;
        if self.audio_index[input] >= available { return None; }
        // Bound backlog after a blocked network write to four periods.
        self.audio_index[input] = self.audio_index[input].max(available.saturating_sub(4));
        let index = self.audio_index[input];
        self.audio_index[input] += 1;
        let tone = |hz: f64, j: u64| ((2.0 * std::f64::consts::PI * hz * ((index * 1024 + j) as f64) / 48000.0).sin() * 8192.0) as i16;
        let (channels, samples): (u8, Vec<i16>) = if input == 1 {
            (1, (0..1024).map(|j| tone(880.0, j)).collect())
        } else if env::var_os("STEPHANIA_CI_AUDIO_TONES").is_some() {
            (2, (0..1024).flat_map(|j| { let v = tone(440.0, j); [v, v] }).collect())
        } else {
            let markers = env::var_os("STEPHANIA_LAB_MARKERS").is_some();
            (2, (0..1024).flat_map(|j| {
                let v = ((((index * 1024 + j) % 109) as i16) - 54) * 200;
                let v = if markers && (index*1024+j)%48000 < 2400 { v } else if markers {0} else {v};
                [v,v]
            }).collect())
        };
        Some(crate::media_core::AudioFrame {
            timestamp_us: self.origin + (index * 1024 * 1_000_000).div_ceil(48000),
            sample_rate: 48000, channels, samples,
        })
    }
}

/// CI only: feed a deterministic phone:rear track when requested, so the
/// composed broadcast can be verified without a phone.
#[cfg(feature = "ci-virtual")]
fn virtual_phone_tick(timestamp_us: u64) {
    if env::var_os("STEPHANIA_CI_VIRTUAL_PHONE").is_some() {
        crate::composition::sources().submit("phone:rear", crate::composition::virtual_phone_frame(timestamp_us));
    }
}

fn publish(url: &str, s: &Settings, sources: &mut Sources, rate: &mut crate::rate_control::RateController, cancelled: &impl Fn() -> bool) -> io::Result<()> {
    let mut aac = AacBackend::new(s.audio_bitrate)?;
    let mut session = stage(rtmps_tls::begin_publish(url), "publish negotiation")?;
    // Opt-in until the independent network benchmark validates the budget.
    let watchdog = if env::var("STEPHANIA_DEADLINE_TRANSPORT").as_deref() == Ok("1") {
        let guard = crate::transport_deadline::DeadlineWatchdog::start_with_queue_budget(
            session.tcp_monitor()?, crate::transport_deadline::QueueBudget {
                max_not_sent_bytes: (rate.current() * 150 / 8).max(32_768),
                max_sustained: Duration::from_millis(500),
            })?;
        session.set_media_deadline(guard.handle(), Duration::from_millis(500));
        Some(guard)
    } else { None };
    let telemetry = session.tcp_monitor().ok();
    let mut next_sample = Instant::now();
    let adaptive = env::var("STEPHANIA_ADAPTIVE_RATE").as_deref() == Ok("1");
    let content_aware = adaptive && env::var("STEPHANIA_CONTENT_AWARE").as_deref() == Ok("1");
    let mut network_rate = if adaptive { rate.current() } else { s.video_bitrate };
    let mut applied_rate = network_rate;
    let mut pipeline = NativeVideoPipeline::new(session, s.capture.width as usize,
        s.capture.height as usize, s.fps, network_rate)?;
    if adaptive { pipeline.reconfigure_rate(network_rate, 500)?; }
    if content_aware { pipeline.enable_content_analysis(); }
    pipeline.configure_audio(&aac.configuration()?)?;
    let mut composition = crate::composition::enabled()
        .then(|| crate::composition::CompositionStage::new(s.capture.width as usize, s.capture.height as usize));
    let origin = timestamp_us();
    let mut audio = AudioPipeline::new()?;
    let mut frames = 0u32;
    let mut last_video = Instant::now();
    let mut last_audio = Instant::now();
    loop {
        if cancelled() { return Ok(()); }
        if watchdog.as_ref().is_some_and(|guard| guard.handle().expired()) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "native transport deadline expired"));
        }
        if Instant::now() >= next_sample {
            next_sample = Instant::now() + Duration::from_secs(1);
            if let Some(socket) = &telemetry {
                if let Ok(sample) = crate::tcp_telemetry::TcpSample::read(socket) {
                    eprintln!("native TCP rtt_us={} rttvar_us={} retrans_total={} unacked_segments={} send_queue_bytes={} not_sent_bytes={}",
                        sample.rtt_us, sample.rtt_variance_us, sample.retransmitted_segments,
                        sample.unacked_segments, sample.send_queue_bytes, sample.not_sent_bytes);
                    if adaptive {
                        if let Some(kbps) = rate.observe(crate::rate_control::Observation {
                            not_sent_bytes: sample.not_sent_bytes, rtt_us: sample.rtt_us,
                            retrans_total: sample.retransmitted_segments,
                        }) {
                            network_rate = kbps;
                        }
                        let class = pipeline.content_class();
                        let desired = if content_aware {
                            crate::content_analysis::content_rate(network_rate, class)
                        } else { network_rate };
                        if desired != applied_rate {
                            stage(pipeline.reconfigure_rate(desired, 500), "adaptive rate")?;
                            applied_rate = desired;
                            eprintln!("native adaptive video bitrate_kbps={desired} network_budget_kbps={network_rate} content={class:?}");
                        }
                    }
                } else { rate.missing_sample(); }
            }
        }
        let mut progress = false;
        for (block, _captured) in stage(audio.blocks(sources, &s.capture, origin), "audio pipeline")? {
            progress = true;
            last_audio = Instant::now();
            stage(send_audio(&mut pipeline, stage(aac.encode(&block), "AAC encode")?), "audio send")?;
        }
        if let Some(mut frame) = stage(sources.video(video_max_age(s.fps)), "video capture")? {
            progress = true;
            if frame.timestamp_us >= origin {
                last_video = Instant::now();
                frame.timestamp_us = frame.timestamp_us - origin + 100_000;
                #[cfg(feature = "ci-virtual")]
                virtual_phone_tick(frame.timestamp_us);
                stage(pipeline.push_frame_with(&frame, composition.as_mut()), "video encode/send")?;
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

fn publish_parallel(url: &str, s: &Settings, sources: &mut Sources, rate: &mut crate::rate_control::RateController, cancelled: &impl Fn() -> bool) -> io::Result<()> {
    let mut aac = AacBackend::new(s.audio_bitrate)?;
    let mut session = stage(rtmps_tls::begin_publish(url), "publish negotiation")?;
    // Opt-in until the independent network benchmark validates the budget.
    let watchdog = {
        let guard = crate::transport_deadline::DeadlineWatchdog::start_with_queue_budget(
            session.tcp_monitor()?, crate::transport_deadline::QueueBudget {
                max_not_sent_bytes: (rate.current() * 150 / 8).max(32_768),
                max_sustained: Duration::from_millis(500),
            })?;
        session.set_media_deadline(guard.handle(), Duration::from_millis(500));
        Some(guard)
    };
    let telemetry = session.tcp_monitor().ok();
    let mut next_sample = Instant::now();
    let adaptive = env::var("STEPHANIA_ADAPTIVE_RATE").as_deref() == Ok("1");
    let content_aware = adaptive && env::var("STEPHANIA_CONTENT_AWARE").as_deref() == Ok("1");
    let mut network_rate = if adaptive { rate.current() } else { s.video_bitrate };
    let mut applied_rate = network_rate;
    let mut encoder = NativeVideoEncoder::new(s.capture.width as usize,
        s.capture.height as usize, s.fps, network_rate)?;
    if adaptive { encoder.reconfigure_rate(network_rate, 500)?; }
    if content_aware { encoder.enable_content_analysis(); }
    let (sps,pps) = encoder.configuration()?;
    let socket = session.tcp_monitor()?;
    let mut sink = NativeMediaSink::new(session,&sps,&pps)?;
    sink.configure_audio(&aac.configuration()?)?;
    let writer = crate::media_worker::MediaWorker::start(sink,socket)?;
    let mut composition = crate::composition::enabled()
        .then(|| crate::composition::CompositionStage::new(s.capture.width as usize, s.capture.height as usize));
    let origin = timestamp_us();
    let mut audio = AudioPipeline::new()?;
    let mut frames = 0u32;
    let mut last_video = Instant::now();
    let mut last_audio = Instant::now();
    loop {
        if cancelled() { return Ok(()); }
        writer.check()?;
        if watchdog.as_ref().is_some_and(|guard| guard.handle().expired()) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "native transport deadline expired"));
        }
        if Instant::now() >= next_sample {
            next_sample = Instant::now() + Duration::from_secs(1);
            if let Some(socket) = &telemetry {
                if let Ok(sample) = crate::tcp_telemetry::TcpSample::read(socket) {
                    eprintln!("native TCP rtt_us={} rttvar_us={} retrans_total={} unacked_segments={} send_queue_bytes={} not_sent_bytes={}",
                        sample.rtt_us, sample.rtt_variance_us, sample.retransmitted_segments,
                        sample.unacked_segments, sample.send_queue_bytes, sample.not_sent_bytes);
                    if adaptive {
                        if let Some(kbps) = rate.observe(crate::rate_control::Observation {
                            not_sent_bytes: sample.not_sent_bytes, rtt_us: sample.rtt_us,
                            retrans_total: sample.retransmitted_segments,
                        }) {
                            network_rate = kbps;
                        }
                        let class = encoder.content_class();
                        let desired = if content_aware {
                            crate::content_analysis::content_rate(network_rate, class)
                        } else { network_rate };
                        if desired != applied_rate {
                            stage(encoder.reconfigure_rate(desired, 500), "adaptive rate")?;
                            applied_rate = desired;
                            eprintln!("native adaptive video bitrate_kbps={desired} network_budget_kbps={network_rate} content={class:?}");
                        }
                    }
                } else { rate.missing_sample(); }
            }
        }
        let mut progress = false;
        for (block, captured) in stage(audio.blocks(sources, &s.capture, origin), "audio pipeline")? {
            progress = true;
            last_audio = Instant::now();
            stage(writer.audio(stage(aac.encode(&block), "AAC encode")?, captured), "audio enqueue")?;
        }
        if let Some(mut frame) = stage(sources.video(video_max_age(s.fps)), "video capture")? {
            progress = true;
            if frame.timestamp_us >= origin {
                last_video = Instant::now();
                let captured = Instant::now().checked_sub(Duration::from_micros(timestamp_us().saturating_sub(frame.timestamp_us))).unwrap_or_else(Instant::now);
                frame.timestamp_us = frame.timestamp_us - origin + 100_000;
                #[cfg(feature = "ci-virtual")]
                virtual_phone_tick(frame.timestamp_us);
                if let Some(unit) = stage(encoder.encode_with(&frame, composition.as_mut()), "video encode")? {
                    stage(writer.video(unit,captured), "video enqueue")?;
                }
                frames = frames.saturating_add(1);
                if s.max_frames != 0 && frames >= s.max_frames { break; }
            }
        }
        if last_audio.elapsed() > Duration::from_secs(5) || last_video.elapsed() > Duration::from_secs(5) {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "capture stalled"));
        }
        if !progress { thread::sleep(Duration::from_millis(2)); }
    }
    stage(writer.audio(stage(aac.finish(), "AAC flush")?, Instant::now()), "audio flush enqueue")?;
    for unit in encoder.finish()? { writer.video(unit,Instant::now())?; }
    let count = stage(writer.finish(), "media drain")?;
    eprintln!("native A/V session finished: {count} video access units");
    Ok(())
}

pub fn run_from_env() -> io::Result<()> {
    let url = env::var("STEPHANIA_OUTPUT").map_err(|_| invalid("STEPHANIA_OUTPUT required"))?;
    rtmps_tls::Target::parse(&url)?;
    let mut capture = CaptureSettings::default();
    if let Ok(path) = env::var("STEPHANIA_VIDEO") { capture.video_path = path; }
    if let Ok(device) = env::var("STEPHANIA_AUDIO") { capture.audio_device = device; }
    capture.width = number("STEPHANIA_WIDTH", 1280)?;
    capture.height = number("STEPHANIA_HEIGHT", 720)?;
    capture.preferred_format = match env::var("STEPHANIA_PIXEL_FORMAT").as_deref() {
        Ok("nv12") => PixelFormat::Nv12,
        Ok("yuyv") | Err(_) => PixelFormat::Yuyv422,
        _ => return Err(invalid("unsupported pixel format")),
    };
    let s = Settings { capture, fps: number("STEPHANIA_FPS", 30)?,
        video_bitrate: number("STEPHANIA_BITRATE_KBPS", 3000)?,
        audio_bitrate: number("STEPHANIA_AUDIO_BITRATE_KBPS", 128)?,
        max_frames: number("STEPHANIA_MAX_FRAMES", 0)? };
    run_with_settings(&url, &s, || false)
}

/// Run the native publisher until completion or cooperative cancellation.
/// Network and device I/O already in progress may delay cancellation.
pub fn run_with_settings(url: &str, s: &Settings, cancelled: impl Fn() -> bool) -> io::Result<()> {
    rtmps_tls::Target::parse(url)?;
    if cancelled() { return Ok(()); }
    // Validate codecs before opening devices or attempting network retries.
    AacBackend::new(s.audio_bitrate)?;
    crate::native_x264::X264Backend::new(s.capture.width as usize,
        s.capture.height as usize, s.fps, s.video_bitrate, true)?;
    let mut sources = Sources::open(s)?;
    let _audio_running = AudioRunning::start();
    let mut failures = 0u32;
    let mut rate = crate::rate_control::RateController::new(s.video_bitrate);
    while !cancelled() {
        let started = Instant::now();
        let result = if env::var("STEPHANIA_PARALLEL_MEDIA").as_deref() == Ok("1") {
            publish_parallel(url, s, &mut sources, &mut rate, &cancelled)
        } else {
            publish(url, s, &mut sources, &mut rate, &cancelled)
        };
        match result {
            Ok(()) => return Ok(()),
            Err(e) if !reconnect::retryable(&e) => return Err(e),
            Err(e) => {
                if env::var("STEPHANIA_ADAPTIVE_RATE").as_deref() == Ok("1") {
                    rate.reconnect();
                }
                if started.elapsed() >= Duration::from_secs(60) { failures = 0; }
                failures = failures.saturating_add(1);
                let delay = reconnect::retry_delay(failures);
                // Do not print URLs or server responses containing stream keys.
                eprintln!("native A/V session failed ({:?}); reconnect in {}s", e.kind(), delay.as_secs());
                // Keep draining capture queues during backoff; never replay stale media.
                let until = Instant::now() + delay;
                while !cancelled() && Instant::now() < until {
                    let _ = sources.video(video_max_age(s.fps));
                    sources.drain_audio();
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }
    Ok(())
}
