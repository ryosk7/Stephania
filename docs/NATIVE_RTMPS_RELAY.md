# Native RTMPS FLV relay (experimental)

This standalone command connects the existing verified rustls RTMPS transport,
performs RTMP handshake / connect / createStream / publish confirmation, then
reads pre-encoded FLV tags and relays AAC, AVC and script metadata as RTMP
messages. **It does not encode video/audio, operate GPIO or talk directly to
V4L2/ALSA.** The main T3Box controller still uses its FFmpeg backend.

Build:

```sh
cargo build --release --bin t3box-send-flv
```

Stream a local, compatible H.264/AAC FLV file at its encoded timestamps:

```sh
export T3BOX_OUTPUT='rtmps://a.rtmps.youtube.com/live2/PRIVATE_STREAM_KEY'
./target/release/t3box-send-flv /path/to/capture.flv
```

Or connect an FFmpeg encoder to the Rust sender through a pipe:

```sh
ffmpeg -nostdin -f lavfi -i 'testsrc2=size=1280x720:rate=30' \
  -f lavfi -i 'sine=frequency=440:sample_rate=48000' \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 60 \
  -c:a aac -ar 48000 -b:a 160k -f flv - \
  | ./target/release/t3box-send-flv -
```

The target URL is read from an environment variable, not process argv. It may
still be visible to programs running as the same operating-system account.
Use a restricted appliance account and never commit the stream key.

The relay uses FLV timestamp pacing and refuses backwards timestamps, gaps
greater than 30 seconds, oversized tags, unsupported codecs and media packets
without codec headers. The current transport has limited server control-message
handling and no live reconnect. Confirm YouTube Studio ingest before deployment.

**CI only verifies deterministic mock-server packet transport and synthetic
AV encoding. It does not verify YouTube Live ingestion or physical Pi hardware.**
