# Experimental native combined A/V

Issue #2. V4L2 and ALSA capture, Rust color conversion and bounded PCM clock
correction, libx264 H.264 and direct libavcodec AAC-LC, one native RTMPS session.
No FFmpeg command is launched. The AAC backend uses the FFmpeg project's
libavcodec library via a small C bridge, not a custom Rust codec.

Install libasound2-dev libclang-dev libx264-dev libavcodec-dev libavutil-dev
pkg-config and a C compiler. Build with:

    cargo build --release --locked --all-features --bin stephania-native-av

Use STEPHANIA_VIDEO, STEPHANIA_AUDIO, STEPHANIA_OUTPUT (private RTMPS test key),
STEPHANIA_WIDTH, STEPHANIA_HEIGHT, STEPHANIA_FPS, STEPHANIA_BITRATE_KBPS,
STEPHANIA_AUDIO_BITRATE_KBPS (default 128), STEPHANIA_PIXEL_FORMAT (yuyv/nv12),
STEPHANIA_MAX_FRAMES (0 = continuous). Audio is fixed at 48 kHz stereo AAC-LC.

The session starts a shared monotonic timeline with 100 ms priming headroom.
ALSA period arrival time minus duration estimates the first sample time.
The clock corrector uses a 10 ms deadband and bounded linear interpolation
(up to 1000 ppm). A gap above 250 ms restarts the session instead of inventing
missing media. This is not hardware timestamp calibration. Video uses capture
timestamps; RTMP timestamps are monotonic within each track. Encoder latency
and cross-track arrival order are not yet measured or guaranteed.

Connection/write failures and capture stalls recreate the codecs and publish
session, with exponential backoff capped at 32 seconds. Authentication errors
and invalid inputs stop. Each new session sends fresh sequence headers and
starts with a fresh x264 encoder; old queued media is discarded. No claim of
seamless ingest recovery: RTMP asynchronous server controls are not yet handled.
Existing controller still uses its original FFmpeg path.

Tests cover AAC packet creation/flush, simulated clock drift in both directions,
large gaps and backoff policy. Actual network fault injection and combined
RTMP integration tests remain necessary; CI and Pi 5 acceptance are required
before event use. Licensing of linked libraries must be evaluated for distribution.
