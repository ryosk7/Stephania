# Phone camera media

Implements the device side of docs/MASTER-ARCHITECTURE.md §12–13: one
phone, up to two independent camera tracks (`phone:rear`, `phone:front`),
received over WebRTC and decoded on the Pi into the compositor.

```
phone app ──HTTPS (SDP)──▶ control (Sinatra) ──Unix socket──▶ stephania-phone
phone app ══WebRTC (ICE/DTLS-SRTP/RTP, UDP 40100)══▶ stephania-phone
stephania-phone ──Unix socket (H.264 access units)──▶ engine ── libavcodec ──▶ compositor sources
```

## Processes
- **stephania-phone** (`services/phone`, webrtc-rs): terminates WebRTC in its
  own unprivileged process (uid 855). H.264 only
  (`packetization-mode=1`, constrained baseline 42e01f), LAN host
  candidates on one UDP port, no STUN/TURN, non-trickle answer. RTP is
  reassembled into Annex-B access units; after any loss nothing is
  forwarded until the next keyframe and a PLI is sent, so the program
  output freezes briefly instead of showing corrupt frames. A crash or
  restart never touches the engine.
- **engine** (`src/phone_ingest.rs`, `src/h264_decode.rs`): ingest socket
  `/run/stephania/media.sock`, only uid 855. One libavcodec decoder per source
  (slice threads, low delay); decoded I420 frames go to the
  `SourceRegistry`, where frames older than 500 ms are not composed (the
  layer falls back). Decoder errors and streams joined mid-GOP produce
  keyframe requests back to stephania-phone (rate-limited).
- **control**: `POST /api/v1/cameras/session/offer {sdp, tracks, replace?}`
  → `{sdp}`; `GET`/`DELETE /api/v1/cameras/session`. `tracks` maps the
  offer's MediaStreamTrack ids to `rear`/`front`. A second phone gets
  409 `busy` unless `replace` is true.

## Camera credentials
The owner issues short-lived camera tokens (`POST /api/v1/credentials/camera
{ttl_s ≤ 86400, label}` → `t3c_…`, shown once, stored as SHA-256, at most 8).
A camera token can only call the three camera-session endpoints (403
`forbidden_role` elsewhere, including events, status and credentials),
cannot replace or end a session it did not open, stops working at expiry,
and revoking it ends the session it opened. A session already running when
its credential expires continues until it is closed or revoked.

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

## Phone app sender (iOS)
`apps/mobile/modules/stephania-camera` (native) + `apps/mobile/src/features/camera`
(shared TypeScript). The split is the cross-platform contract
(`modules/stephania-camera/src/StephaniaCamera.types.ts`): native code captures,
encodes and transports; TypeScript decides. An Android implementation
provides the same interface and reuses everything in `src/features/camera`.

- **Capture** (`ios/CameraCapture.swift`): built-in wide cameras,
  `AVCaptureMultiCamSession` for rear+front. Output is always landscape
  16:9: each connection is rotated in the ISP to the horizon-level angle
  (iOS 17 `RotationCoordinator`), because stephania-phone ignores the RTP
  orientation extension. Frames go from the capture queue straight into
  the WebRTC video sources; no frame crosses the JS bridge.
- **Profiles** (§12): dual `720p30+720p30`, `720p30+540p24`,
  `540p24+540p24`; a profile is offered only if a multi-cam session
  configured with its formats reports `hardwareCost` and
  `systemPressureCost` below 1 (`capabilities().dualProfiles`).
- **Encode / transport** (`ios/CameraSender`, SwiftPM, also the pod's
  sources): libwebrtc (`WebRTC-SDK` 150.7871.09), VideoToolbox H.264 via
  the default encoder factory. The local offer is restricted to H.264
  constrained baseline (`42e0xx`; stephania-phone matches profile and
  packetization mode, not level), `packetization-mode=1` (+RTX) before
  `setLocalDescription`; one sendonly transceiver per camera, stream
  `stephania`, track ids `rear-…`/`front-…`; LAN only (no STUN/TURN), the
  offer is sent after ICE gathering completes (non-trickle). Plans change
  `maxBitrate`, `maxFramerate`, `scaleResolutionDownBy` and `active` on
  the running senders, without renegotiation.
- **Policy** (`src/features/camera/session.ts`, `plan.ts`, `sdp.ts`): idle →
  starting → connecting → live; `reconnecting` with backoff 0.5–8 s
  (6 attempts) after `failed`, or after `disconnected` lasting 3 s;
  reconnects replace the phone's own session (`replace: true`). HTTP 409
  `busy` → the user may take over. Background → peer and cameras closed,
  `paused`; foreground → new session. Capture interruptions (phone call,
  other app, multitasking, system pressure) keep the peer and are shown.
  Thermal: the hotter of `ProcessInfo.thermalState` and the capture
  devices' system pressure selects the plan; dual mode steps down one
  profile per level and switches the front (audience) camera off at
  `critical`, keeping the presenter. The offer is checked against what
  stephania-phone accepts before it is sent.
- **Screen** (`src/features/camera/camera-screen.tsx`): native Metal previews of what is fed
  to the encoder, rear/front/both selector (both only when a dual profile
  is available), status, thermal notice, take-over prompt; the screen
  stays awake while open.

Verified in CI: `src/features/camera/*.test.ts` (state machine with a fake native
sender and clock, plans, offer checks); `mobile-ios-camera` (macOS,
`swift test`): SDP restriction, and a real libwebrtc loopback — two
tracks with different synthetic content arrive decoded as H.264 at the
planned size, not swapped; switching the front track off stops it, rear
drops to 540p, and both come back; `phone-ios-offer`: that libwebrtc offer
is accepted by the real stephania-phone through control and answered with
H.264 42e01f for both tracks. Capture, rotation, MultiCam cost, hardware
encoder use and thermal behaviour on a real iPhone are physical
qualification (Q-PHONE); none has been run.

## Not done yet
- Android sender (Kotlin, Camera2 concurrent cameras, MediaCodec): paused;
  it implements `StephaniaCamera.types.ts`.
- WSS/trickle ICE, audio from the phone.
- Decode load on the Pi 5 (two 720p30 software decodes next to x264) is
  unmeasured: physical qualification Q-PHONE / Q-AV.
