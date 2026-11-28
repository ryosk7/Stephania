# Native capture backends — Pi 5 (experimental)

`--features native-capture` enables direct Linux **V4L2 mmap** HDMI video
capture and **ALSA PCM** stereo S16LE audio capture, without an FFmpeg capture
subprocess. `VideoSource` and `AudioSource` are independent Rust interfaces.

```sh
sudo apt install libasound2-dev pkg-config clang libclang-dev v4l-utils alsa-utils
cargo build --release --features native-capture --bin stephania-native-probe
STEPHANIA_VIDEO=/dev/video0 STEPHANIA_AUDIO=plughw:1,0 \
  ./target/release/stephania-native-probe
```

The V4L2 backend requests 1280x720 YUYV by default. If that format is not
supported by the attached UVC capture unit it fails explicitly: run
`v4l2-ctl -d /dev/video0 --list-formats-ext` and negotiate the actual
capture device capabilities. Some HDMI USB devices require MJPEG decoding
(which is not yet implemented here). V4L2 memory is mmap-backed inside the
capture worker; copying occurs when the frame is sent to its bounded channel.

ALSA capture requests 48 kHz, 16-bit, 2-channel interleaved PCM. It
performs an xrun recovery via `prepare()`. When the publisher loop stalls
and the bounded queue is full, the newest periods are dropped (logged once
per overload) and the device keeps running; the mixer covers the gap on its
timeline. Dropping a source closes its PCM before a replacement opens the
same device, and inputs whose device and channel count are unchanged are
kept across audio reconfiguration and reconnects. Timestamp origins use the same process
`Instant` origin for both workers, but this is **arrival-time stamping**,
not exact hardware presentation timestamps. Audio clock drift correction
and sample-accurate synchronization remain future tasks.

The capture worker reconnects to devices after errors, with a fixed 2-second
retry. A dequeued V4L2 buffer flagged `V4L2_BUF_FLAG_ERROR` or shorter than
one frame (`stride × height`, ×1.5 for NV12) is dropped and counted, not
passed on. A renegotiated mode that differs from the requested one is
reported as a retryable error: the publisher reconnects while the worker
keeps reopening the device. Once the ingest has accepted a publish, the
publisher treats every later failure as transient and reconnects with
backoff (src/reconnect.rs); only errors before the first accepted publish
(configuration, authentication) end the run.

**Feature scope:** This demonstrates native device acquisition and frame
transport only. It does not yet connect this data to the encoder or the
main `stephania` controller. No YUV conversion, JPEG decoding, AAC or H.264
encoding, GPIO, or actual YouTube publishing is performed by the probe.
Testing CI without physical HDMI/ALSA hardware only validates compilation,
interfaces and configuration boundaries. Real Pi 5 tests remain mandatory.
