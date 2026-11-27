# Stephania

**The open-source operating system for live broadcasting.**

Stephania turns a Raspberry Pi 5 into a self-contained broadcasting box for
lightning talks, meetups and small events. HDMI slides, both cameras of one
phone and audio are composed on the device and streamed to YouTube Live —
no cloud service, no laptop running OBS, no terminal.

The name comes from the plant genus *Stephania* and stands for
**S**treaming and **T**ransmission **E**nvironment for **P**ortable
**H**ardware, **A**utonomous **N**etworking and **I**ntegrated
**A**rchitecture. The project was formerly called T3Box.

> **Status: in development.** The engine, management plane, Studio, phone
> app, OS image and installer are implemented and tested in CI (QEMU and
> ARM64 runners). Physical Raspberry Pi 5 qualification — real HDMI capture,
> thermals, power and multi-hour endurance — is still open; see
> [Validation](#validation). Release status of every part is tracked in
> [docs/MASTER-ARCHITECTURE.md](docs/MASTER-ARCHITECTURE.md).

## How it works

1. **Install.** Download the release ZIP, extract it onto a FAT32 microSD
   card and power the Pi on. A RAM installer verifies the signed payload and
   lays out an A/B system ([docs/INSTALL.md](docs/INSTALL.md)).
2. **Own it.** The [setup page](https://ryosk7.github.io/Stephania/) creates
   a secret in the browser; it goes onto the card as `stephania-setup.txt`
   and is shown as a QR code.
3. **Connect.** The **Stephania Connect** app scans the QR code, registers
   the phone as owner over Bluetooth LE, puts the device on Wi-Fi and
   receives a pinned TLS certificate for the LAN
   ([contracts/ble-protocol.md](contracts/ble-protocol.md)).
4. **Produce.** **Stephania Studio** (in any browser on the LAN) edits
   layouts of HDMI, phone cameras, images and text; Connect switches
   layouts, mutes audio and sends the phone's front and rear cameras.
5. **Go live.** The Rust engine composes Program, encodes H.264/AAC and
   publishes over RTMPS. Updates arrive as signed RAUC bundles with
   automatic rollback ([docs/UPDATE.md](docs/UPDATE.md)).

```text
Stephania Connect ─ BLE ──▶ provisioning (BlueZ) ──▶ NetworkManager
Stephania Connect ─ HTTPS ─▶ control (Ruby) ◀─ HTTPS ─ Stephania Studio
                                  │ Unix socket (JSON IPC)
HDMI (V4L2) ───────────▶ engine (Rust): sources ▶ compositor ▶ x264 + AAC ▶ RTMPS ▶ YouTube
phone cameras ─ WebRTC ─▶ ┘                     ALSA audio ┘
```

No media passes through the management plane, and a phone or browser
failure never stops a healthy HDMI broadcast.

## Repository

| Path | What it is |
|---|---|
| `src/` | Engine (Rust): capture, compositor, encoding, RTMPS, IPC, provisioning core, PKI |
| `services/control` | Management plane (Ruby, Sinatra, SQLite): REST + SSE API, accounts, scenes |
| `services/ble`, `services/phone` | BLE GATT adapter and WebRTC phone receiver (Rust) |
| `apps/studio` | Stephania Studio (React, Vite, Tailwind CSS, TanStack Router) |
| `apps/mobile` | Stephania Connect (Expo, React Native, Uniwind, native camera and pinned-TLS modules) |
| `packages/provisioning-client` | TypeScript BLE provisioning client |
| `tools/setup-page` | Offline setup-QR page, published on GitHub Pages |
| `os/` | Debian 13 / Raspberry Pi OS image, RAM installer, RAUC, systemd units |
| `contracts/` | OpenAPI, IPC and scene schemas, BLE protocol, shared test fixtures |
| `ci/`, `tests/` | CI scripts, QEMU and virtual-lab tests, end-to-end suites |
| `docs/` | Architecture, install, update, licensing and component documents |
| `DESIGN.md` | Design system and UI architecture for Connect and Studio |

## Development

**Engine** (Linux; native media needs `libasound2-dev libclang-dev libx264-dev libavcodec-dev libavutil-dev pkg-config`):

```bash
cargo test
cargo build --release --locked --bin stephania
```

Configuration is by environment variables, for example:

```bash
export STEPHANIA_VIDEO=/dev/video0 STEPHANIA_AUDIO=plughw:1,0
export STEPHANIA_WIDTH=1920 STEPHANIA_HEIGHT=1080 STEPHANIA_FPS=30 STEPHANIA_BITRATE_KBPS=6000
export STEPHANIA_OUTPUT='rtmps://a.rtmps.youtube.com/live2/YOUR_STREAM_KEY'   # never commit stream keys
./target/release/stephania
```

The engine is controlled over its Unix socket (`STEPHANIA_ENGINE_SOCKET`,
[docs/IPC.md](docs/IPC.md)), normally through the control service.

**Studio:** `cd apps/studio && npm ci && npm run dev` · Storybook: `npm run storybook`

**Connect:** `cd apps/mobile && npm ci && npx expo run:ios` (a development
build; the app has native modules) · Storybook on the web:
`npm run storybook:web`, on the phone: `npm run storybook`

**OS image:** `os/image/build.sh` (see [docs/IMAGE.md](docs/IMAGE.md)); the
CI builds the QEMU and Raspberry Pi variants and boots the QEMU one.

## Validation

CI runs unit and contract tests across Rust, Ruby and TypeScript;
end-to-end tests of BLE provisioning, Wi-Fi, LAN TLS, the Studio, phone
WebRTC, the RAM installer (with power cuts at every stage) and A/B
updates; and native encoding and RTMPS transport on ARM64 runners
([ci/README.md](ci/README.md)). ARM64 CI is not a Raspberry Pi 5:
physical HDMI/ALSA capture, thermal behaviour, power and long-run
reliability still need qualification ([docs/QUALIFICATION.md](docs/QUALIFICATION.md)).
`native_worker_running` reports worker activity, never confirmed YouTube
ingest.

## Licence

Stephania's own code is MIT ([LICENSE](LICENSE)). What else ships in the
OS image and the apps, and under which terms, is in
[docs/LICENSING.md](docs/LICENSING.md).
