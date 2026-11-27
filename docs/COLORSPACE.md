# I420 conversion baseline

Stephania includes a native Rust scalar converter for YUYV 4:2:2 and NV12
input frames. It produces tightly packed planar Y, U, V buffers for
H.264 encoder APIs. Input pitch/stride is explicit so padding on USB
capture devices is handled correctly.

- YUYV to I420: every pair of luma pixels is preserved; each 2x2 block's
  vertical U and V values are averaged with integer rounding.
- NV12 to I420: luma rows and interleaved U/V rows are copied to separate
  planes, supporting independent Y/UV row strides.
- All conversions reject odd dimensions, oversized frames, truncated
  buffers, and malformed row pitches.
- Output should be compared to a known reference implementation for actual
  HDMI capture frames before production use. Chroma siting and color range
  metadata still need negotiated handling and may affect presentation color.

This is a **correctness-first scalar reference implementation**, not yet
a benchmarked NEON-accelerated Pi 5 converter. It is not yet wired to
an H.264 encoder.
