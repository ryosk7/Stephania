# Raspberry Pi 5 CI scope

GitHub Actions uses Docker Buildx and QEMU to run **Linux AArch64** binaries.
This is an ARM64 container, not a virtual Raspberry Pi 5: the VideoCore, HDMI
capture device, USB audio ADC, GPIO pins, Wi-Fi radio, EDID and PSU are not simulated.

`rust-native` checks formatting, Clippy and unit tests.
`arm64-qemu` compiles the Rust controller inside an ARM64 Docker image and checks
a synthetic FFmpeg H.264/AAC FLV pipeline plus the local HTTP start/stop API.

Passing CI is **not** proof that a Raspberry Pi 5 can sustain 1080p/30fps.
The CI synthetic case deliberately uses 640x360 to avoid conflating emulator speed
with real hardware performance.

## Real-Pi acceptance criteria (still required)

- UGREEN HDMI capture mode negotiation at 1920x1080/30fps and stable passthrough
- Audio input gain, clipping/noise and sample clock drift over 3+ hours
- H.264 encoding without frame drops, thermals/power under continuous load
- RTMPS upload stability on actual venue Wi-Fi; reconnect and packet loss
- HDMI unplug/replug, media-input recovery, and projector behavior with Pi unpowered
- Anker USB-C power compatibility across the entire device
- GPIO physical start/stop, secure Wi-Fi onboarding and local UI
