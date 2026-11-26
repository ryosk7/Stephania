# Reference compositor (Phase 3 start)

`src/compositor.rs` is the deterministic CPU reference. `src/composition.rs` puts it in the live media path, **opt-in with `T3BOX_COMPOSITOR=1`**:

- After capture conversion, each frame passes through the composition stage. If the applied Program is "HDMI full screen" (the default), the converted capture goes to x264 unchanged. This is the **HDMI fast path**: no composition and no copy. Without `T3BOX_COMPOSITOR=1` the stage does not exist at all.
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

Not yet: alpha/opacity, crop, rotate, fit modes, images/text, transitions, bilinear/filtered scaling, integration with capture/encoder threads, HDMI fast-path bypass selection, and any performance measurement (none has been taken on a Pi 5; do not quote numbers).
