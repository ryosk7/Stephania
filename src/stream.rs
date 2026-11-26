use std::env;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Config {
    pub video_device: String,
    pub audio_device: String,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub bitrate_kbps: u32,
    pub preset: String,
    pub output: String,
    pub test_sources: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let fps = number("T3BOX_FPS", 30, 1, 60)?;
        let width = number("T3BOX_WIDTH", 1920, 320, 3840)?;
        let height = number("T3BOX_HEIGHT", 1080, 240, 2160)?;
        let bitrate_kbps = number("T3BOX_BITRATE_KBPS", 8000, 500, 30000)?;
        let preset = env::var("T3BOX_PRESET").unwrap_or_else(|_| "ultrafast".into());
        if !["ultrafast", "superfast", "veryfast", "faster", "fast"].contains(&preset.as_str()) {
            return Err("unsupported T3BOX_PRESET".into());
        }
        let output = env::var("T3BOX_OUTPUT").unwrap_or_default();
        let test_sources = env::var("T3BOX_TEST_SOURCES").as_deref() == Ok("1");
        if !output.is_empty() && !output.starts_with("rtmps://") && !output.starts_with("file:") {
            return Err("T3BOX_OUTPUT must start with rtmps:// or file:".into());
        }
        Ok(Self {
            video_device: env::var("T3BOX_VIDEO").unwrap_or_else(|_| "/dev/video0".into()),
            audio_device: env::var("T3BOX_AUDIO").unwrap_or_else(|_| "plughw:1,0".into()),
            fps, width, height, bitrate_kbps, preset, output, test_sources,
        })
    }

    pub fn ffmpeg_args(&self) -> Vec<String> {
        let mut a = vec!["-hide_banner", "-stdin", "-loglevel", "error", "-y"]
            .into_iter().map(str::to_string).collect::<Vec<_>>();
        if self.test_sources {
            a.extend(["-re", "-f", "lavfi", "-i", &format!("testsrc2=size={}x{}:rate={}", self.width, self.height, self.fps)].map(str::to_string));
            a.extend(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"].map(str::to_string));
        } else {
            a.extend(["-thread_queue_size", "512", "-f", "v4l2",
                "-framerate", &self.fps.to_string(),
                "-video_size", &format!("{}x{}", self.width, self.height),
                "-i", &self.video_device].map(str::to_string));
            a.extend(["-thread_queue_size", "512", "-f", "alsa", "-i", &self.audio_device].map(str::to_string));
        }
        a.extend(["-map", "0:v:0", "-map", "1:a:0",
            "-c:v", "libx264", "-preset", &self.preset, "-tune", "zerolatency",
            "-pix_fmt", "yuv420p", "-b:v", &format!("{}k", self.bitrate_kbps),
            "-maxrate", &format!("{}k", self.bitrate_kbps),
            "-bufsize", &format!("{}k", self.bitrate_kbps * 2),
            "-g", &(self.fps * 2).to_string(),
            "-c:a", "aac", "-ar", "48000", "-b:a", "160k",
            "-af", "highpass=f=80,alimiter=limit=0.9,aresample=async=1000",
            "-f", "flv"].map(str::to_string));
        if let Some(path) = self.output.strip_prefix("file:") {
            a.push(path.to_string());
        } else {
            a.push(self.output.clone());
        }
        a
    }
}

fn number(key: &str, default: u32, min: u32, max: u32) -> Result<u32, String> {
    let value = env::var(key).ok().map(|x| x.parse::<u32>())
        .transpose().map_err(|_| format!("{key} must be a number"))?.unwrap_or(default);
    if value < min || value > max { return Err(format!("{key} out of range")); }
    Ok(value)
}

#[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
struct NativeWorker {
    cancel: Arc<std::sync::atomic::AtomicBool>,
    handle: thread::JoinHandle<std::io::Result<()>>,
}

// Ask FFmpeg to finish its current packet and write the muxer trailer.
// Bound the wait below the HTTP client's stop deadline; kill only if needed.
fn stop_child(child: &mut Child) -> Result<(), String> {
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"q\n");
    }
    let deadline = Instant::now() + Duration::from_millis(1500);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("ffmpeg stopped with {status}")),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) | Err(_) => break,
        }
    }
    let _ = child.kill();
    child.wait().map_err(|e| format!("ffmpeg reap failed: {e}"))?;
    Err("ffmpeg did not stop gracefully; output may be incomplete".into())
}

struct State {
    #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
    native: Option<NativeWorker>,
    child: Option<Child>,
    desired: bool,
    restarts: u64,
    last_error: String,
}

#[derive(Clone)]
pub struct Controller {
    state: Arc<Mutex<State>>,
    config: Config,
}

impl Controller {
    pub fn new(config: Config) -> Self {
        let ctrl = Self {
            state: Arc::new(Mutex::new(State {
                #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
                native: None,
                child: None, desired: false, restarts: 0, last_error: String::new()
            })),
            config,
        };
        let worker = ctrl.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(3));
            let mut s = worker.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(child) = s.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(exit)) => {
                        s.child = None;
                        s.last_error = format!("ffmpeg exited: {exit}");
                    }
                    Ok(None) => {}
                    Err(e) => { s.last_error = e.to_string(); s.child = None; }
                }
            }
            #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
            if let Some(worker) = s.native.take_if(|w| w.handle.is_finished()) {
                    s.last_error = match worker.handle.join() {
                        Ok(Ok(())) => String::new(),
                        Ok(Err(e)) => format!("native publisher stopped ({:?})", e.kind()),
                        Err(_) => "native publisher panicked".into(),
                    };
                    // A native worker handles transient retries itself.
                    s.desired = false;
            }
            if s.desired && !Self::running(&s) {
                // Only retry configured output; avoid runaway spawning on unconfigured devices.
                if let Err(e) = worker.spawn(&mut s) { s.last_error = e; }
                s.restarts += 1;
            }
        });
        ctrl
    }

    fn running(state: &State) -> bool {
        #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
        if state.native.is_some() { return true; }
        state.child.is_some()
    }

    fn spawn(&self, state: &mut State) -> Result<(), String> {
        if self.config.output.is_empty() { return Err("T3BOX_OUTPUT is not configured".into()); }
        if self.config.output == "file:" { return Err("file output path is empty".into()); }
        // Explicit native mode initially; preserve the file/synthetic FFmpeg
        // path and its existing CI contract until native file output exists.
        if env::var("T3BOX_BACKEND").as_deref() == Ok("native") {
            #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
            { return self.spawn_native(state); }
            #[cfg(not(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac")))]
            { return Err("native backend is not compiled in".into()); }
        }
        // Stream key in a command-line argument may be readable by local processes.
        // Use a dedicated unprivileged account and restrict local login access.
        let child = Command::new("ffmpeg")
            .args(self.config.ffmpeg_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn().map_err(|e| e.to_string())?;
        state.child = Some(child);
        state.last_error.clear();
        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
    fn spawn_native(&self, state: &mut State) -> Result<(), String> {
        use std::sync::atomic::{AtomicBool, Ordering};
        use t3box::{native_av, native_capture::CaptureSettings, media_core::PixelFormat};
        if self.config.test_sources || !self.config.output.starts_with("rtmps://") {
            return Err("native controller requires hardware input and RTMPS output".into());
        }
        if self.config.preset != "ultrafast" {
            return Err("native controller currently supports only ultrafast".into());
        }
        if !self.config.width.is_multiple_of(2) || !self.config.height.is_multiple_of(2) {
            return Err("native controller requires even dimensions".into());
        }
        t3box::rtmps_tls::Target::parse(&self.config.output).map_err(|e| e.to_string())?;
        let preferred_format = match env::var("T3BOX_PIXEL_FORMAT").as_deref() {
            Ok("nv12") => PixelFormat::Nv12,
            Ok("yuyv") | Err(_) => PixelFormat::Yuyv422,
            _ => return Err("unsupported native pixel format".into()),
        };
        let settings = native_av::Settings {
            capture: CaptureSettings {
                video_path: self.config.video_device.clone(),
                audio_device: self.config.audio_device.clone(),
                width: self.config.width, height: self.config.height, preferred_format,
                ..CaptureSettings::default()
            },
            fps: self.config.fps, video_bitrate: self.config.bitrate_kbps,
            audio_bitrate: number("T3BOX_AUDIO_BITRATE_KBPS", 160, 64, 320)?,
            max_frames: 0,
        };
        let url = self.config.output.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancel);
        let handle = thread::Builder::new().name("t3box-native-publisher".into())
            .spawn(move || native_av::run_with_settings(&url, &settings, || signal.load(Ordering::Acquire)))
            .map_err(|e| e.to_string())?;
        state.native = Some(NativeWorker { cancel, handle });
        state.last_error.clear();
        Ok(())
    }

    pub fn start(&self) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.desired { return Ok(()); }
        if Self::running(&s) { return Err("previous stream is still stopping".into()); }
        self.spawn(&mut s)?;
        s.desired = true;
        Ok(())
    }

    pub fn stop(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.desired = false;
        #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
        if let Some(worker) = &s.native {
            worker.cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        if let Some(mut child) = s.child.take() {
            if let Err(error) = stop_child(&mut child) { s.last_error = error; }
        }
    }

    pub fn native_running(&self) -> bool {
        #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
        {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.native.as_ref().is_some_and(|w| !w.handle.is_finished())
        }
        #[cfg(not(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac")))]
        { false }
    }

    pub fn status(&self) -> (bool, bool, u64, String) {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (s.desired, s.child.is_some(), s.restarts, s.last_error.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ffmpeg_args_are_structured() {
        let config = Config {
            video_device: "/dev/video2".into(), audio_device: "plughw:1,0".into(),
            fps: 30, width: 1280, height: 720, bitrate_kbps: 3000,
            preset: "ultrafast".into(), output: "file:/tmp/test.flv".into(), test_sources: true,
        };
        let a = config.ffmpeg_args();
        assert!(a.contains(&"testsrc2=size=1280x720:rate=30".to_string()));
        assert!(a.contains(&"sine=frequency=440:sample_rate=48000".to_string()));
        assert_eq!(a.last().unwrap(), "/tmp/test.flv");
        assert!(a.contains(&"-stdin".to_string()));
    }
    #[cfg(unix)]
    #[test]
    fn stop_requests_graceful_exit_before_killing() {
        let mut child = Command::new("sh")
            .args(["-c", "read -r line; test \"$line\" = q"])
            .stdin(Stdio::piped()).spawn().unwrap();
        assert!(stop_child(&mut child).is_ok());
        assert!(child.try_wait().unwrap().unwrap().success());
    }
    #[test]
    fn stop_does_not_start() {
        let config = Config {
            video_device: "".into(), audio_device: "".into(),
            fps: 30, width: 1280, height: 720, bitrate_kbps: 3000,
            preset: "ultrafast".into(), output: "".into(), test_sources: true,
        };
        let controller = Controller::new(config);
        controller.stop();
        assert!(!controller.status().0);
    }
}
