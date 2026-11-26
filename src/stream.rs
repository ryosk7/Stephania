use std::env;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug)]
#[cfg_attr(not(any(test, feature = "ci-virtual", all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))), allow(dead_code))]
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
        if !output.is_empty() && !output.starts_with("rtmps://") {
            return Err("T3BOX_OUTPUT must start with rtmps://".into());
        }
        Ok(Self {
            video_device: env::var("T3BOX_VIDEO").unwrap_or_else(|_| "/dev/video0".into()),
            audio_device: env::var("T3BOX_AUDIO").unwrap_or_else(|_| "plughw:1,0".into()),
            fps, width, height, bitrate_kbps, preset, output, test_sources,
        })
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

struct State {
    #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
    native: Option<NativeWorker>,
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
                desired: false, restarts: 0, last_error: String::new()
            })),
            config,
        };
        let worker = ctrl.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(3));
            let mut s = worker.state.lock().unwrap_or_else(|e| e.into_inner());
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

    fn running(_state: &State) -> bool {
        #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
        { _state.native.is_some() }
        #[cfg(not(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac")))]
        { false }
    }

    fn spawn(&self, state: &mut State) -> Result<(), String> {
        if self.config.output.is_empty() { return Err("T3BOX_OUTPUT is not configured".into()); }
        if self.config.test_sources || !self.config.output.starts_with("rtmps://") {
            return Err("production requires hardware input and RTMPS output".into());
        }
        if self.config.preset != "ultrafast" || !self.config.width.is_multiple_of(2) || !self.config.height.is_multiple_of(2) {
            return Err("native requires ultrafast and even dimensions".into());
        }
        if env::var("T3BOX_BACKEND").is_ok_and(|v| v != "native") {
            return Err("only native production backend is supported".into());
        }
        #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
        { self.spawn_native(state) }
        #[cfg(not(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac")))]
        { let _ = state; Err("native backend is not compiled in".into()) }
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
        (s.desired, false, s.restarts, s.last_error.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_output_is_not_a_publisher() {
        let config = Config { video_device: "".into(), audio_device: "".into(), fps: 30,
            width: 1280, height: 720, bitrate_kbps: 3000, preset: "ultrafast".into(),
            output: "file:/tmp/test.flv".into(), test_sources: false };
        let controller = Controller::new(config);
        assert!(controller.start().is_err());
        assert!(!controller.status().0);
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
