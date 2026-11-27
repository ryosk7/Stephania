# T3Box

Rust-based Raspberry Pi 5 LT event streaming box.

## Production architecture

Native is the only RTMPS backend. The controller runs V4L2/ALSA capture, NEON I420 conversion, libx264, AAC via libavcodec, and RTMPS via rustls in-process. No FFmpeg executable is required for production. FFmpeg is only an independent test stimulus/decoder; no publisher subprocess remains.

## Raspberry Pi OS Lite (64-bit)

Install Rust and native packages: libasound2-dev libclang-dev libx264-dev libavcodec-dev libavutil-dev pkg-config v4l-utils alsa-utils. Confirm device paths with v4l2-ctl and arecord.

```bash
cargo build --release --locked --bin t3box
export T3BOX_VIDEO=/dev/video0
export T3BOX_AUDIO=plughw:1,0
export T3BOX_WIDTH=1920 T3BOX_HEIGHT=1080 T3BOX_FPS=30
export T3BOX_BITRATE_KBPS=6000 T3BOX_PRESET=ultrafast
export T3BOX_OUTPUT='rtmps://a.rtmps.youtube.com/live2/YOUR_STREAM_KEY'
./target/release/t3box
```

Optional experimental controls: T3BOX_PARALLEL_MEDIA=1, T3BOX_ADAPTIVE_RATE=1, T3BOX_DEADLINE_TRANSPORT=1, T3BOX_CONTENT_AWARE=1. They remain opt-in pending physical and composite-fault validation. Native supports YUYV/NV12 and ultrafast. Stop is cooperative. No automatic FFmpeg fallback.

The engine is controlled over its Unix socket (`T3BOX_ENGINE_SOCKET`, docs/IPC.md), normally through the control service. For development and the Virtual Lab, a transitional loopback HTTP API can be turned on explicitly:

```bash
export T3BOX_LEGACY_HTTP=127.0.0.1:8787   # loopback only; off when unset
curl http://127.0.0.1:8787/status
curl -X POST http://127.0.0.1:8787/stream/start
curl -X POST http://127.0.0.1:8787/stream/stop
```

native_worker_running is worker activity, not confirmed YouTube ingest. Never commit stream keys.

## Validation limits

ARM64 CI is not physical Raspberry Pi 5 hardware. Physical HDMI/ALSA, thermal behavior, power, long-run reliability and composite communication faults still need validation. See ci/README.md. Native file recording and legacy audio filtering parity are not implemented. Controller CI uses native RTMPS with explicit ci-virtual input and a local test CA. Builds without default features provide diagnostics but cannot publish RTMPS unless native features are enabled.
