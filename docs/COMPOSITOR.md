# Reference compositor (Phase 3 start)

`src/compositor.rs` is the deterministic CPU reference and test oracle. **It is not connected to the live media path**; IPC `scene.apply` still fails closed and Program over RTMPS is unchanged.

- I420 in/out, limited-range black fill, layers bottom-to-top, centre-sampled nearest-neighbour stretch, outward-rounded chroma rectangles for odd geometry.
- A visible layer whose source has no fresh frame is drawn black and reported in `FrameReport.missing_sources`; HDMI keeps composing when phone tracks disappear.
- Preallocated output and scratch buffers; steady-state composition does not allocate (tested by capacity checks).
- `ProgramSwitch`: validated immutable Program snapshots, swapped only at a frame boundary; the ack names the first frame that used the new Program. A superseded pending request's waiter observes failure, never false success.
- Golden FNV-1a hashes of a three-input pattern lock the reference output. Optimised NEON/libyuv/GPU paths must reproduce them exactly.

Not yet: alpha/opacity, crop, rotate, fit modes, images/text, transitions, bilinear/filtered scaling, integration with capture/encoder threads, HDMI fast-path bypass selection, and any performance measurement (none has been taken on a Pi 5; do not quote numbers).
