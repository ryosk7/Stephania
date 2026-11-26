# T3Box Studio architecture (accepted direction)
One Pi 5, one phone (iOS first, Android later), one HDMI input and two independent phone camera tracks. Preserve native V4L2/ALSA, x264, AAC, RTMPS and HDMI-only fast path. Expo owns BLE provisioning and native camera capture/encoding; React/Vite owns browser editing; Sinatra/Puma/Sequel/SQLite owns management. No Ruby in the media path. Buildroot target, Pi OS Lite reference. ZIP copy to empty FAT32 is the desired installer, raw image fallback. No public release until authorized.
## Safety gates
- Local management only until trusted HTTPS and authenticated BLE ownership enrollment are implemented. No default credentials. Never expose the legacy engine HTTP interface.
- One phone only; H.264/HEVC negotiation and dual hardware encoding must be tested on real phones. No downgrade of HDMI/main audio to accommodate phone overload.
- Scene drafts must not be represented as applied program until the engine acknowledges actual frame-boundary application.
- A/B update, signed bundles, power loss recovery, thermal and three-hour physical soak are release gates, not claims made by virtual CI.
- Sinatra is selected conditional on media non-regression; Spinel is research only.
## Delivery status
Initial control-plane slice: loopback-only token-authenticated API, engine adapter, scene draft persistence and Vite editor. No BLE enrollment, camera receiver, native compositor, production installer or trusted LAN TLS yet. Do not ship as a production appliance.
