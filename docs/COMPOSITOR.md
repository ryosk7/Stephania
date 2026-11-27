# Reference compositor

`src/compositor.rs` is the deterministic CPU reference. `src/composition.rs` puts it in the live media path, **on by default** (`T3BOX_COMPOSITOR=0` removes the stage, diagnostics only):

- After capture conversion, each frame passes through the composition stage. If the applied Program is "HDMI full screen" (the default), the converted capture goes to x264 unchanged. This is the **HDMI fast path**: no composition and no copy. With `T3BOX_COMPOSITOR=0` the stage does not exist at all.
- Scenes are drawn on the 1920×1080 canvas and scaled to the encoder size, with layers aligned to even pixels.
- IPC `scene.apply {scene, scene_id, revision}` works like this:
  1. It hands an immutable Program to the media thread.
  2. It returns `{applied, first_frame_index}` only after a frame was encoded with it.
  3. With no stream running it returns `program_inactive`. If no frame boundary arrives within 2 s it returns `program_timeout`, and the pending request is cancelled so it can never take effect later.
- `engine.status.program` reports the Program on air, for reconciliation.
- Phone tracks enter through `SourceRegistry`. A frame older than 500 ms counts as missing and is drawn black. A WebRTC receiver will feed it; in CI, a virtual `phone:rear` source does (`T3BOX_CI_VIRTUAL_PHONE=1`).
- Verified by CI job `arm64-composition` (`ci/composition-e2e.sh`), which runs the controller binary with ci-virtual HDMI and a virtual phone over local RTMPS:
  - **IPC behaviour:** apply before the stream starts fails closed.
  - **Frame accuracy:** PIP and then full screen are applied mid-stream. Pixel checks after H.264 decoding show the phone layer exactly from the PIP ack frame up to the frame before the full-screen ack, with HDMI-only frames before and after.

- I420 in/out, limited-range black fill, layers bottom-to-top, centre-sampled nearest-neighbour stretch, outward-rounded chroma rectangles for odd geometry.
- A visible layer whose source has no fresh frame is drawn black and reported in `FrameReport.missing_sources`; HDMI keeps composing when phone tracks disappear.
- Preallocated output and scratch buffers; steady-state composition does not allocate (tested by capacity checks).
- `ProgramSwitch`: validated immutable Program snapshots, swapped only at a frame boundary; the ack names the first frame that used the new Program. A superseded pending request's waiter observes failure, never false success.
- Golden FNV-1a hashes of a three-input pattern lock the reference output. Optimised NEON/libyuv/GPU paths must reproduce them exactly.

Scene v2 (docs/SCENE.md):

- Opacity, rotation (integer degrees, Q16 sine table), fit (stretch/contain/cover) and crop go through one integer-only mapping (Q8 positions) with nearest-neighbour sampling and straight-alpha blending; no floating point, so x86 and Arm frames are identical. Plain video layers keep the original path and its golden hashes; a second golden hash locks the v2 path.
- Image (PNG asset, straight alpha) and colour layers are static: rendered into an `Overlay` (Y/U/V + luma and chroma alpha) once per output size — on the IPC thread before the request reaches the media thread — and only blended per frame. RGB→YCbCr is BT.709 limited range in integers.
- Canvas background colour; a missing video source is still drawn black (signal loss stays visible).
- Fade transitions: `ProgramSwitch::request_with(program, Transition::Fade{duration_us})`; the stage composes the outgoing Program into a second buffer and cross-fades by frame timestamps. The ack is the first frame showing any of the new Program.
- CI `arm64-composition` additionally fades in a v2 scene with an image asset and checks, after H.264 decoding, that the region is grey before the ack, rises monotonically over the 15 fade frames and is the asset's colour afterwards; a scene with a missing asset is refused with Program unchanged.

Optimised kernels (`src/simd.rs`): alpha blending of overlays and the fade cross-mix use AArch64 NEON (16 pixels per step, exact integer division by 255, proven exhaustively in the unit tests and checked against the scalar reference on random rows); the plain video path copies whole rows when the horizontal scale is 1:1. Output is byte-identical to the reference, and the golden hashes run on x86_64 and on arm64 (`arm64-logical`).

Not yet: bilinear/filtered scaling, NEON for the nearest-neighbour gather and the rotation/crop mapping, and any performance measurement on a Pi 5 (none has been taken; do not quote numbers).
