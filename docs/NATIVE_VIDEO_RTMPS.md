# Native V4L2 -> I420 -> libx264 -> RTMPS (experimental)

This is an **all-Rust-managed, video-only** path, separate from the original
FFmpeg controller. It demonstrates direct capture, direct software H.264 encoding
and the already implemented Rust RTMPS publisher.

```sh
sudo apt install libasound2-dev libx264-dev libclang-dev pkg-config
cargo build --release --features native-capture,native-x264 --bin t3box-native-video
export T3BOX_VIDEO=/dev/video0
export T3BOX_PIXEL_FORMAT=yuyv
export T3BOX_WIDTH=1280 T3BOX_HEIGHT=720 T3BOX_FPS=30
export T3BOX_BITRATE_KBPS=3000 T3BOX_MAX_FRAMES=900
export T3BOX_OUTPUT='rtmps://a.rtmps.youtube.com/live2/PRIVATE_TEST_KEY'
./target/release/t3box-native-video
```

**Do not use this to broadcast a real event.** It has no audio, AAC or A/V sync,
no YouTube live ingest confirmation, no automatic network reconnect, no GPIO
integration, and no thermal/CPU auto-fallback.

Current supported capture pixel formats: uncompressed YUYV and NV12. MJPEG
capture is rejected rather than silently sending corrupt frames. The V4L2
driver must accept the selected width/height/pixel format. The source reports
its negotiated V4L2 bytes-per-line stride; the YUYV converter respects that
stride. NV12 conversion currently assumes the V4L2 luma/chroma pitches match
and requires hardware validation. Framerate is configured at the encoder
level, but input V4L2 frame interval negotiation is not yet guaranteed.

The CI uses a deterministic synthetic YUYV frame, native libx264 encode, then
reads the resulting RTMP H.264 messages back through the native RTMP decoder
without a real network. This demonstrates frame-to-packet integration, not
actual Raspberry Pi 5 real-time performance.

For continuous venue streaming, native USB audio + AAC encoder + clock drift
correction must be connected to the same publish session, with Pi5 soak tests.
