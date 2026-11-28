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
        let fps = number("STEPHANIA_FPS", 30, 1, 60)?;
        let width = number("STEPHANIA_WIDTH", 1920, 320, 3840)?;
        let height = number("STEPHANIA_HEIGHT", 1080, 240, 2160)?;
        let bitrate_kbps = number("STEPHANIA_BITRATE_KBPS", 8000, 500, 30000)?;
        let preset = env::var("STEPHANIA_PRESET").unwrap_or_else(|_| "ultrafast".into());
        if !["ultrafast", "superfast", "veryfast", "faster", "fast"].contains(&preset.as_str()) {
            return Err("unsupported STEPHANIA_PRESET".into());
        }
        let output = env::var("STEPHANIA_OUTPUT").unwrap_or_default();
        let test_sources = env::var("STEPHANIA_TEST_SOURCES").as_deref() == Ok("1");
        if !output.is_empty() && !output.starts_with("rtmps://") {
            return Err("STEPHANIA_OUTPUT must start with rtmps://".into());
        }
        Ok(Self {
            video_device: env::var("STEPHANIA_VIDEO").unwrap_or_else(|_| "/dev/video0".into()),
            audio_device: env::var("STEPHANIA_AUDIO").unwrap_or_else(|_| "plughw:1,0".into()),
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
    /// Wall-clock time the worker was spawned: what "on air for 12:04" counts from.
    started: std::time::SystemTime,
}

struct State {
    #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
    native: Option<NativeWorker>,
    desired: bool,
    restarts: u64,
    last_error: String,
    /// Destination of the current start request: the output control sent
    /// with `stream.start` (stored on /data by control), else
    /// `STEPHANIA_OUTPUT`. Contains the stream key: never log or report it.
    output: String,
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
                desired: false, restarts: 0, last_error: String::new(), output: String::new()
            })),
            config,
        };
        let worker = ctrl.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(3));
            let mut s = worker.state.lock().unwrap_or_else(|e| e.into_inner());
            #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
            if let Some(worker) = s.native.take_if(|w| w.handle.is_finished()) {
                    let panicked;
                    (s.last_error, panicked) = match worker.handle.join() {
                        Ok(Ok(())) => (String::new(), false),
                        Ok(Err(e)) => (format!("native publisher stopped ({:?})", e.kind()), false),
                        Err(_) => ("native publisher panicked".into(), true),
                    };
                    // A native worker handles transient retries itself and
                    // returns an error only for a final (configuration or
                    // authentication) failure. A panic is not a decision to
                    // stop: keep `desired` so the worker is respawned below.
                    if !panicked { s.desired = false; }
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
        if state.output.is_empty() { return Err("no stream output is configured (Studio settings, or STEPHANIA_OUTPUT for development)".into()); }
        if self.config.test_sources || !state.output.starts_with("rtmps://") {
            return Err("production requires hardware input and RTMPS output".into());
        }
        if self.config.preset != "ultrafast" || !self.config.width.is_multiple_of(2) || !self.config.height.is_multiple_of(2) {
            return Err("native requires ultrafast and even dimensions".into());
        }
        if env::var("STEPHANIA_BACKEND").is_ok_and(|v| v != "native") {
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
        use stephania::{native_av, native_capture::CaptureSettings, media_core::PixelFormat};
        if self.config.test_sources || !state.output.starts_with("rtmps://") {
            return Err("native controller requires hardware input and RTMPS output".into());
        }
        if self.config.preset != "ultrafast" {
            return Err("native controller currently supports only ultrafast".into());
        }
        if !self.config.width.is_multiple_of(2) || !self.config.height.is_multiple_of(2) {
            return Err("native controller requires even dimensions".into());
        }
        stephania::rtmps_tls::Target::parse(&state.output).map_err(|e| e.to_string())?;
        let preferred_format = match env::var("STEPHANIA_PIXEL_FORMAT").as_deref() {
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
            audio_bitrate: number("STEPHANIA_AUDIO_BITRATE_KBPS", 160, 64, 320)?,
            max_frames: 0,
        };
        let url = state.output.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancel);
        let handle = thread::Builder::new().name("stephania-native-publisher".into())
            .spawn(move || native_av::run_with_settings(&url, &settings, || signal.load(Ordering::Acquire)))
            .map_err(|e| e.to_string())?;
        state.native = Some(NativeWorker { cancel, handle, started: std::time::SystemTime::now() });
        state.last_error.clear();
        Ok(())
    }

    /// Start streaming to `output` (an RTMPS URL including the stream key)
    /// or, when None, to `STEPHANIA_OUTPUT` (development). The destination
    /// is fixed for this start, including automatic retries; a new one
    /// applies at the next start after a stop.
    pub fn start(&self, output: Option<&str>) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.desired { return Ok(()); }
        if Self::running(&s) { return Err("previous stream is still stopping".into()); }
        s.output = output.map_or_else(|| self.config.output.clone(), str::to_owned);
        self.spawn(&mut s)?;
        s.desired = true;
        Ok(())
    }

    pub fn stop(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.desired = false;
        s.output.clear();
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

    /// When the running native worker started; None while none runs.
    pub fn native_started(&self) -> Option<std::time::SystemTime> {
        #[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
        {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.native.as_ref().filter(|w| !w.handle.is_finished()).map(|w| w.started)
        }
        #[cfg(not(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac")))]
        { None }
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
        assert!(controller.start(None).is_err());
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
    #[test]
    fn output_from_control_overrides_env_and_is_never_echoed() {
        // Test sources stop before any network I/O, right after the output
        // check, so the error tells which destination was used.
        let config = Config {
            video_device: "".into(), audio_device: "".into(),
            fps: 30, width: 1280, height: 720, bitrate_kbps: 3000,
            preset: "ultrafast".into(), output: "".into(), test_sources: true,
        };
        let controller = Controller::new(config);
        let unset = controller.start(None).unwrap_err();
        assert!(unset.contains("no stream output is configured"), "{unset}");
        let key = "SECRET-KEY-1234";
        let given = controller.start(Some(&format!("rtmps://a.rtmps.youtube.com/live2/{key}"))).unwrap_err();
        assert!(given.contains("hardware input"), "{given}");
        assert!(!given.contains(key));
        assert!(!controller.status().3.contains(key));
        // The env value is the fallback when control sends none.
        let env_config = Config { output: format!("rtmps://a.rtmps.youtube.com/live2/{key}"), ..controller.config.clone() };
        let err = Controller::new(env_config).start(None).unwrap_err();
        assert!(err.contains("hardware input"), "{err}");
    }
}
