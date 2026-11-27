//! Deterministic CPU reference compositor (docs/COMPOSITOR.md, docs/SCENE.md).
//!
//! Optimised (NEON/libyuv/GPU) compositors must match this output bit-exactly
//! on the golden tests before they may replace it.
//!
//! Model:
//! - Output and inputs are I420 (limited-range BT.709 assumed for every
//!   source; colour conversion happens before frames reach the compositor).
//! - Layers are drawn in array order (later layers on top) over the canvas
//!   background.
//! - Plain video layers (stretch, no crop, no rotation, opaque — every scene
//!   v1 layer) take the original centre-sampled nearest-neighbour path, so v1
//!   output is unchanged bit for bit.
//! - Everything else goes through one integer-only mapping (Q8 pixel
//!   positions, Q16 sine table): rotation about the layer centre, fit
//!   (stretch/contain/cover), crop, then nearest-neighbour sampling and
//!   alpha blending. No floating point, so x86 and Arm produce identical
//!   frames.
//! - Image and colour layers are static: they are rendered once per output
//!   size into an [`Overlay`] (Y/U/V plus alpha) and only blended per frame.
//! - A visible video layer whose source has no fresh frame is filled with
//!   black, so a stalled phone track can never freeze stale content.
//! - [`ProgramSwitch`] holds the immutable applied Program; a new Program is
//!   validated up front and swapped only at a frame boundary (optionally
//!   with a timed fade), and the acknowledgement names the first frame that
//!   used it.

use crate::colorspace::I420Frame;
use serde_json::Value;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// Limited-range black.
pub const BLACK_Y: u8 = 16;
pub const BLACK_C: u8 = 128;
pub const BLACK: [u8; 3] = [BLACK_Y, BLACK_C, BLACK_C];
/// Longest accepted fade.
pub const MAX_FADE_MS: u64 = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    #[default]
    Stretch,
    Contain,
    Cover,
}

/// Crop in 1/10000 of the source size per side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Crop {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Crop {
    pub fn is_none(&self) -> bool {
        *self == Crop::default()
    }
}

/// Decoded straight-alpha RGBA8 image.
#[derive(Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for Rgba {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rgba({}x{})", self.width, self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Video { source: String, fit: Fit, crop: Crop },
    Image { asset: String, fit: Fit, image: Arc<Rgba> },
    Color { yuv: [u8; 3] },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub id: String,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    /// 0..=100 percent.
    pub opacity: u8,
    /// Degrees, -180..=180, clockwise on screen.
    pub rotation: i32,
    pub content: Content,
}

impl Layer {
    pub fn video(id: &str, source: &str, x: usize, y: usize, width: usize, height: usize) -> Self {
        Layer { id: id.into(), x, y, width, height, opacity: 100, rotation: 0,
                content: Content::Video { source: source.into(), fit: Fit::Stretch, crop: Crop::default() } }
    }

    /// Source id for video layers.
    pub fn source(&self) -> Option<&str> {
        match &self.content { Content::Video { source, .. } => Some(source), _ => None }
    }

    /// The original v1 path applies: opaque, unrotated, stretched, uncropped video.
    fn is_plain_video(&self) -> bool {
        matches!(&self.content, Content::Video { fit: Fit::Stretch, crop, .. } if crop.is_none()) && self.opacity == 100 && self.rotation == 0
    }
}

/// BT.709 limited-range RGB → YCbCr, integer only.
pub fn rgb_to_yuv(r: u8, g: u8, b: u8) -> [u8; 3] {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let y = 16 + ((47 * r + 157 * g + 16 * b + 128) >> 8);
    let u = 128 + ((-26 * r - 87 * g + 112 * b + 128) >> 8);
    let v = 128 + ((112 * r - 102 * g - 10 * b + 128) >> 8);
    [y.clamp(16, 235) as u8, u.clamp(16, 240) as u8, v.clamp(16, 240) as u8]
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 { return None; }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// sin(d°) in Q16 for d = 0..=90.
const SIN_Q16: [i64; 91] = [0, 1144, 2287, 3430, 4572, 5712, 6850, 7987, 9121, 10252, 11380, 12505, 13626, 14742, 15855, 16962, 18064, 19161,
    20252, 21336, 22415, 23486, 24550, 25607, 26656, 27697, 28729, 29753, 30767, 31772, 32768, 33754, 34729, 35693, 36647, 37590, 38521,
    39441, 40348, 41243, 42126, 42995, 43852, 44695, 45525, 46341, 47143, 47930, 48703, 49461, 50203, 50931, 51643, 52339, 53020, 53684,
    54332, 54963, 55578, 56175, 56756, 57319, 57865, 58393, 58903, 59396, 59870, 60326, 60764, 61183, 61584, 61966, 62328, 62672, 62997,
    63303, 63589, 63856, 64104, 64332, 64540, 64729, 64898, 65048, 65177, 65287, 65376, 65446, 65496, 65526, 65536];

/// (sin, cos) in Q16 for integer degrees.
pub fn sin_cos_q16(deg: i32) -> (i64, i64) {
    let d = deg.rem_euclid(360);
    let sin = |d: i32| -> i64 {
        match d {
            0..=90 => SIN_Q16[d as usize],
            91..=180 => SIN_Q16[(180 - d) as usize],
            181..=270 => -SIN_Q16[(d - 180) as usize],
            _ => -SIN_Q16[(360 - d) as usize],
        }
    };
    (sin(d), sin((d + 90).rem_euclid(360)))
}

/// Integer mapping from output pixels to content pixels for one layer.
/// Positions are Q8 (1/256 pixel).
#[derive(Debug, Clone, Copy)]
struct Mapping {
    cx: i64,
    cy: i64,
    lw: i64,
    lh: i64,
    sin: i64,
    cos: i64,
    /// Displayed content rectangle inside the layer (layer-local Q8).
    ox: i64,
    oy: i64,
    dw: i64,
    dh: i64,
    /// Content source window (content pixels Q8) after crop.
    sx0: i64,
    sy0: i64,
    sw: i64,
    sh: i64,
    content_w: usize,
    content_h: usize,
}

impl Mapping {
    fn new(layer: &Layer, content_w: usize, content_h: usize, fit: Fit, crop: Crop) -> Mapping {
        let (lw, lh) = (layer.width as i64 * 256, layer.height as i64 * 256);
        let (cw, ch) = (content_w as i64, content_h as i64);
        let sx0 = cw * 256 * i64::from(crop.left) / 10_000;
        let sy0 = ch * 256 * i64::from(crop.top) / 10_000;
        let sw = (cw * 256 * i64::from(10_000 - crop.left - crop.right) / 10_000).max(1);
        let sh = (ch * 256 * i64::from(10_000 - crop.top - crop.bottom) / 10_000).max(1);
        let (dw, dh) = match fit {
            Fit::Stretch => (lw, lh),
            Fit::Contain | Fit::Cover => {
                // Width-limited when lw/lh <= sw/sh (contain) or >= (cover).
                let width_limited = (lw * sh <= lh * sw) == (fit == Fit::Contain);
                if width_limited { (lw, (lw * sh / sw).max(1)) } else { ((lh * sw / sh).max(1), lh) }
            }
        };
        let (sin, cos) = sin_cos_q16(layer.rotation);
        Mapping {
            cx: layer.x as i64 * 256 + lw / 2, cy: layer.y as i64 * 256 + lh / 2, lw, lh, sin, cos,
            ox: (lw - dw) / 2, oy: (lh - dh) / 2, dw, dh, sx0, sy0, sw, sh, content_w, content_h,
        }
    }

    /// Content pixel for an output point given in Q8, or None when the point
    /// is outside the layer or outside the displayed content.
    fn map(&self, px: i64, py: i64) -> Option<(usize, usize)> {
        let (dx, dy) = (px - self.cx, py - self.cy);
        // Inverse rotation (screen rotation is clockwise for positive angles).
        let lx = ((dx * self.cos + dy * self.sin) >> 16) + self.lw / 2;
        let ly = ((dy * self.cos - dx * self.sin) >> 16) + self.lh / 2;
        if lx < 0 || ly < 0 || lx >= self.lw || ly >= self.lh {
            return None;
        }
        let (u, v) = (lx - self.ox, ly - self.oy);
        if u < 0 || v < 0 || u >= self.dw || v >= self.dh {
            return None;
        }
        let sx = ((self.sx0 + u * self.sw / self.dw) >> 8) as usize;
        let sy = ((self.sy0 + v * self.sh / self.dh) >> 8) as usize;
        Some((sx.min(self.content_w - 1), sy.min(self.content_h - 1)))
    }

    /// Output bounding box (x0, y0, x1, y1) of the rotated layer, clipped.
    fn bbox(&self, out_w: usize, out_h: usize) -> (usize, usize, usize, usize) {
        let (s, c) = (self.sin.abs(), self.cos.abs());
        let ex = ((self.lw * c + self.lh * s) >> 17) + 256;
        let ey = ((self.lw * s + self.lh * c) >> 17) + 256;
        let x0 = ((self.cx - ex) >> 8).max(0) as usize;
        let y0 = ((self.cy - ey) >> 8).max(0) as usize;
        let x1 = (((self.cx + ex + 255) >> 8) as usize).min(out_w);
        let y1 = (((self.cy + ey + 255) >> 8) as usize).min(out_h);
        (x0.min(x1), y0.min(y1), x1, y1)
    }
}

/// Pre-rendered static layer: luma rectangle (x, y, w, h) with alpha, and
/// the covering chroma rectangle with its own alpha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    luma: Vec<u8>,
    alpha: Vec<u8>,
    cx: usize,
    cy: usize,
    cw: usize,
    ch: usize,
    u: Vec<u8>,
    v: Vec<u8>,
    calpha: Vec<u8>,
}

fn blend(dst: u8, src: u8, a: u8) -> u8 {
    if a == 255 { return src; }
    if a == 0 { return dst; }
    ((u32::from(src) * u32::from(a) + u32::from(dst) * (255 - u32::from(a)) + 127) / 255) as u8
}

fn opacity_alpha(opacity: u8) -> u32 {
    u32::from(opacity.min(100)) * 255 / 100
}

impl Overlay {
    /// Render a static layer at output size. `sample(x, y)` returns
    /// straight RGBA for a content pixel (images) — colour layers pass a
    /// 1×1 content.
    fn render(layer: &Layer, out_w: usize, out_h: usize, content_w: usize, content_h: usize, fit: Fit,
              sample: impl Fn(usize, usize) -> ([u8; 3], u8)) -> Overlay {
        let m = Mapping::new(layer, content_w, content_h, fit, Crop::default());
        let op = opacity_alpha(layer.opacity);
        let (x0, y0, x1, y1) = m.bbox(out_w, out_h);
        let (w, h) = (x1 - x0, y1 - y0);
        let mut luma = vec![0; w * h];
        let mut alpha = vec![0; w * h];
        for row in 0..h {
            for col in 0..w {
                let p = ((x0 + col) as i64 * 256 + 128, (y0 + row) as i64 * 256 + 128);
                if let Some((sx, sy)) = m.map(p.0, p.1) {
                    let (yuv, a) = sample(sx, sy);
                    luma[row * w + col] = yuv[0];
                    alpha[row * w + col] = (u32::from(a) * op / 255) as u8;
                }
            }
        }
        let (cx0, cy0, cx1, cy1) = (x0 / 2, y0 / 2, x1.div_ceil(2).min(out_w / 2), y1.div_ceil(2).min(out_h / 2));
        let (cw, ch) = (cx1.saturating_sub(cx0), cy1.saturating_sub(cy0));
        let mut u = vec![BLACK_C; cw * ch];
        let mut v = vec![BLACK_C; cw * ch];
        let mut calpha = vec![0; cw * ch];
        for row in 0..ch {
            for col in 0..cw {
                // Chroma sample centre in luma Q8 coordinates.
                let p = ((2 * (cx0 + col) + 1) as i64 * 256, (2 * (cy0 + row) + 1) as i64 * 256);
                if let Some((sx, sy)) = m.map(p.0, p.1) {
                    let (yuv, a) = sample(sx, sy);
                    u[row * cw + col] = yuv[1];
                    v[row * cw + col] = yuv[2];
                    calpha[row * cw + col] = (u32::from(a) * op / 255) as u8;
                }
            }
        }
        Overlay { x: x0, y: y0, w, h, luma, alpha, cx: cx0, cy: cy0, cw, ch, u, v, calpha }
    }

    fn image(layer: &Layer, image: &Rgba, fit: Fit, out_w: usize, out_h: usize) -> Overlay {
        Overlay::render(layer, out_w, out_h, image.width, image.height, fit, |x, y| {
            let i = (y * image.width + x) * 4;
            let p = &image.pixels[i..i + 4];
            (rgb_to_yuv(p[0], p[1], p[2]), p[3])
        })
    }

    fn color(layer: &Layer, yuv: [u8; 3], out_w: usize, out_h: usize) -> Overlay {
        Overlay::render(layer, out_w, out_h, 1, 1, Fit::Stretch, |_, _| (yuv, 255))
    }

    fn blend_into(&self, out: &mut I420Frame) {
        let (w, h) = (out.width, out.height);
        let n = w * h;
        let (cw, ch) = (w / 2, h / 2);
        for row in 0..self.h {
            let d = (self.y + row) * w + self.x;
            for col in 0..self.w {
                let a = self.alpha[row * self.w + col];
                out.bytes[d + col] = blend(out.bytes[d + col], self.luma[row * self.w + col], a);
            }
        }
        for (plane, src) in [&self.u, &self.v].into_iter().enumerate() {
            let base = n + plane * cw * ch;
            for row in 0..self.ch {
                let d = base + (self.cy + row) * cw + self.cx;
                for col in 0..self.cw {
                    let a = self.calpha[row * self.cw + col];
                    out.bytes[d + col] = blend(out.bytes[d + col], src[row * self.cw + col], a);
                }
            }
        }
    }
}

/// Loads a decoded image asset by id (crate::assets).
pub type AssetLoader<'a> = &'a dyn Fn(&str) -> Result<Arc<Rgba>, String>;

/// Immutable, validated scene ready for composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub scene_id: String,
    pub revision: u64,
    pub width: usize,
    pub height: usize,
    pub background: [u8; 3],
    /// Visible layers only, bottom first, in output coordinates.
    pub layers: Vec<Layer>,
    /// The same layers in canvas (1920×1080) coordinates; output scaling
    /// always starts from these so it never compounds.
    canvas_layers: Arc<Vec<Layer>>,
    /// Pre-rendered static layers (image/colour), aligned with `layers`.
    overlays: Vec<Option<Arc<Overlay>>>,
    /// Set by [`Program::for_output`]: geometry and overlays are final for
    /// this output size.
    prepared: bool,
}

impl Program {
    fn new(scene_id: &str, revision: u64, background: [u8; 3], layers: Vec<Layer>) -> Program {
        let mut p = Program {
            scene_id: scene_id.to_owned(), revision, width: crate::scene::CANVAS_WIDTH as usize, height: crate::scene::CANVAS_HEIGHT as usize,
            background, canvas_layers: Arc::new(layers.clone()), layers, overlays: Vec::new(), prepared: false,
        };
        p.render_overlays();
        p
    }

    fn render_overlays(&mut self) {
        let (w, h) = (self.width, self.height);
        self.overlays = self.layers.iter().map(|l| match &l.content {
            Content::Image { image, fit, .. } => Some(Arc::new(Overlay::image(l, image, *fit, w, h))),
            Content::Color { yuv } => Some(Arc::new(Overlay::color(l, *yuv, w, h))),
            Content::Video { .. } => None,
        }).collect();
    }

    /// Black 1920x1080 Program used before any scene is applied.
    pub fn empty() -> Self {
        Program::new("", 0, BLACK, Vec::new())
    }

    /// Build from a scene document without image assets (image layers fail).
    pub fn from_scene(scene_id: &str, revision: u64, doc: &Value) -> Result<Self, crate::scene::SceneError> {
        Program::from_scene_with(scene_id, revision, doc, &|_| Err("no asset store".into()))
    }

    /// Build from a scene document (v1 or v2). Validation is delegated to the
    /// same validator used by IPC so a Program can never hold an invalid
    /// scene; a missing or undecodable asset fails the whole Program.
    pub fn from_scene_with(scene_id: &str, revision: u64, doc: &Value, assets: AssetLoader) -> Result<Self, crate::scene::SceneError> {
        crate::scene::validate(doc)?;
        let v2 = doc["version"] == 2;
        let background = doc["canvas"]["background"].as_str().and_then(parse_hex).map(|c| rgb_to_yuv(c[0], c[1], c[2])).unwrap_or(BLACK);
        let mut layers = Vec::new();
        for (i, l) in doc["layers"].as_array().into_iter().flatten().enumerate() {
            if l["visible"] != Value::Bool(true) {
                continue;
            }
            let n = |k: &str| l[k].as_u64().unwrap_or_default() as usize;
            let id = l["id"].as_str().unwrap_or_default().to_owned();
            let fit = match l["fit"].as_str() { Some("contain") => Fit::Contain, Some("cover") => Fit::Cover, _ => Fit::Stretch };
            let content = match (v2, l["type"].as_str()) {
                (true, Some("image")) => {
                    let asset = l["asset"].as_str().unwrap_or_default().to_owned();
                    let image = assets(&asset).map_err(|_| crate::scene::SceneError { path: format!("/layers/{i}/asset"), reason: "asset_unavailable" })?;
                    Content::Image { asset, fit, image }
                }
                (true, Some("color")) => {
                    let c = l["color"].as_str().and_then(parse_hex).unwrap_or([0, 0, 0]);
                    Content::Color { yuv: rgb_to_yuv(c[0], c[1], c[2]) }
                }
                _ => {
                    let c = |k: &str| l["crop"][k].as_u64().unwrap_or_default() as u32;
                    Content::Video { source: l["source"].as_str().unwrap_or_default().to_owned(), fit,
                                     crop: Crop { left: c("left"), top: c("top"), right: c("right"), bottom: c("bottom") } }
                }
            };
            layers.push(Layer {
                id, x: n("x"), y: n("y"), width: n("width"), height: n("height"),
                opacity: l["opacity"].as_u64().map_or(100, |o| o.min(100) as u8),
                rotation: l["rotation"].as_i64().map_or(0, |r| r.clamp(-180, 180) as i32),
                content,
            });
        }
        Ok(Program::new(scene_id, revision, background, layers))
    }

    /// Map scene coordinates (1920×1080 canvas) onto an encoder of
    /// `width`×`height`, rounding rectangles to even luma positions so chroma
    /// edges stay aligned and every layer keeps at least 2×2 pixels. Static
    /// layers are re-rendered at the new size.
    pub fn for_output(&self, width: usize, height: usize) -> Program {
        let (cw, ch) = (crate::scene::CANVAS_WIDTH as usize, crate::scene::CANVAS_HEIGHT as usize);
        if self.prepared && (width, height) == (self.width, self.height) {
            return self.clone();
        }
        let sx = |v: usize| (v * width / cw) & !1;
        let sy = |v: usize| (v * height / ch) & !1;
        let layers = self.canvas_layers.iter().filter_map(|l| {
            let x = sx(l.x).min(width.saturating_sub(2));
            let y = sy(l.y).min(height.saturating_sub(2));
            let w = (sx(l.x + l.width).max(x + 2)).min(width) - x;
            let h = (sy(l.y + l.height).max(y + 2)).min(height) - y;
            (w >= 2 && h >= 2).then(|| Layer { x, y, width: w, height: h, ..l.clone() })
        }).collect();
        let mut p = Program {
            scene_id: self.scene_id.clone(), revision: self.revision, width, height, background: self.background,
            layers, canvas_layers: Arc::clone(&self.canvas_layers), overlays: Vec::new(), prepared: true,
        };
        p.render_overlays();
        p
    }

    /// Geometry and overlays are already final for `width`×`height`.
    pub fn is_prepared_for(&self, width: usize, height: usize) -> bool {
        self.prepared && (self.width, self.height) == (width, height)
    }

    /// True when the Program is exactly "HDMI, full canvas": the encoder can
    /// take the captured frame directly (HDMI fast path, no composition).
    pub fn is_hdmi_passthrough(&self) -> bool {
        matches!(self.layers.as_slice(), [l] if l.source() == Some("hdmi") && l.is_plain_video() && l.x == 0 && l.y == 0 && l.width == self.width && l.height == self.height)
    }

    /// The Program used before any scene is applied: HDMI full screen.
    pub fn hdmi_only() -> Program {
        Program::new("", 0, BLACK, vec![Layer::video("hdmi", "hdmi", 0, 0, 1920, 1080)])
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

    pub fn output_mut(&mut self) -> &mut I420Frame {
        &mut self.output
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
        self.output.bytes[..n].fill(program.background[0]);
        let (cw, ch) = plane_dims(w, h);
        self.output.bytes[n..n + cw * ch].fill(program.background[1]);
        self.output.bytes[n + cw * ch..].fill(program.background[2]);
        self.output.timestamp_us = timestamp_us;
        let mut report = FrameReport { frame_index: self.frame_index, program_revision: program.revision, missing_sources: Vec::new() };
        for (i, layer) in program.layers.iter().enumerate() {
            if let Some(Some(overlay)) = program.overlays.get(i) {
                overlay.blend_into(&mut self.output);
                continue;
            }
            let Content::Video { source: id, fit, crop } = &layer.content else { continue };
            match source(id) {
                Some(frame) if frame.width >= 2 && frame.height >= 2 && frame.bytes.len() == frame.width * frame.height * 3 / 2 => {
                    if layer.is_plain_video() { self.draw(layer, frame) } else { self.draw_mapped(layer, frame, *fit, *crop) }
                }
                _ => {
                    if layer.is_plain_video() { self.fill_black(layer) } else { self.fill_mapped(layer, BLACK) }
                    report.missing_sources.push(id.clone());
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

    /// Generic path: rotation/fit/crop/opacity through [`Mapping`].
    fn draw_mapped(&mut self, layer: &Layer, src: &I420Frame, fit: Fit, crop: Crop) {
        let m = Mapping::new(layer, src.width, src.height, fit, crop);
        let a = opacity_alpha(layer.opacity) as u8;
        let (w, h) = (self.output.width, self.output.height);
        let (x0, y0, x1, y1) = m.bbox(w, h);
        let sw = src.width;
        let src_y = src.y();
        for py in y0..y1 {
            for px in x0..x1 {
                if let Some((sx, sy)) = m.map(px as i64 * 256 + 128, py as i64 * 256 + 128) {
                    let d = py * w + px;
                    self.output.bytes[d] = blend(self.output.bytes[d], src_y[sy * sw + sx], a);
                }
            }
        }
        let n = w * h;
        let (cw, ch) = plane_dims(w, h);
        let scw = src.width / 2;
        let (cx0, cy0, cx1, cy1) = (x0 / 2, y0 / 2, x1.div_ceil(2).min(cw), y1.div_ceil(2).min(ch));
        for (plane, sp) in [src.u(), src.v()].into_iter().enumerate() {
            let base = n + plane * cw * ch;
            for cy in cy0..cy1 {
                for cx in cx0..cx1 {
                    if let Some((sx, sy)) = m.map((2 * cx + 1) as i64 * 256, (2 * cy + 1) as i64 * 256) {
                        let d = base + cy * cw + cx;
                        let s = sp[(sy / 2).min(src.height / 2 - 1) * scw + (sx / 2).min(scw - 1)];
                        self.output.bytes[d] = blend(self.output.bytes[d], s, a);
                    }
                }
            }
        }
    }

    /// Fill a (possibly rotated) layer area with a constant colour.
    fn fill_mapped(&mut self, layer: &Layer, yuv: [u8; 3]) {
        let overlay = Overlay::color(layer, yuv, self.output.width, self.output.height);
        overlay.blend_into(&mut self.output);
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

/// Mix `from` into `out`: out = out·mix + from·(255 − mix), per plane.
pub fn crossfade(out: &mut I420Frame, from: &I420Frame, mix: u8) {
    for (d, &s) in out.bytes.iter_mut().zip(from.bytes.iter()) {
        *d = blend(s, *d, mix);
    }
}

/// How a new Program replaces the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transition {
    #[default]
    Cut,
    /// Cross-fade over the given duration (frame timestamps).
    Fade { duration_us: u64 },
}

/// Acknowledgement that a Program was actually used to produce a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyAck {
    pub scene_id: String,
    pub revision: u64,
    pub first_frame_index: u64,
}

/// What to compose for one frame: the Program, and during a fade the
/// outgoing Program with the incoming weight (0..=255).
#[derive(Debug, Clone)]
pub struct FramePlan {
    pub program: Arc<Program>,
    pub fade_from: Option<(Arc<Program>, u8)>,
}

type Pending = (Arc<Program>, Transition, mpsc::Sender<ApplyAck>);

struct Fade {
    from: Arc<Program>,
    start_us: Option<u64>,
    duration_us: u64,
}

/// Holds the applied Program and at most one pending replacement. The media
/// thread calls [`ProgramSwitch::at_frame`] once per output frame; the
/// control side calls [`ProgramSwitch::request`] and waits on the receiver.
/// A newer request supersedes an older pending one (whose sender is dropped,
/// so its waiter observes failure rather than a false success).
pub struct ProgramSwitch {
    current: Mutex<Arc<Program>>,
    pending: Mutex<Option<Pending>>,
    fade: Mutex<Option<Fade>>,
}

impl ProgramSwitch {
    pub fn new(initial: Program) -> Self {
        Self { current: Mutex::new(Arc::new(initial)), pending: Mutex::new(None), fade: Mutex::new(None) }
    }

    pub fn current(&self) -> Arc<Program> {
        Arc::clone(&self.current.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn request(&self, program: Program) -> mpsc::Receiver<ApplyAck> {
        self.request_with(program, Transition::Cut)
    }

    pub fn request_with(&self, program: Program, transition: Transition) -> mpsc::Receiver<ApplyAck> {
        let (tx, rx) = mpsc::channel();
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some((Arc::new(program), transition, tx));
        rx
    }

    /// Drop a pending request that was not applied yet (its waiter sees failure).
    pub fn cancel_pending(&self) {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    /// Program for frame `frame_index` (no fade information).
    pub fn at_frame_boundary(&self, frame_index: u64) -> Arc<Program> {
        self.at_frame(frame_index, 0).program
    }

    /// Swap in a pending Program (if any) and return what to compose for
    /// frame `frame_index` with timestamp `timestamp_us`. The ack is sent
    /// before composition begins and names that frame — for a fade, the first
    /// frame that shows any of the new Program. Media thread only.
    pub fn at_frame(&self, frame_index: u64, timestamp_us: u64) -> FramePlan {
        let taken = self.pending.lock().unwrap_or_else(|e| e.into_inner()).take();
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let mut fade = self.fade.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((program, transition, ack)) = taken {
            let previous = std::mem::replace(&mut *current, Arc::clone(&program));
            *fade = match transition {
                Transition::Fade { duration_us } if duration_us > 0 => Some(Fade { from: previous, start_us: None, duration_us }),
                _ => None,
            };
            let _ = ack.send(ApplyAck { scene_id: program.scene_id.clone(), revision: program.revision, first_frame_index: frame_index });
        }
        let fade_from = match fade.as_mut() {
            Some(f) => {
                let start = *f.start_us.get_or_insert(timestamp_us);
                let elapsed = timestamp_us.saturating_sub(start);
                if elapsed >= f.duration_us {
                    *fade = None;
                    None
                } else {
                    // At least 1 so the first frame already shows the new Program.
                    let mix = ((elapsed * 255 / f.duration_us) as u8).max(1);
                    Some((Arc::clone(&f.from), mix))
                }
            }
            None => None,
        };
        FramePlan { program: Arc::clone(&current), fade_from }
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
        assert_eq!(p.layers[0], Layer::video("slides", "hdmi", 0, 0, 480, 360));
        assert_eq!(p.layers[1], Layer::video("rear", "phone:rear", 480, 0, 160, 180));
        assert!(p.is_prepared_for(640, 360));
        // Scaling always starts from canvas coordinates: no compounding.
        assert_eq!(p.for_output(1920, 1080).for_output(640, 360), p);
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
        assert!(Program::from_scene("x", 1, &json!({"version":3, "name":"a", "layers":[]})).is_err());
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

    fn v2(layers: Value) -> Value {
        json!({"version": 2, "name": "v2", "layers": layers})
    }

    fn compose_one(doc: &Value, sources: &[(&str, &I420Frame)]) -> I420Frame {
        let program = Program::from_scene_with("v2", 1, doc, &|id| Err(id.to_owned())).unwrap();
        let mut c = Compositor::new(1920, 1080);
        c.compose(&program, 0, |s| sources.iter().find(|(id, _)| *id == s).map(|(_, f)| *f));
        c.output().clone()
    }

    #[test]
    fn v2_plain_video_matches_v1_golden() {
        let mut doc = lt_scene();
        doc["version"] = json!(2);
        for l in doc["layers"].as_array_mut().unwrap() { l["type"] = json!("video"); }
        let (hdmi, rear, front) = (pattern(1920, 1080, 1), pattern(1280, 720, 2), pattern(720, 1280, 3));
        let out = compose_one(&doc, &[("hdmi", &hdmi), ("phone:rear", &rear), ("phone:front", &front)]);
        assert_eq!((fnv1a(out.y()), fnv1a(out.u()), fnv1a(out.v())), (GOLDEN_Y, GOLDEN_U, GOLDEN_V));
    }

    #[test]
    fn opacity_blends_over_lower_layers() {
        let doc = v2(json!([
            {"id":"a","type":"video","source":"hdmi","x":0,"y":0,"width":1920,"height":1080,"visible":true},
            {"id":"b","type":"video","source":"phone:rear","x":0,"y":0,"width":960,"height":540,"visible":true,"opacity":50}]));
        let (a, b) = (solid(64, 64, 200, 60, 70), solid(64, 64, 100, 160, 170));
        let out = compose_one(&doc, &[("hdmi", &a), ("phone:rear", &b)]);
        // a = 50% → 127/255.
        assert_eq!(y_at(&out, 10, 10), ((100 * 127 + 200 * 128 + 127) / 255) as u8);
        assert_eq!(u_at(&out, 10, 10), ((160 * 127 + 60 * 128 + 127) / 255) as u8);
        assert_eq!(y_at(&out, 1000, 600), 200);
    }

    #[test]
    fn rotation_turns_the_layer_about_its_centre() {
        // A 400×100 bar rotated 90° becomes a 100×400 bar around (960, 540).
        let doc = v2(json!([{"id":"b","type":"video","source":"hdmi","x":760,"y":490,"width":400,"height":100,"visible":true,"rotation":90}]));
        let src = solid(64, 16, 180, 128, 128);
        let out = compose_one(&doc, &[("hdmi", &src)]);
        assert_eq!(y_at(&out, 960, 540), 180);
        assert_eq!(y_at(&out, 960, 345), 180, "inside after rotation");
        assert_eq!(y_at(&out, 960, 735), 180);
        assert_eq!(y_at(&out, 800, 540), BLACK_Y, "outside after rotation");
        assert_eq!(y_at(&out, 960, 330), BLACK_Y);
        // −90 and 270-equivalent give the same footprint; 180 keeps it.
        let (s, c) = sin_cos_q16(90);
        assert_eq!((s, c), (65536, 0));
        assert_eq!(sin_cos_q16(-90), (-65536, 0));
        assert_eq!(sin_cos_q16(180), (0, -65536));
        assert_eq!(sin_cos_q16(-180), (0, -65536));
    }

    #[test]
    fn rotation_maps_content_orientation() {
        // Left half dark, right half bright; rotated 90° clockwise the bright
        // half must end up at the bottom.
        let mut src = solid(64, 64, 40, 128, 128);
        for y in 0..64 { for x in 32..64 { src.bytes[y * 64 + x] = 220; } }
        let doc = v2(json!([{"id":"b","type":"video","source":"hdmi","x":760,"y":340,"width":400,"height":400,"visible":true,"rotation":90}]));
        let out = compose_one(&doc, &[("hdmi", &src)]);
        assert_eq!(y_at(&out, 960, 400), 40, "top after rotation = original left");
        assert_eq!(y_at(&out, 960, 700), 220, "bottom after rotation = original right");
    }

    #[test]
    fn contain_letterboxes_and_cover_fills() {
        let wide = solid(160, 90, 150, 100, 100);
        let contain = v2(json!([{"id":"c","type":"video","source":"hdmi","x":0,"y":0,"width":1080,"height":1080,"visible":true,"fit":"contain"}]));
        let out = compose_one(&contain, &[("hdmi", &wide)]);
        // 16:9 in a square: 1080×607.5 band in the middle, background above/below.
        assert_eq!(y_at(&out, 540, 540), 150);
        assert_eq!(y_at(&out, 540, 100), BLACK_Y);
        assert_eq!(y_at(&out, 540, 1000), BLACK_Y);
        let cover = v2(json!([{"id":"c","type":"video","source":"hdmi","x":0,"y":0,"width":1080,"height":1080,"visible":true,"fit":"cover"}]));
        let out = compose_one(&cover, &[("hdmi", &wide)]);
        assert_eq!(y_at(&out, 540, 100), 150);
        assert_eq!(y_at(&out, 0, 0), 150);
        assert_eq!(y_at(&out, 1079, 1079), 150);
        assert_eq!(y_at(&out, 1200, 500), BLACK_Y, "outside the layer");
    }

    #[test]
    fn crop_selects_part_of_the_source() {
        let mut src = solid(64, 64, 40, 128, 128);
        for y in 0..64 { for x in 32..64 { src.bytes[y * 64 + x] = 220; } }
        let doc = v2(json!([{"id":"c","type":"video","source":"hdmi","x":0,"y":0,"width":1920,"height":1080,"visible":true,
            "crop":{"left":5000,"top":0,"right":0,"bottom":0}}]));
        let out = compose_one(&doc, &[("hdmi", &src)]);
        assert_eq!(y_at(&out, 0, 500), 220, "left edge now shows the right half");
        assert_eq!(y_at(&out, 1919, 500), 220);
    }

    #[test]
    fn image_and_colour_layers_with_alpha_and_background() {
        // 2×1 image: opaque red, fully transparent.
        let img = Arc::new(Rgba { width: 2, height: 1, pixels: vec![255, 0, 0, 255, 0, 255, 0, 0] });
        let doc = json!({"version":2,"name":"i","canvas":{"background":"#0000ff"},"layers":[
            {"id":"c","type":"color","color":"#ffffff","x":0,"y":0,"width":100,"height":100,"visible":true,"opacity":100},
            {"id":"i","type":"image","asset":"0123456789abcdef","x":200,"y":0,"width":200,"height":100,"visible":true}]});
        let loader = |id: &str| if id == "0123456789abcdef" { Ok(Arc::clone(&img)) } else { Err("missing".to_string()) };
        let program = Program::from_scene_with("i", 1, &doc, &loader).unwrap();
        let mut c = Compositor::new(1920, 1080);
        c.compose(&program, 0, |_| None);
        let out = c.output();
        let [by, bu, _] = rgb_to_yuv(0, 0, 255);
        let [wy, ..] = rgb_to_yuv(255, 255, 255);
        let [ry, ru, rv] = rgb_to_yuv(255, 0, 0);
        assert_eq!((y_at(out, 1000, 1000), u_at(out, 1000, 1000)), (by, bu), "canvas background");
        assert_eq!(y_at(out, 50, 50), wy);
        assert_eq!((y_at(out, 250, 50), u_at(out, 250, 50)), (ry, ru));
        assert_eq!(out.v()[25 * 960 + 125], rv);
        assert_eq!(y_at(out, 350, 50), by, "transparent pixel shows the background");
        // A missing asset fails the whole Program.
        let err = Program::from_scene_with("i", 1, &doc, &|_| Err("gone".into())).unwrap_err();
        assert_eq!((err.path.as_str(), err.reason), ("/layers/1/asset", "asset_unavailable"));
        // Overlays are re-rendered for the output size.
        let small = program.for_output(640, 360);
        let mut c = Compositor::new(640, 360);
        c.compose(&small, 0, |_| None);
        assert_eq!(y_at(c.output(), 80, 10), ry);
    }

    #[test]
    fn rgb_to_yuv_limited_range_endpoints() {
        assert_eq!(rgb_to_yuv(0, 0, 0), [16, 128, 128]);
        assert_eq!(rgb_to_yuv(255, 255, 255)[0], 235);
        let [_, u, v] = rgb_to_yuv(255, 255, 255);
        assert!((127..=129).contains(&u) && (127..=129).contains(&v));
    }

    #[test]
    fn missing_rotated_source_is_black_inside_its_footprint_only() {
        let doc = v2(json!([
            {"id":"a","type":"color","color":"#ffffff","x":0,"y":0,"width":1920,"height":1080,"visible":true},
            {"id":"b","type":"video","source":"phone:rear","x":760,"y":490,"width":400,"height":100,"visible":true,"rotation":45}]));
        let program = Program::from_scene("v2", 1, &doc).unwrap();
        let mut c = Compositor::new(1920, 1080);
        let report = c.compose(&program, 0, |_| None);
        assert_eq!(report.missing_sources, vec!["phone:rear".to_string()]);
        assert_eq!(y_at(c.output(), 960, 540), BLACK_Y);
        assert_eq!(y_at(c.output(), 770, 500), 235, "corner of the unrotated rect is outside the rotated one");
    }

    #[test]
    fn golden_v2_features() {
        let img = Arc::new(Rgba { width: 3, height: 2, pixels: (0..24).map(|i| (i * 37 % 256) as u8).collect() });
        let doc = json!({"version":2,"name":"g","canvas":{"background":"#203040"},"layers":[
            {"id":"s","type":"video","source":"hdmi","x":0,"y":0,"width":1440,"height":1080,"visible":true,"fit":"contain","crop":{"left":500,"top":250,"right":1500,"bottom":0}},
            {"id":"r","type":"video","source":"phone:rear","x":1440,"y":0,"width":480,"height":540,"visible":true,"fit":"cover","rotation":-17,"opacity":85},
            {"id":"c","type":"color","color":"#c0ffee","x":100,"y":900,"width":700,"height":120,"visible":true,"opacity":60,"rotation":3},
            {"id":"i","type":"image","asset":"0123456789abcdef","x":1500,"y":700,"width":333,"height":222,"visible":true,"fit":"contain","rotation":30}]});
        let program = Program::from_scene_with("g", 1, &doc, &|_| Ok(Arc::clone(&img))).unwrap();
        let (hdmi, rear) = (pattern(1920, 1080, 4), pattern(720, 1280, 5));
        let mut c = Compositor::new(1920, 1080);
        c.compose(&program, 0, |s| match s { "hdmi" => Some(&hdmi), "phone:rear" => Some(&rear), _ => None });
        let out = c.output();
        if let Ok(p) = std::env::var("T3BOX_DUMP_GOLDEN") { std::fs::write(p, &out.bytes).unwrap(); }
        let got = (fnv1a(out.y()), fnv1a(out.u()), fnv1a(out.v()));
        assert_eq!(got, GOLDEN_V2, "v2 golden changed: {:#018x?}", got);
    }

    const GOLDEN_V2: (u64, u64, u64) = (0xaa187d24b8096401, 0xf975c6f72beded9d, 0x42a7eb94cdafa829);

    #[test]
    fn fade_mixes_from_the_previous_program_by_timestamp() {
        let switch = ProgramSwitch::new(Program::hdmi_only());
        let next = Program::from_scene("lt", 2, &lt_scene()).unwrap();
        let ack = switch.request_with(next, Transition::Fade { duration_us: 100_000 });
        let plan = switch.at_frame(10, 1_000_000);
        assert_eq!(ack.recv().unwrap().first_frame_index, 10, "ack names the first frame showing the new Program");
        assert_eq!(plan.program.revision, 2);
        let (from, mix) = plan.fade_from.expect("fading");
        assert_eq!((from.revision, mix), (0, 1));
        let (_, mid) = switch.at_frame(11, 1_050_000).fade_from.unwrap();
        assert_eq!(mid, 127);
        assert!(switch.at_frame(12, 1_100_000).fade_from.is_none(), "fade ends on time");
        // A cut replaces immediately.
        let _ = switch.request(Program::hdmi_only());
        assert!(switch.at_frame(13, 1_200_000).fade_from.is_none());
        // Cross-fade arithmetic.
        let mut out = solid(4, 4, 200, 100, 100);
        crossfade(&mut out, &solid(4, 4, 0, 100, 100), 128);
        assert_eq!(out.y()[0], ((200 * 128 + 127) / 255) as u8);
    }
}
