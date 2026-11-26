//! Live composition stage between capture conversion and the encoder
//! (docs/COMPOSITOR.md). Opt-in with `T3BOX_COMPOSITOR=1`; without it the
//! media loop never touches this module.
//!
//! - One process-wide [`ProgramSwitch`]: the IPC `scene.apply` handler
//!   requests a Program and waits for the media thread's frame-boundary
//!   acknowledgement. No ack (not streaming / timeout) means no success.
//! - [`SourceRegistry`] holds the freshest frame per non-HDMI source
//!   (`phone:rear`, `phone:front`), submitted by receivers (WebRTC later, a
//!   virtual source in CI). Frames older than [`SOURCE_MAX_AGE`] are treated
//!   as missing and drawn black, never frozen into Program.
//! - HDMI fast path: while the applied Program is "HDMI full screen" the
//!   captured frame goes to the encoder unchanged (no copy, no composition).

use crate::colorspace::I420Frame;
use crate::compositor::{ApplyAck, Compositor, FrameReport, Program, ProgramSwitch};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const SOURCE_MAX_AGE: Duration = Duration::from_millis(500);
pub const APPLY_TIMEOUT: Duration = Duration::from_secs(2);

static SWITCH: OnceLock<ProgramSwitch> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static SOURCES: OnceLock<SourceRegistry> = OnceLock::new();

pub fn enabled() -> bool {
    std::env::var("T3BOX_COMPOSITOR").as_deref() == Ok("1")
}

pub fn switch() -> &'static ProgramSwitch {
    SWITCH.get_or_init(|| ProgramSwitch::new(Program::hdmi_only()))
}

pub fn sources() -> &'static SourceRegistry {
    SOURCES.get_or_init(SourceRegistry::default)
}

/// True while a media loop with a composition stage is running.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// No composing media loop: nothing could acknowledge the Program.
    Inactive,
    /// The media loop did not reach a frame boundary in time.
    Timeout,
}

/// Request `program` and wait until a frame was produced with it.
pub fn apply(program: Program, timeout: Duration) -> Result<ApplyAck, ApplyError> {
    if !active() {
        return Err(ApplyError::Inactive);
    }
    let rx = switch().request(program);
    match rx.recv_timeout(timeout) {
        Ok(ack) => Ok(ack),
        Err(_) => {
            // Never let an unacknowledged request take effect later.
            switch().cancel_pending();
            // It may have been applied between the timeout and the cancel.
            rx.try_recv().map_err(|_| ApplyError::Timeout)
        }
    }
}

/// The Program currently on air: (scene_id, revision). Revision 0 = the
/// built-in HDMI-only Program.
pub fn current_program() -> (String, u64) {
    let p = switch().current();
    (p.scene_id.clone(), p.revision)
}

/// Freshest frame per source id.
#[derive(Default)]
pub struct SourceRegistry {
    frames: Mutex<HashMap<String, (I420Frame, Instant)>>,
}

impl SourceRegistry {
    pub fn submit(&self, source: &str, frame: I420Frame) {
        self.submit_at(source, frame, Instant::now());
    }

    pub fn submit_at(&self, source: &str, frame: I420Frame, at: Instant) {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).insert(source.to_owned(), (frame, at));
    }

    /// Clone the frames that are fresh at `now`.
    fn snapshot(&self, now: Instant) -> HashMap<String, I420Frame> {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).iter()
            .filter(|(_, (_, at))| now.duration_since(*at) <= SOURCE_MAX_AGE)
            .map(|(k, (f, _))| (k.clone(), f.clone()))
            .collect()
    }

    pub fn clear(&self) {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// Marks the composition loop active for its lifetime.
pub struct ActiveGuard(());

impl ActiveGuard {
    pub fn acquire() -> Self {
        ACTIVE.store(true, Ordering::Release);
        ActiveGuard(())
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

/// Per-media-loop state; lives on the encoder thread.
pub struct CompositionStage {
    compositor: Option<Compositor>,
    frame_index: u64,
    width: usize,
    height: usize,
    pub last_report: Option<FrameReport>,
    _active: ActiveGuard,
}

impl CompositionStage {
    pub fn new(width: usize, height: usize) -> Self {
        Self { compositor: None, frame_index: 0, width, height, last_report: None, _active: ActiveGuard::acquire() }
    }

    /// Decide this frame: `None` = encode `hdmi` as is (fast path);
    /// `Some(frame)` = encode the composed frame.
    pub fn process(&mut self, hdmi: &I420Frame, registry: &SourceRegistry) -> Option<&I420Frame> {
        let program = switch().at_frame_boundary(self.frame_index).for_output(self.width, self.height);
        self.frame_index += 1;
        if program.is_hdmi_passthrough() && hdmi.width == self.width && hdmi.height == self.height {
            self.last_report = None;
            return None;
        }
        let fresh = registry.snapshot(Instant::now());
        let compositor = self.compositor.get_or_insert_with(|| Compositor::new(self.width, self.height));
        let report = compositor.compose(&program, hdmi.timestamp_us, |source| {
            if source == "hdmi" { Some(hdmi) } else { fresh.get(source) }
        });
        self.last_report = Some(report);
        Some(compositor.output())
    }
}

/// Deterministic stand-in for a phone track (CI only): a solid colour frame
/// so composed output can be checked pixel-wise after H.264 round trip.
#[cfg(feature = "ci-virtual")]
pub fn virtual_phone_frame(timestamp_us: u64) -> I420Frame {
    let (w, h) = (320usize, 180usize);
    let n = w * h;
    let mut bytes = vec![VIRTUAL_PHONE_YUV.0; n];
    bytes.extend(std::iter::repeat_n(VIRTUAL_PHONE_YUV.1, n / 4));
    bytes.extend(std::iter::repeat_n(VIRTUAL_PHONE_YUV.2, n / 4));
    I420Frame { width: w, height: h, timestamp_us, bytes }
}

/// Saturated magenta-ish in limited-range YUV; far from the virtual HDMI
/// pattern and from black.
pub const VIRTUAL_PHONE_YUV: (u8, u8, u8) = (105, 212, 234);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn solid(w: usize, h: usize, y: u8) -> I420Frame {
        let n = w * h;
        let mut bytes = vec![y; n];
        bytes.extend(std::iter::repeat_n(128, n / 2));
        I420Frame { width: w, height: h, timestamp_us: 0, bytes }
    }

    // One test owns the process-wide switch to avoid cross-test interference.
    #[test]
    fn stage_fast_path_apply_ack_and_staleness() {
        let registry = SourceRegistry::default();
        assert_eq!(apply(Program::hdmi_only(), Duration::from_millis(10)), Err(ApplyError::Inactive));
        let mut stage = CompositionStage::new(64, 36);
        assert!(active());
        let hdmi = solid(64, 36, 50);
        // Default Program = HDMI full screen: fast path, no composition.
        assert!(stage.process(&hdmi, &registry).is_none());

        let scene = json!({"version":1,"name":"pip","layers":[
            {"id":"s","source":"hdmi","x":0,"y":0,"width":1920,"height":1080,"visible":true},
            {"id":"r","source":"phone:rear","x":960,"y":0,"width":960,"height":540,"visible":true}]});
        let program = Program::from_scene("pip", 4, &scene).unwrap();
        let waiter = std::thread::spawn(move || apply(program, Duration::from_secs(2)));
        // Media thread keeps producing frames until the ack is delivered.
        registry.submit("phone:rear", solid(32, 18, 200));
        let mut composed = None;
        for _ in 0..200 {
            if let Some(out) = stage.process(&hdmi, &registry) { composed = Some(out.clone()); break; }
            std::thread::sleep(Duration::from_millis(2));
        }
        let ack = waiter.join().unwrap().expect("acknowledged");
        assert_eq!((ack.scene_id.as_str(), ack.revision), ("pip", 4));
        let out = composed.expect("composed frame");
        assert_eq!(out.y()[2], 50, "HDMI area");
        assert_eq!(out.y()[40], 200, "phone area");

        // Phone goes stale: its region turns black instead of freezing.
        registry.submit_at("phone:rear", solid(32, 18, 200), Instant::now() - SOURCE_MAX_AGE * 2);
        let out = stage.process(&hdmi, &registry).unwrap().clone();
        assert_eq!(out.y()[40], crate::compositor::BLACK_Y);
        assert_eq!(stage.last_report.as_ref().unwrap().missing_sources, vec!["phone:rear".to_string()]);

        drop(stage);
        assert!(!active());
        assert_eq!(apply(Program::hdmi_only(), Duration::from_millis(10)), Err(ApplyError::Inactive));
        // Leave the global Program as HDMI-only for any later user.
        let _ = switch().request(Program::hdmi_only());
        switch().at_frame_boundary(0);
    }
}
