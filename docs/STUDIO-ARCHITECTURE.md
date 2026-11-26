# T3Box Studio architecture (accepted direction)
One Pi 5, one phone (iOS first, Android later), one HDMI input and two independent phone camera tracks. Preserve native V4L2/ALSA, x264, AAC, RTMPS and HDMI-only fast path. Expo owns BLE provisioning and native camera capture/encoding; React/Vite owns browser editing; Sinatra/Puma/Sequel/SQLite owns management. No Ruby in the media path. Buildroot target, Pi OS Lite reference. ZIP copy to empty FAT32 is the desired installer, raw image fallback. No public release until authorized.
## Safety gates
- Local management only until trusted HTTPS and authenticated BLE ownership enrollment are implemented. No default credentials. Never expose the legacy engine HTTP interface.
- One phone only; H.264/HEVC negotiation and dual hardware encoding must be tested on real phones. No downgrade of HDMI/main audio to accommodate phone overload.
- Scene drafts must not be represented as applied program until the engine acknowledges actual frame-boundary application.
- A/B update, signed bundles, power loss recovery, thermal and three-hour physical soak are release gates, not claims made by virtual CI.
- Sinatra is selected conditional on media non-regression; Spinel is research only.
## Delivery status
Control-plane slice: loopback-only token-authenticated API, versioned engine IPC over `/run/t3box/engine.sock` (docs/IPC.md), consistent error envelope with request IDs (contracts/openapi.yaml), scene draft persistence with optimistic revisions, independent engine-side scene validation with shared cross-language fixtures, and a Vite editor. Program transition is wired through `scene.apply` and fails closed (501) in the engine. No BLE enrollment, camera receiver, native compositor, production installer or trusted LAN TLS yet. Do not ship as a production appliance.

Run locally:
```bash
T3BOX_ENGINE_SOCKET=/tmp/t3box/engine.sock ./target/release/t3box   # engine
cd services/control && T3BOX_CONTROL_TOKEN=$(openssl rand -hex 16) \
  T3BOX_ENGINE_SOCKET=/tmp/t3box/engine.sock bundle exec rackup -o 127.0.0.1
```
