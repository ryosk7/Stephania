//! Deterministic CPU reference compositor (docs/MASTER-ARCHITECTURE.md §22–23).
//!
//! Status: reference implementation and test oracle only. It is **not** wired
//! into the live media path, so `scene.apply` over IPC still fails closed.
//! Optimised (NEON/libyuv/GPU) compositors must match this output bit-exactly
//! on the golden tests before they may replace it.
//!
//! Model:
//! - Output and inputs are I420 (limited-range BT.709 assumed for every
//!   source; colour conversion happens before frames reach the compositor).
//! - Layers are drawn in array order (later layers on top), stretched to
//!   their rectangle with centre-sampled nearest-neighbour scaling.
//! - A visible layer whose source has no fresh frame is filled with black, so
//!   a stalled phone track can never freeze stale content into Program.
//! - [`ProgramSwitch`] holds the immutable applied Program; a new Program is
//!   validated up front and swapped only at a frame boundary, and the
//!   acknowledgement names the first frame that used it.

use crate::colorspace::I420Frame;
use serde_json::Value;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// Limited-range black.
pub const BLACK_Y: u8 = 16;
pub const BLACK_C: u8 = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub id: String,
    pub source: String,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// Immutable, validated scene ready for composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub scene_id: String,
    pub revision: u64,
    pub width: usize,
    pub height: usize,
    /// Visible layers only, bottom first.
    pub layers: Vec<Layer>,
}

impl Program {
    /// Black 1920x1080 Program used before any scene is applied.
    pub fn empty() -> Self {
        Self { scene_id: String::new(), revision: 0, width: 1920, height: 1080, layers: Vec::new() }
    }

    /// Build from a draft scene v1 document. Validation is delegated to the
    /// same validator used by IPC so a Program can never hold an invalid scene.
    pub fn from_scene(scene_id: &str, revision: u64, doc: &Value) -> Result<Self, crate::scene::SceneError> {
        crate::scene::validate_draft_v1(doc)?;
        let mut layers = Vec::new();
        for l in doc["layers"].as_array().into_iter().flatten() {
            if l["visible"] != Value::Bool(true) {
                continue;
            }
            let n = |k: &str| l[k].as_u64().unwrap_or_default() as usize;
            layers.push(Layer {
                id: l["id"].as_str().unwrap_or_default().to_owned(),
                source: l["source"].as_str().unwrap_or_default().to_owned(),
                x: n("x"), y: n("y"), width: n("width"), height: n("height"),
            });
        }
        Ok(Self {
            scene_id: scene_id.to_owned(),
            revision,
            width: crate::scene::CANVAS_WIDTH as usize,
            height: crate::scene::CANVAS_HEIGHT as usize,
            layers,
        })
    }
}

impl Program {
    /// Map scene coordinates (1920×1080 canvas) onto an encoder of
    /// `width`×`height`, rounding rectangles to even luma positions so chroma
    /// edges stay aligned and every layer keeps at least 2×2 pixels.
    pub fn for_output(&self, width: usize, height: usize) -> Program {
        let sx = |v: usize| (v * width / self.width) & !1;
        let sy = |v: usize| (v * height / self.height) & !1;
        let layers = self.layers.iter().filter_map(|l| {
            let x = sx(l.x).min(width.saturating_sub(2));
            let y = sy(l.y).min(height.saturating_sub(2));
            let w = (sx(l.x + l.width).max(x + 2)).min(width) - x;
            let h = (sy(l.y + l.height).max(y + 2)).min(height) - y;
            (w >= 2 && h >= 2).then(|| Layer { id: l.id.clone(), source: l.source.clone(), x, y, width: w, height: h })
        }).collect();
        Program { scene_id: self.scene_id.clone(), revision: self.revision, width, height, layers }
    }

    /// True when the Program is exactly "HDMI, full canvas": the encoder can
    /// take the captured frame directly (HDMI fast path, no composition).
    pub fn is_hdmi_passthrough(&self) -> bool {
        matches!(self.layers.as_slice(), [l] if l.source == "hdmi" && l.x == 0 && l.y == 0 && l.width == self.width && l.height == self.height)
    }

    /// The Program used before any scene is applied: HDMI full screen.
    pub fn hdmi_only() -> Program {
        Program {
            scene_id: String::new(), revision: 0, width: 1920, height: 1080,
            layers: vec![Layer { id: "hdmi".into(), source: "hdmi".into(), x: 0, y: 0, width: 1920, height: 1080 }],
        }
    }
}

/// Per-frame composition report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameReport {
    pub frame_index: u64,
    pub program_revision: u64,
    /// Layers drawn as black because their source had no frame.
    pub missing_sources: Vec<String>,
}

/// Reusable compositor with a preallocated output frame and scratch maps,
/// so steady-state composition performs no heap allocation.
pub struct Compositor {
    output: I420Frame,
    x_map: Vec<usize>,
    frame_index: u64,
}

fn plane_dims(width: usize, height: usize) -> (usize, usize) {
    (width / 2, height / 2)
}

impl Compositor {
    pub fn new(width: usize, height: usize) -> Self {
        assert!(width.is_multiple_of(2) && height.is_multiple_of(2) && width > 0 && height > 0, "I420 needs even dimensions");
        let n = width * height;
        let mut bytes = vec![BLACK_C; n + n / 2];
        bytes[..n].fill(BLACK_Y);
        Self { output: I420Frame { width, height, timestamp_us: 0, bytes }, x_map: Vec::with_capacity(width), frame_index: 0 }
    }

    pub fn output(&self) -> &I420Frame {
        &self.output
    }

    /// Compose one output frame. `source` returns the freshest frame for a
    /// source id, or `None` when it is missing or stale.
    pub fn compose<'a>(
        &mut self,
        program: &Program,
        timestamp_us: u64,
        mut source: impl FnMut(&str) -> Option<&'a I420Frame>,
    ) -> FrameReport {
        assert_eq!((program.width, program.height), (self.output.width, self.output.height), "program/output size mismatch");
        let (w, h) = (self.output.width, self.output.height);
        let n = w * h;
        self.output.bytes[..n].fill(BLACK_Y);
        self.output.bytes[n..].fill(BLACK_C);
        self.output.timestamp_us = timestamp_us;
        let mut report = FrameReport { frame_index: self.frame_index, program_revision: program.revision, missing_sources: Vec::new() };
        for layer in &program.layers {
            match source(&layer.source) {
                Some(frame) if frame.width >= 2 && frame.height >= 2 && frame.bytes.len() == frame.width * frame.height * 3 / 2 => {
                    self.draw(layer, frame)
                }
                _ => {
                    self.fill_black(layer);
                    report.missing_sources.push(layer.source.clone());
                }
            }
        }
        self.frame_index += 1;
        report
    }

    /// Chroma rectangle covering a luma rectangle (rounded outward).
    fn chroma_rect(layer: &Layer) -> (usize, usize, usize, usize) {
        let cx0 = layer.x / 2;
        let cy0 = layer.y / 2;
        let cx1 = (layer.x + layer.width).div_ceil(2);
        let cy1 = (layer.y + layer.height).div_ceil(2);
        (cx0, cy0, cx1, cy1)
    }

    fn fill_black(&mut self, layer: &Layer) {
        let (w, h) = (self.output.width, self.output.height);
        let (cw, ch) = plane_dims(w, h);
        let n = w * h;
        for row in layer.y..layer.y + layer.height {
            self.output.bytes[row * w + layer.x..row * w + layer.x + layer.width].fill(BLACK_Y);
        }
        let (cx0, cy0, cx1, cy1) = Self::chroma_rect(layer);
        for plane in 0..2 {
            let base = n + plane * cw * ch;
            for row in cy0..cy1 {
                self.output.bytes[base + row * cw + cx0..base + row * cw + cx1].fill(BLACK_C);
            }
        }
    }

    /// Centre-sampled nearest-neighbour index: dst `d` of `dn` -> src of `sn`.
    fn sample(d: usize, dn: usize, sn: usize) -> usize {
        ((2 * d + 1) * sn / (2 * dn)).min(sn - 1)
    }

    fn draw(&mut self, layer: &Layer, src: &I420Frame) {
        let (w, h) = (self.output.width, self.output.height);
        let n = w * h;
        let (sw, sh) = (src.width, src.height);
        // Luma.
        self.x_map.clear();
        self.x_map.extend((0..layer.width).map(|dx| Self::sample(dx, layer.width, sw)));
        let (src_y, dst) = (src.y(), &mut self.output.bytes);
        for dy in 0..layer.height {
            let sy = Self::sample(dy, layer.height, sh);
            let srow = &src_y[sy * sw..sy * sw + sw];
            let drow = &mut dst[(layer.y + dy) * w + layer.x..(layer.y + dy) * w + layer.x + layer.width];
            for (d, &sx) in drow.iter_mut().zip(self.x_map.iter()) {
                *d = srow[sx];
            }
        }
        // Chroma: map the chroma rectangle onto the source chroma plane.
        let (cw, ch) = plane_dims(w, h);
        let (scw, sch) = plane_dims(sw, sh);
        let (cx0, cy0, cx1, cy1) = Self::chroma_rect(layer);
        let (dcw, dch) = (cx1 - cx0, cy1 - cy0);
        self.x_map.clear();
        self.x_map.extend((0..dcw).map(|dx| Self::sample(dx, dcw, scw)));
        for (plane, src_plane) in [src.u(), src.v()].into_iter().enumerate() {
            let base = n + plane * cw * ch;
            for dy in 0..dch {
                let sy = Self::sample(dy, dch, sch);
                let srow = &src_plane[sy * scw..sy * scw + scw];
                let start = base + (cy0 + dy) * cw + cx0;
                for (d, &sx) in self.output.bytes[start..start + dcw].iter_mut().zip(self.x_map.iter()) {
                    *d = srow[sx];
                }
            }
        }
    }
}

/// Acknowledgement that a Program was actually used to produce a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyAck {
    pub scene_id: String,
    pub revision: u64,
    pub first_frame_index: u64,
}

/// Holds the applied Program and at most one pending replacement. The media
/// thread calls [`ProgramSwitch::at_frame_boundary`] once per output frame; the
/// control side calls [`ProgramSwitch::request`] and waits on the receiver.
/// A newer request supersedes an older pending one (whose sender is dropped,
/// so its waiter observes failure rather than a false success).
pub struct ProgramSwitch {
    current: Mutex<Arc<Program>>,
    pending: Mutex<Option<(Arc<Program>, mpsc::Sender<ApplyAck>)>>,
}

impl ProgramSwitch {
    pub fn new(initial: Program) -> Self {
        Self { current: Mutex::new(Arc::new(initial)), pending: Mutex::new(None) }
    }

    pub fn current(&self) -> Arc<Program> {
        Arc::clone(&self.current.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn request(&self, program: Program) -> mpsc::Receiver<ApplyAck> {
        let (tx, rx) = mpsc::channel();
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some((Arc::new(program), tx));
        rx
    }

    /// Drop a pending request that was not applied yet (its waiter sees failure).
    pub fn cancel_pending(&self) {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    /// Swap in a pending Program (if any) and return the Program to compose
    /// for frame `frame_index`. The ack is sent before composition begins, and
    /// names that frame; call it from the media thread only.
    pub fn at_frame_boundary(&self, frame_index: u64) -> Arc<Program> {
        let taken = self.pending.lock().unwrap_or_else(|e| e.into_inner()).take();
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((program, ack)) = taken {
            *current = Arc::clone(&program);
            let _ = ack.send(ApplyAck { scene_id: program.scene_id.clone(), revision: program.revision, first_frame_index: frame_index });
        }
        Arc::clone(&current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    fn solid(width: usize, height: usize, y: u8, u: u8, v: u8) -> I420Frame {
        let n = width * height;
        let mut bytes = vec![y; n];
        bytes.extend(std::iter::repeat_n(u, n / 4));
        bytes.extend(std::iter::repeat_n(v, n / 4));
        I420Frame { width, height, timestamp_us: 0, bytes }
    }

    /// Deterministic pattern exercising scaling in both axes.
    fn pattern(width: usize, height: usize, seed: u8) -> I420Frame {
        let n = width * height;
        let mut bytes = Vec::with_capacity(n * 3 / 2);
        for y in 0..height { for x in 0..width { bytes.push(((x * 7 + y * 13) as u8).wrapping_add(seed)); } }
        for y in 0..height / 2 { for x in 0..width / 2 { bytes.push(((x * 3 + y) as u8) ^ seed); } }
        for y in 0..height / 2 { for x in 0..width / 2 { bytes.push(((x + y * 5) as u8).wrapping_sub(seed)); } }
        I420Frame { width, height, timestamp_us: 0, bytes }
    }

    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
    }

    fn lt_scene() -> Value {
        json!({"version":1,"name":"LT","layers":[
            {"id":"slides","source":"hdmi","x":0,"y":0,"width":1440,"height":1080,"visible":true},
            {"id":"rear","source":"phone:rear","x":1440,"y":0,"width":480,"height":540,"visible":true},
            {"id":"front","source":"phone:front","x":1440,"y":540,"width":480,"height":540,"visible":true}]})
    }

    fn y_at(f: &I420Frame, x: usize, y: usize) -> u8 { f.y()[y * f.width + x] }
    fn u_at(f: &I420Frame, x: usize, y: usize) -> u8 { f.u()[(y / 2) * (f.width / 2) + x / 2] }

    #[test]
    fn three_input_layout_places_each_source_exactly() {
        let program = Program::from_scene("lt", 3, &lt_scene()).unwrap();
        let hdmi = solid(1920, 1080, 200, 60, 70);
        let rear = solid(1280, 720, 100, 90, 160);
        let front = solid(640, 360, 50, 140, 110);
        let mut c = Compositor::new(1920, 1080);
        let report = c.compose(&program, 42, |s| match s {
            "hdmi" => Some(&hdmi), "phone:rear" => Some(&rear), "phone:front" => Some(&front), _ => None,
        });
        assert!(report.missing_sources.is_empty());
        assert_eq!(report.program_revision, 3);
        let out = c.output();
        assert_eq!(out.timestamp_us, 42);
        for (x, y, expect_y, expect_u) in [(0, 0, 200, 60), (1439, 1079, 200, 60), (1440, 0, 100, 90), (1919, 539, 100, 90), (1440, 540, 50, 140), (1919, 1079, 50, 140)] {
            assert_eq!(y_at(out, x, y), expect_y, "Y at {x},{y}");
            assert_eq!(u_at(out, x, y), expect_u, "U at {x},{y}");
        }
    }

    #[test]
    fn missing_source_is_black_not_stale() {
        let program = Program::from_scene("lt", 1, &lt_scene()).unwrap();
        let hdmi = solid(1920, 1080, 200, 60, 70);
        let rear = solid(1280, 720, 100, 90, 160);
        let mut c = Compositor::new(1920, 1080);
        c.compose(&program, 0, |s| match s { "hdmi" => Some(&hdmi), "phone:rear" => Some(&rear), _ => None });
        // Next frame: rear disappears. Its area must not keep the old pixels.
        let report = c.compose(&program, 1, |s| (s == "hdmi").then_some(&hdmi));
        assert_eq!(report.missing_sources, vec!["phone:rear".to_string(), "phone:front".to_string()]);
        assert_eq!(report.frame_index, 1);
        let out = c.output();
        assert_eq!(y_at(out, 1500, 100), BLACK_Y);
        assert_eq!(u_at(out, 1500, 100), BLACK_C);
        assert_eq!(y_at(out, 100, 100), 200, "HDMI continues when phone tracks are lost");
    }

    #[test]
    fn later_layers_draw_on_top_and_hidden_layers_are_skipped() {
        let doc = json!({"version":1,"name":"o","layers":[
            {"id":"a","source":"hdmi","x":0,"y":0,"width":1920,"height":1080,"visible":true},
            {"id":"b","source":"phone:rear","x":100,"y":100,"width":200,"height":200,"visible":true},
            {"id":"c","source":"phone:front","x":0,"y":0,"width":1920,"height":1080,"visible":false}]});
        let program = Program::from_scene("o", 1, &doc).unwrap();
        assert_eq!(program.layers.len(), 2);
        let (a, b) = (solid(64, 64, 10, 20, 30), solid(64, 64, 220, 230, 240));
        let mut c = Compositor::new(1920, 1080);
        c.compose(&program, 0, |s| match s { "hdmi" => Some(&a), "phone:rear" => Some(&b), _ => None });
        assert_eq!(y_at(c.output(), 150, 150), 220);
        assert_eq!(y_at(c.output(), 99, 150), 10);
        assert_eq!(y_at(c.output(), 300, 150), 10);
    }

    #[test]
    fn odd_geometry_stays_in_bounds_and_covers_chroma() {
        let doc = json!({"version":1,"name":"odd","layers":[
            {"id":"a","source":"hdmi","x":1,"y":1,"width":1919,"height":1079,"visible":true},
            {"id":"b","source":"phone:rear","x":1917,"y":1077,"width":3,"height":3,"visible":true}]});
        let program = Program::from_scene("odd", 1, &doc).unwrap();
        let (a, b) = (solid(2, 2, 90, 91, 92), solid(1920, 1080, 30, 31, 32));
        let mut c = Compositor::new(1920, 1080);
        c.compose(&program, 0, |s| match s { "hdmi" => Some(&a), "phone:rear" => Some(&b), _ => None });
        assert_eq!(y_at(c.output(), 0, 0), BLACK_Y);
        assert_eq!(y_at(c.output(), 1, 1), 90);
        assert_eq!(y_at(c.output(), 1919, 1079), 30);
        assert_eq!(u_at(c.output(), 1919, 1079), 31);
    }

    #[test]
    fn scaling_is_centre_sampled() {
        assert_eq!((0..4).map(|d| Compositor::sample(d, 4, 2)).collect::<Vec<_>>(), vec![0, 0, 1, 1]);
        assert_eq!((0..2).map(|d| Compositor::sample(d, 2, 4)).collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(Compositor::sample(0, 1, 1920), 960);
        assert_eq!(Compositor::sample(1919, 1920, 1920), 1919);
    }

    /// Golden regression for the reference output. Any optimised compositor
    /// must reproduce these hashes. Update only with a reviewed, intentional
    /// change to the reference algorithm.
    #[test]
    fn golden_three_input_pattern() {
        let program = Program::from_scene("lt", 1, &lt_scene()).unwrap();
        let (hdmi, rear, front) = (pattern(1920, 1080, 1), pattern(1280, 720, 2), pattern(720, 1280, 3));
        let mut c = Compositor::new(1920, 1080);
        c.compose(&program, 0, |s| match s {
            "hdmi" => Some(&hdmi), "phone:rear" => Some(&rear), "phone:front" => Some(&front), _ => None,
        });
        let out = c.output();
        assert_eq!(fnv1a(out.y()), GOLDEN_Y, "Y plane changed: {:#018x}", fnv1a(out.y()));
        assert_eq!(fnv1a(out.u()), GOLDEN_U, "U plane changed: {:#018x}", fnv1a(out.u()));
        assert_eq!(fnv1a(out.v()), GOLDEN_V, "V plane changed: {:#018x}", fnv1a(out.v()));
    }

    const GOLDEN_Y: u64 = 0xc3d04b5bd372e8e5;
    const GOLDEN_U: u64 = 0x9c10b89d51eea885;
    const GOLDEN_V: u64 = 0x6253dc2dae1ed6e5;

    #[test]
    fn compose_is_deterministic_and_does_not_grow_buffers() {
        let program = Program::from_scene("lt", 1, &lt_scene()).unwrap();
        let hdmi = pattern(1920, 1080, 9);
        let mut c = Compositor::new(1920, 1080);
        c.compose(&program, 0, |s| (s == "hdmi").then_some(&hdmi));
        let first = c.output().bytes.clone();
        let (cap_out, cap_map) = (c.output.bytes.capacity(), c.x_map.capacity());
        for i in 1..5 { c.compose(&program, i, |s| (s == "hdmi").then_some(&hdmi)); }
        assert_eq!(c.output().bytes, first);
        assert_eq!((c.output.bytes.capacity(), c.x_map.capacity()), (cap_out, cap_map));
    }

    #[test]
    fn for_output_scales_and_detects_passthrough() {
        let p = Program::from_scene("lt", 1, &lt_scene()).unwrap().for_output(640, 360);
        assert_eq!((p.width, p.height), (640, 360));
        assert_eq!(p.layers[0], Layer { id: "slides".into(), source: "hdmi".into(), x: 0, y: 0, width: 480, height: 360 });
        assert_eq!(p.layers[1], Layer { id: "rear".into(), source: "phone:rear".into(), x: 480, y: 0, width: 160, height: 180 });
        assert!(!p.is_hdmi_passthrough());
        assert!(Program::hdmi_only().for_output(640, 360).is_hdmi_passthrough());
        assert!(Program::hdmi_only().is_hdmi_passthrough());
        // Tiny layers survive as at least 2x2 and stay in bounds.
        let doc = json!({"version":1,"name":"t","layers":[{"id":"a","source":"phone:front","x":1919,"y":1079,"width":1,"height":1,"visible":true}]});
        let t = Program::from_scene("t", 1, &doc).unwrap().for_output(640, 360);
        let l = &t.layers[0];
        assert!(l.x + l.width <= 640 && l.y + l.height <= 360 && l.width >= 2 && l.height >= 2, "{l:?}");
        let mut c = Compositor::new(640, 360);
        c.compose(&t, 0, |_| None);
    }

    #[test]
    fn program_rejects_invalid_scene() {
        assert!(Program::from_scene("x", 1, &json!({"version":2})).is_err());
    }

    #[test]
    fn switch_acknowledges_only_at_frame_boundary() {
        let switch = ProgramSwitch::new(Program::empty());
        let next = Program::from_scene("lt", 7, &lt_scene()).unwrap();
        let ack = switch.request(next.clone());
        // Not applied until the media thread reaches a boundary.
        assert!(ack.recv_timeout(Duration::from_millis(10)).is_err());
        assert_eq!(switch.current().revision, 0);
        let used = switch.at_frame_boundary(120);
        assert_eq!(*used, next);
        assert_eq!(ack.recv().unwrap(), ApplyAck { scene_id: "lt".into(), revision: 7, first_frame_index: 120 });
        assert_eq!(switch.at_frame_boundary(121).revision, 7);
    }

    #[test]
    fn superseded_request_never_reports_success() {
        let switch = ProgramSwitch::new(Program::empty());
        let first = switch.request(Program::from_scene("a", 1, &lt_scene()).unwrap());
        let second = switch.request(Program::from_scene("b", 2, &lt_scene()).unwrap());
        switch.at_frame_boundary(5);
        assert!(first.recv().is_err(), "superseded waiter must see failure");
        assert_eq!(second.recv().unwrap().revision, 2);
    }

    #[test]
    fn switch_from_media_thread() {
        let switch = Arc::new(ProgramSwitch::new(Program::empty()));
        let media = {
            let switch = Arc::clone(&switch);
            std::thread::spawn(move || {
                let mut revisions = Vec::new();
                for frame in 0..2000u64 {
                    revisions.push(switch.at_frame_boundary(frame).revision);
                    std::thread::sleep(Duration::from_micros(200));
                }
                revisions
            })
        };
        let ack = switch.request(Program::from_scene("lt", 9, &lt_scene()).unwrap());
        let got = ack.recv_timeout(Duration::from_secs(5)).expect("acked");
        assert_eq!(got.revision, 9);
        assert!(media.join().unwrap().contains(&9));
    }
}
