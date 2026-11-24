# T3Box

Rust-based Raspberry Pi 5 LT event streaming box.

## Current implementation (prototype, not event-ready)

- Rust controller and HTTP API on `127.0.0.1:8787` (`/health`, `/status`, `/stream/start`, `/stream/stop`).
- Device diagnostics (Linux video nodes, ALSA capture presence, Wi-Fi interface, CPU temperature).
- Rust process supervisor launching FFmpeg with V4L2 HDMI capture, ALSA audio, libx264 video, AAC audio, FLV muxing and RTMPS output.
- Synthetic A/V input option for ARM64 QEMU CI. Automatic FFmpeg process restart attempts.
- An ARM64 Docker build and simulated-input test in GitHub Actions.

The media pipeline **currently uses FFmpeg**, which is open source but not a custom Rust codec. A custom encoder would need considerable work and comparative benchmarks; an unproven implementation cannot be called better quality. The subprocess being alive **does not prove YouTube ingested the stream**.

## Setup on Raspberry Pi OS Lite (64-bit)

Install Rust, FFmpeg with libx264, `v4l-utils` and `alsa-utils`. Confirm device paths with `v4l2-ctl --list-formats-ext` and `arecord -l`.

```bash
cargo test
cargo build --release
export T3BOX_VIDEO=/dev/video0
export T3BOX_AUDIO=plughw:1,0
export T3BOX_WIDTH=1920 T3BOX_HEIGHT=1080 T3BOX_FPS=30
export T3BOX_BITRATE_KBPS=8000 T3BOX_PRESET=ultrafast
export T3BOX_OUTPUT='rtmps://a.rtmps.youtube.com/live2/YOUR_STREAM_KEY'
./target/release/t3box
```

In a separate terminal on the Pi:

```bash
curl http://127.0.0.1:8787/status
curl -X POST http://127.0.0.1:8787/stream/start
curl -X POST http://127.0.0.1:8787/stream/stop
```

Test without HDMI/audio hardware using `T3BOX_TEST_SOURCES=1 T3BOX_WIDTH=640 T3BOX_HEIGHT=360 T3BOX_OUTPUT=file:/tmp/t3box-test.flv cargo run` and then the same start/stop commands.

**Security:** Do not commit stream keys. The current prototype passes RTMPS URL in FFmpeg process arguments; other local users may be able to inspect it. The API is loopback-only until authentication is added. Do not deploy on untrusted networks without completing authentication and secret management.

## Hardware and CI limits

Presenter HDMI → HDMI capture with hardware HDMI passthrough → projector. Capture USB → Raspberry Pi 5; venue mixer LINE/REC → USB audio ADC → Pi 5; venue Wi-Fi → YouTube.

Docker/QEMU runs Linux ARM64 userspace, **not a full Raspberry Pi 5 VM**. Real HDMI capture, EDID/passthrough, GPIO, venue Wi-Fi, Anker power delivery, real-time 1080p30 and thermal behavior require Pi 5 hardware. See [CI acceptance checklist](ci/README.md).

## Remaining features

Wi-Fi setup portal, physical button/LED, audio levels and clip alarms, HDMI disconnection fallback slate, secure smartphone UI, secure RTMPS key storage, YouTube ingest confirmation, native Rust media pipeline, throughput/temperature benchmarking and power management are not implemented.
