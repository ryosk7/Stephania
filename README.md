# T3Box

A Raspberry Pi 5-based portable LT streaming box: HDMI in and passthrough to a projector, venue audio capture, and eventual YouTube Live streaming over Wi-Fi.

## Current implementation: milestone 1 (diagnostics only)

This repository currently provides a **Rust standard-library-only diagnostic controller**, not a video streaming application. Do **not** use this milestone to run an event.

- `GET http://127.0.0.1:8787/health` — local health endpoint
- `GET http://127.0.0.1:8787/status` — Linux `/dev/video*` presence, ALSA capture presence, Wi-Fi interface link state, CPU temperature
- `POST /stream/start` — returns HTTP 501 (media engine is not implemented)
- `POST /stream/stop` — reports not streaming

Status results are diagnostic hints only. Video nodes are not necessarily HDMI capture nodes; interface `operstate=up` does not prove internet connectivity.

### Build and run (Raspberry Pi OS Lite 64-bit)

Install Rust using a trusted package/toolchain source:

```sh
cargo test
cargo build --release
./target/release/t3box
curl http://127.0.0.1:8787/status
```

For deployment, create an unprivileged system account named `t3box`, install the binary at `/usr/local/bin/t3box`, and use `systemd/t3box.service`. The HTTP service is bound to loopback until authentication, authorization and CSRF protection are implemented. Never expose this API directly to an untrusted network.

### Proposed hardware

- Raspberry Pi 5, 8GB (already owned)
- USB HDMI UVC capture with HDMI passthrough (verify Linux V4L2 output modes and projector compatibility)
- USB class-compliant line-input audio ADC for a venue mixer's REC/LINE output
- Active cooling, adequate power supply (power bank compatibility needs verification)
- Venue Wi-Fi for eventual RTMPS upload

### Next engineering milestones

1. V4L2 capture: capability negotiation, timestamps, hotplug/recovery.
2. ALSA capture and PCM levels with deterministic audio/video timestamp handling.
3. H.264 encode feasibility benchmarks at 1080p30 on Pi 5; compare custom code against proven implementations, prioritizing quality.
4. AAC, FLV muxing, secure RTMPS transport, YouTube ingest tests.
5. Safe start/stop state machine, Wi-Fi provisioning, local UI, GPIO status indicators.
6. Three-hour hardware soak test, thermal and frame drop metrics, slide sharpness, A/V sync, reconnection and power interruption tests.

No live encoding, HDMI pass-through firmware control, microphone recording, Wi-Fi provisioning, or streaming has been implemented yet.
