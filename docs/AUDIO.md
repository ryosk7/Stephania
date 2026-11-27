# Program audio

The broadcast's audio is a live mix of up to two inputs:

| Input | Source | Default |
|---|---|---|
| `hdmi` | Embedded HDMI audio of the capture device (stereo) | enabled, device from `T3BOX_AUDIO` |
| `mic` | USB microphone (mono or stereo) | disabled (or `T3BOX_AUDIO_MIC`) |

Every input, including the single-input case, goes through the same mixer
(`src/audio_mixer.rs`); there is no separate code path or opt-in mode.

## Engine
- **Timeline:** each input has its own clock correction (`AudioClock`:
  drift and discontinuities of independent USB clocks) and a bounded queue
  (8192 frames), placed on the publisher's monotonic 48 kHz timeline. A
  missing, unplugged or failing input contributes silence: program audio
  and its PTS never stop. A block leaves as soon as every input
  that is still delivering has supplied it, so a single source adds no
  delay. Only while one input lags does the mixer wait, at most 40 ms
  (`T3BOX_AUDIO_LATENCY_MS`, 20–200), before mixing silence for the missing
  part; an input whose newest sample is older than that budget is not
  waited on. Capture timestamps are kept, so A/V sync is unaffected.
- **Gain and mute** change live and are ramped across one 21 ms block (no
  clicks). Range −60…+12 dB.
- **Limiter:** keeps the sum below −1 dBFS; instant attack inside the block
  (no sample above the ceiling), gradual release (~0.5 dB per block) so
  overload does not pump. Blocks that needed it are counted.
- **Publisher stalls:** if the network write blocks, the mixer produces the
  due blocks to catch up; beyond 1 s it moves the timeline forward and
  counts a `resync` instead of failing or replaying stale audio. The
  session keeps running in both publisher paths.
- **Devices:** inputs open and close when the configuration changes;
  ALSA errors and unplugging are reported per input and the device is
  reopened automatically every 2 s.

## Control
IPC (contracts/ipc.schema.json): `audio.status` (per input: configuration,
`state` = disabled | idle | waiting | active | stalled | error, and while
streaming `levels` {peak_dbfs, rms_dbfs, clipped} over the last ~320 ms;
`master` levels; `resyncs`; `limited_blocks`) and `audio.configure
{inputs:[{id, enabled?, device?, channels?, gain_db?, muted?}]}` (partial;
validated: ALSA names only, distinct devices, ranges).

Management API: `GET /api/v1/audio` (stored configuration + live status),
`PUT /api/v1/audio` (partial update; owner only, camera credentials get
403). The configuration is stored in SQLite (`audio_inputs`) and pushed to
the engine on every engine (re)start. SSE: `audio.levels` (~4 Hz while
streaming and someone is listening) and `audio.configured`.

Studio: audio panel with program and per-input meters, state, enable,
device, gain slider (applied live) and mute.

## Verified in CI
- Unit: mixing, bit-exact single input at unity, gain ramp, limiter
  ceiling and release, catch-up and resync; configuration validation and
  status states; control API validation, persistence, re-apply on engine
  reconnect, role restriction, level events; Studio reducer and meters.
- `ci/audio-e2e.sh` (arm64): real engine, virtual 440 Hz (hdmi) and 880 Hz
  (mic) inputs, live enable/mute/gain over IPC while publishing RTMPS;
  status levels checked at each step, and the decoded recording must show
  hdmi only → both → mic muted → hdmi −12 dB (±1.5 dB), each ≥ 2.5 s, with no
  silent window.
- Playwright: configure, persist across reload, apply to the engine, refuse
  unsafe device names.

## Needs hardware (docs/QUALIFICATION.md Q-AV)
Real USB clock drift between two devices over 3 h, hotplug timing, CPU on
the Pi 5, and level calibration of the actual HDMI source and microphone.
