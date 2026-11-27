# Direct libx264 backend (experimental)

The `native-x264` feature uses the Rust `x264` wrapper with a system
`libx264` encoder directly. It does not run FFmpeg.

```sh
sudo apt install libx264-dev pkg-config
cargo test --all-features
```

Input: I420 YUV 4:2:0 planar frames produced by the Rust colorspace
converters. Output: access units containing length-prefixed AVC NALs,
keyframe information, and PTS/DTS in microseconds.

A 64x64 synthetic-frame test confirms the native encoder can emit
SPS/PPS plus compressed picture NALs and that AVCC boundaries parse
cleanly. This **does not establish 1080p30 speed on Pi 5**; that requires
measured CPU and thermal testing on the actual device.

This module is optional and not wired into the main controller or the
native RTMPS relay yet. AAC encoding, audio/video drift correction,
rate-control measurements and encoder reconfiguration remain unimplemented.

**Licensing:** libx264 itself is GPL-licensed even though the Rust wrapper
uses MIT. Before distributing a linked Stephania binary, review copyleft and
other licensing/patent obligations; the project metadata alone cannot
settle the combined binary's licensing requirements.
