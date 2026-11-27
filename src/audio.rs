//! Audio inputs: configuration, live control and status (docs/AUDIO.md).
//!
//! Two inputs feed the program mix: `hdmi` (embedded HDMI audio from the
//! capture device) and `mic` (an optional USB microphone). This module is
//! the process-wide source of truth shared by the IPC server (configure,
//! status) and the media loop (which applies the configuration and reports
//! levels and input health). It has no media dependencies, so the control
//! surface works in every engine build.
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub const INPUTS: [&str; 2] = ["hdmi", "mic"];
pub const MIN_GAIN_DB: f64 = -60.0;
pub const MAX_GAIN_DB: f64 = 12.0;
/// Level reports cover this many recent 1024-sample blocks (~320 ms).
const LEVEL_WINDOW: usize = 15;
/// No input frame for this long ⇒ the input is reported as stalled.
pub const STALL_MS: u64 = 500;

#[derive(Debug, Clone, PartialEq)]
pub struct InputConfig {
    pub enabled: bool,
    /// ALSA device name (e.g. `plughw:CARD=Capture,DEV=0`).
    pub device: String,
    pub channels: u8,
    pub gain_db: f64,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputState {
    /// Not enabled in the configuration.
    Disabled,
    /// Enabled, no media loop running (not streaming).
    Idle,
    /// Enabled and opened; no frame yet.
    Waiting,
    /// Frames arriving.
    Active,
    /// Frames stopped arriving (unplugged, device stalled).
    Stalled,
    /// Capture error; the device is reopened automatically.
    Error(String),
}

impl InputState {
    fn name(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Idle => "idle",
            Self::Waiting => "waiting",
            Self::Active => "active",
            Self::Stalled => "stalled",
            Self::Error(_) => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Block {
    peak: f64,
    sum_squares: f64,
    samples: usize,
    clipped: bool,
}

#[derive(Debug)]
struct Live {
    state: InputState,
    last_frame: Option<Instant>,
    blocks: Vec<Block>,
}

impl Default for Live {
    fn default() -> Self { Self { state: InputState::Idle, last_frame: None, blocks: Vec::new() } }
}

#[derive(Debug)]
struct Inner {
    config: [InputConfig; 2],
    /// Increments whenever a device, channel count or enable flag changes;
    /// the media loop reopens inputs when it sees a new generation.
    generation: u64,
    live: [Live; 2],
    master: Vec<Block>,
    running: bool,
    resyncs: u64,
    limited_blocks: u64,
}

pub struct AudioControl {
    inner: Mutex<Inner>,
}

static CONTROL: OnceLock<AudioControl> = OnceLock::new();

/// Initial configuration: HDMI audio from `STEPHANIA_AUDIO` (the existing
/// capture setting), microphone from `STEPHANIA_AUDIO_MIC` when set. The control
/// plane then applies the stored configuration over IPC.
fn initial() -> [InputConfig; 2] {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mic = env("STEPHANIA_AUDIO_MIC");
    [
        InputConfig { enabled: true, device: env("STEPHANIA_AUDIO").unwrap_or_else(|| "plughw:1,0".into()), channels: 2, gain_db: 0.0, muted: false },
        InputConfig {
            enabled: mic.is_some(),
            device: mic.unwrap_or_default(),
            channels: env("STEPHANIA_AUDIO_MIC_CHANNELS").and_then(|c| c.parse().ok()).filter(|c| *c == 1 || *c == 2).unwrap_or(1),
            gain_db: 0.0,
            muted: false,
        },
    ]
}

pub fn control() -> &'static AudioControl {
    CONTROL.get_or_init(|| AudioControl::new(initial()))
}

/// ALSA device names: card/device syntax only, no shell or path tricks.
pub fn valid_device(name: &str) -> bool {
    (1..=96).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b",:=._-".contains(&b))
}

pub fn db_to_gain(db: f64) -> f64 { 10f64.powf(db / 20.0) }

fn dbfs(linear: f64) -> f64 {
    if linear <= 0.0 { -120.0 } else { (20.0 * (linear / 32768.0).log10()).max(-120.0) }
}

fn round1(x: f64) -> f64 { (x * 10.0).round() / 10.0 }

fn summarize(blocks: &[Block]) -> Value {
    let peak = blocks.iter().map(|b| b.peak).fold(0.0, f64::max);
    let (sq, n) = blocks.iter().fold((0.0, 0usize), |(s, n), b| (s + b.sum_squares, n + b.samples));
    let rms = if n == 0 { 0.0 } else { (sq / n as f64).sqrt() };
    json!({"peak_dbfs": round1(dbfs(peak)), "rms_dbfs": round1(dbfs(rms)), "clipped": blocks.iter().any(|b| b.clipped)})
}

pub fn measure(samples: &[i32]) -> (f64, f64, usize) {
    let mut peak = 0.0f64;
    let mut sq = 0.0f64;
    for &s in samples {
        let v = f64::from(s);
        peak = peak.max(v.abs());
        sq += v * v;
    }
    (peak, sq, samples.len())
}

impl AudioControl {
    pub fn new(config: [InputConfig; 2]) -> Self {
        Self { inner: Mutex::new(Inner {
            config, generation: 0, live: Default::default(), master: Vec::new(), running: false, resyncs: 0, limited_blocks: 0,
        }) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> { self.inner.lock().unwrap_or_else(|e| e.into_inner()) }

    pub fn config(&self) -> ([InputConfig; 2], u64) {
        let g = self.lock();
        (g.config.clone(), g.generation)
    }

    /// Validates and applies `{inputs:[{id, enabled?, device?, channels?,
    /// gain_db?, muted?}]}`. Omitted fields keep their value. Gain and mute
    /// apply immediately (ramped by the mixer); device, channel and enable
    /// changes reopen that input at the next media-loop tick.
    pub fn configure(&self, params: &Value) -> Result<Value, String> {
        let inputs = params.get("inputs").and_then(Value::as_array).ok_or("inputs must be an array")?;
        if inputs.is_empty() || inputs.len() > 2 { return Err("one or two inputs".into()); }
        let mut g = self.lock();
        let mut next = g.config.clone();
        let mut seen = [false; 2];
        for item in inputs {
            let obj = item.as_object().ok_or("each input must be an object")?;
            if let Some(k) = obj.keys().find(|k| !matches!(k.as_str(), "id" | "enabled" | "device" | "channels" | "gain_db" | "muted")) {
                return Err(format!("unknown field {k}"));
            }
            let id = obj.get("id").and_then(Value::as_str).ok_or("id required")?;
            let i = INPUTS.iter().position(|n| *n == id).ok_or("id must be hdmi or mic")?;
            if seen[i] { return Err(format!("duplicate input {id}")); }
            seen[i] = true;
            let c = &mut next[i];
            if let Some(v) = obj.get("enabled") { c.enabled = v.as_bool().ok_or("enabled must be a boolean")?; }
            if let Some(v) = obj.get("device") {
                let d = v.as_str().ok_or("device must be a string")?;
                if !(d.is_empty() || valid_device(d)) { return Err("device must be an ALSA name like plughw:CARD=Mic,DEV=0".into()); }
                c.device = d.to_owned();
            }
            if let Some(v) = obj.get("channels") {
                let ch = v.as_u64().ok_or("channels must be 1 or 2")?;
                if !(ch == 1 || ch == 2) { return Err("channels must be 1 or 2".into()); }
                c.channels = ch as u8;
            }
            if let Some(v) = obj.get("gain_db") {
                let db = v.as_f64().filter(|d| d.is_finite()).ok_or("gain_db must be a number")?;
                if !(MIN_GAIN_DB..=MAX_GAIN_DB).contains(&db) { return Err(format!("gain_db must be within {MIN_GAIN_DB}..{MAX_GAIN_DB}")); }
                c.gain_db = db;
            }
            if let Some(v) = obj.get("muted") { c.muted = v.as_bool().ok_or("muted must be a boolean")?; }
            if c.enabled && c.device.is_empty() { return Err(format!("{id}: an enabled input needs a device")); }
        }
        if next[0].enabled && next[1].enabled && next[0].device == next[1].device {
            return Err("hdmi and mic must use different devices".into());
        }
        let reopen = next.iter().zip(g.config.iter()).any(|(a, b)| a.enabled != b.enabled || a.device != b.device || a.channels != b.channels);
        if reopen { g.generation += 1; }
        g.config = next;
        drop(g);
        Ok(self.status())
    }

    // ---- media loop side ----

    pub fn set_running(&self, running: bool) {
        let mut g = self.lock();
        g.running = running;
        for (i, live) in g.live.iter_mut().enumerate() {
            live.blocks.clear();
            live.last_frame = None;
            live.state = InputState::Idle;
            let _ = i;
        }
        g.master.clear();
    }

    pub fn input_opened(&self, i: usize) {
        let mut g = self.lock();
        g.live[i].state = InputState::Waiting;
        g.live[i].last_frame = None;
        g.live[i].blocks.clear();
    }

    pub fn input_frame(&self, i: usize, now: Instant) {
        let mut g = self.lock();
        g.live[i].last_frame = Some(now);
        g.live[i].state = InputState::Active;
    }

    pub fn input_error(&self, i: usize, message: &str) {
        // Keep the reason short and free of device-internal detail.
        let short: String = message.chars().filter(|c| !c.is_control()).take(120).collect();
        self.lock().live[i].state = InputState::Error(short);
    }

    /// One mixed block: per-input post-gain levels and the master output.
    pub fn record_block(&self, inputs: [(f64, f64, usize); 2], master: (f64, f64, usize), clipped: bool, limited: bool) {
        let mut g = self.lock();
        for (live, (peak, sq, n)) in g.live.iter_mut().zip(inputs) {
            live.blocks.push(Block { peak, sum_squares: sq, samples: n, clipped: false });
            if live.blocks.len() > LEVEL_WINDOW { live.blocks.remove(0); }
        }
        g.master.push(Block { peak: master.0, sum_squares: master.1, samples: master.2, clipped });
        if g.master.len() > LEVEL_WINDOW { g.master.remove(0); }
        if limited { g.limited_blocks += 1; }
    }

    pub fn record_resync(&self) { self.lock().resyncs += 1; }

    /// `audio.status`: configuration, health and recent levels per input.
    pub fn status(&self) -> Value {
        let g = self.lock();
        let now = Instant::now();
        let inputs: Vec<Value> = INPUTS.iter().enumerate().map(|(i, id)| {
            let c = &g.config[i];
            let live = &g.live[i];
            let state = if !c.enabled { InputState::Disabled } else if !g.running { InputState::Idle } else {
                match &live.state {
                    InputState::Active if live.last_frame.is_some_and(|t| now.duration_since(t).as_millis() as u64 > STALL_MS) => InputState::Stalled,
                    s => s.clone(),
                }
            };
            let mut v = json!({"id": id, "enabled": c.enabled, "device": c.device, "channels": c.channels,
                               "gain_db": c.gain_db, "muted": c.muted, "state": state.name()});
            if let InputState::Error(m) = &state { v["error"] = json!(m); }
            if g.running && c.enabled { v["levels"] = summarize(&live.blocks); }
            v
        }).collect();
        let mut out = json!({"running": g.running, "inputs": inputs, "resyncs": g.resyncs, "limited_blocks": g.limited_blocks});
        if g.running { out["master"] = summarize(&g.master); }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl() -> AudioControl {
        AudioControl::new([
            InputConfig { enabled: true, device: "plughw:1,0".into(), channels: 2, gain_db: 0.0, muted: false },
            InputConfig { enabled: false, device: String::new(), channels: 1, gain_db: 0.0, muted: false },
        ])
    }

    #[test]
    fn configure_validates_and_only_reopens_on_device_changes() {
        let c = ctl();
        let (_, g0) = c.config();
        c.configure(&json!({"inputs": [{"id": "hdmi", "gain_db": -6.0, "muted": true}]})).unwrap();
        let (cfg, g1) = c.config();
        assert_eq!((cfg[0].gain_db, cfg[0].muted, g1), (-6.0, true, g0), "gain/mute are live, no reopen");
        c.configure(&json!({"inputs": [{"id": "mic", "enabled": true, "device": "plughw:CARD=Mic,DEV=0"}]})).unwrap();
        assert_eq!(c.config().1, g0 + 1);
        for bad in [
            json!({"inputs": [{"id": "aux"}]}),
            json!({"inputs": [{"id": "hdmi", "gain_db": 20}]}),
            json!({"inputs": [{"id": "hdmi", "device": "hw:0; rm -rf /"}]}),
            json!({"inputs": [{"id": "mic", "channels": 3}]}),
            json!({"inputs": [{"id": "mic", "device": "plughw:1,0"}]}),
            json!({"inputs": [{"id": "hdmi"}, {"id": "hdmi"}]}),
            json!({"inputs": [{"id": "hdmi", "volume": 1}]}),
            json!({"inputs": [{"id": "mic", "enabled": true, "device": ""}]}),
        ] {
            assert!(c.configure(&bad).is_err(), "{bad}");
        }
        assert_eq!(c.config().0[1].device, "plughw:CARD=Mic,DEV=0", "rejected changes are not applied");
    }

    #[test]
    fn status_reports_state_and_levels() {
        let c = ctl();
        assert_eq!(c.status()["inputs"][0]["state"], "idle");
        assert_eq!(c.status()["inputs"][1]["state"], "disabled");
        c.set_running(true);
        c.input_opened(0);
        assert_eq!(c.status()["inputs"][0]["state"], "waiting");
        c.input_frame(0, Instant::now());
        let half = 16384.0;
        c.record_block([(half, half * half * 2048.0, 2048), (0.0, 0.0, 2048)], (half, half * half * 2048.0, 2048), false, false);
        let s = c.status();
        assert_eq!(s["inputs"][0]["state"], "active");
        assert!((s["inputs"][0]["levels"]["peak_dbfs"].as_f64().unwrap() + 6.0).abs() < 0.2);
        assert!((s["master"]["rms_dbfs"].as_f64().unwrap() + 6.0).abs() < 0.2);
        c.input_frame(0, Instant::now() - std::time::Duration::from_millis(STALL_MS + 100));
        assert_eq!(c.status()["inputs"][0]["state"], "stalled");
        c.input_error(0, "No such device\n");
        assert_eq!(c.status()["inputs"][0]["error"], "No such device");
        c.set_running(false);
        assert_eq!(c.status()["inputs"][0]["state"], "idle");
        assert!(c.status().get("master").is_none());
    }
}
