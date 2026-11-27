# Phone camera media

Implements the device side of docs/MASTER-ARCHITECTURE.md §12–13: one
phone, up to two independent camera tracks (`phone:rear`, `phone:front`),
received over WebRTC and decoded on the Pi into the compositor.

```
phone app ──HTTPS (SDP)──▶ control (Sinatra) ──Unix socket──▶ t3box-phone
phone app ══WebRTC (ICE/DTLS-SRTP/RTP, UDP 40100)══▶ t3box-phone
t3box-phone ──Unix socket (H.264 access units)──▶ engine ── libavcodec ──▶ compositor sources
```

## Processes
- **t3box-phone** (`services/phone`, webrtc-rs): terminates WebRTC in its
  own unprivileged process (uid 855). H.264 only
  (`packetization-mode=1`, constrained baseline 42e01f), LAN host
  candidates on one UDP port, no STUN/TURN, non-trickle answer. RTP is
  reassembled into Annex-B access units; after any loss nothing is
  forwarded until the next keyframe and a PLI is sent, so the program
  output freezes briefly instead of showing corrupt frames. A crash or
  restart never touches the engine.
- **engine** (`src/phone_ingest.rs`, `src/h264_decode.rs`): ingest socket
  `/run/t3box/media.sock`, only uid 855. One libavcodec decoder per source
  (slice threads, low delay); decoded I420 frames go to the
  `SourceRegistry`, where frames older than 500 ms are not composed (the
  layer falls back). Decoder errors and streams joined mid-GOP produce
  keyframe requests back to t3box-phone (rate-limited).
- **control**: `POST /api/v1/cameras/session/offer {sdp, tracks, replace?}`
  → `{sdp}`; `GET`/`DELETE /api/v1/cameras/session`. `tracks` maps the
  offer's MediaStreamTrack ids to `rear`/`front`. A second phone gets
  409 `busy` unless `replace` is true.

## Engine ingest framing
`u32be length ‖ payload`. Client → engine: `0x01 ‖ source ‖ pts_us u64be ‖
Annex-B AU` (≤ 2 MiB). Engine → client: `0x02 ‖ source` (keyframe
request). Sources: 1 = rear, 2 = front.

## Status
`engine.status` / `GET /api/v1/status` include `sources`: per source
`{id, width, height, age_ms, fresh, mean_luma}`.

## Verified in CI (`tests/phone/run-e2e.sh`)
aiortc sends two 640×360 H.264 tracks of different brightness through the
real stack: answer via control, ICE + DTLS connected, second phone
refused, both sources decoded and fresh with the right geometry and
content (not swapped), access units at frame rate, sources stale after
close. Unit tests: RTP reassembly (FU-A, loss, lost marker, wrap,
duplicates), decoder round-trip with the engine's own x264 encoder,
ingest protocol and keyframe requests.

## Not done yet
- Phone app sender (react-native-webrtc / native camera modules with
  hardware encoding, iOS MultiCam, Android Camera2) — the app only has the
  BLE/LAN enrollment today.
- Camera-role credentials (the endpoint currently takes the management
  token), WSS/trickle ICE, bandwidth/congestion tuning, audio from the phone.
- Decode load on the Pi 5 (two 720p30 software decodes next to x264) is
  unmeasured: physical qualification Q-PHONE / Q-AV.
