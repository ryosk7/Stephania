# Reference image

`os/image/build.sh` builds the reference appliance image in the A/B layout
of docs/UPDATE.md. CI (`.github/workflows/os.yml`, job `image-qemu`) builds
it on arm64 and boots it in QEMU.

## What it contains
- Raspberry Pi OS Lite reference (§6): an mmdebstrap root filesystem of
  Debian 13 "trixie" — the base of Raspberry Pi OS — plus, for the Pi
  variant, the Raspberry Pi archive (`archive.raspberrypi.com`, key pinned
  by fingerprint) for the kernel and device trees, the way pi-gen builds
  Raspberry Pi OS Lite. Packages: systemd, systemd-timesyncd (NTP; listed
  explicitly because it is only a Recommends of systemd and the build
  autoremoves everything not listed), NetworkManager, BlueZ, Avahi,
  RAUC (+ D-Bus service), SQLite, Ruby, the native engine's runtime
  libraries (x264, libavcodec 61, ALSA). The QEMU variant uses the same
  Debian packages with Debian's generic kernel.
- Stephania binaries built in the same distribution as the image (build image
  `ci/images/device-build.Dockerfile`, `debian:trixie`) so library sonames
  match; tests on CI runners keep using `ci/images/build.Dockerfile`: `stephania` (all native features), `stephania-provision`, `stephania-pki`,
  `stephania-bootctl`, `stephania-maint`, `steph`, `stephania-phone` and the
  BLE adapter `stephania-ble` (enabled; it starts with bluetoothd once a
  controller exists, and is the claim and Wi-Fi setup path); control plane in `/opt/stephania/control` with production
  gems; Studio in `/opt/stephania/studio`.
- Gems with native extensions are compiled with temporary build packages
  that are then purged; the build fails if `gcc`, `cc`, `g++` or `make`
  remain (§6: no compilers on the device).
- Units, sysusers/tmpfiles, RAUC `system.conf` (empty keyring: every bundle
  is refused until the release pipeline provisions the release CA),
  watchdog drop-in, BlueZ config.

## Host name and mDNS
Each device picks `stephania-<4 hex>` once and keeps it on `/data`
(`stephania-hostname.service`, before the network, Avahi and the PKI), so the
device certificate names a stable host and two boxes never collide on
`.local`. Avahi publishes the host name plus `_https._tcp` and
`_stephania._tcp` on port 9443 (`os/avahi/stephania.service`, TXT `api=/api/v1`,
`ca=/ca.pem`). The smoke test checks the persisted name, the certificate
host, the DNS-SD record and name resolution.

## Layout and root filesystem
Fixed slot sizes from `os/layout.env` (selector 64 MiB, boot 256 MiB,
root 1 GiB, data ≥ 512 MiB), shared with the installer (docs/INSTALL.md).
GPT: selector (autoboot.txt), boot-a, boot-b, rootfs-a (SquashFS, zstd),
rootfs-b (empty until the first update), data (ext4). The root is never
written: `init=/usr/local/sbin/stephania-overlay-init` mounts a tmpfs overlay
over the read-only SquashFS before exec'ing systemd. Persistent state is on
`/data` (fstab), and `/var/lib/stephania` is bind-mounted from it. The first
boot creates the loopback bootstrap token on `/data`. The boot selector is
mounted root-owned, files 0644: only stephania-bootctl (root) writes
`autoboot.txt`, everyone can read it (`steph update status`).

## Clock
A Pi 5 without an RTC battery starts with the clock systemd was built
with. `/usr/lib/clock-epoch` (its SquashFS mtime is the release commit
time) raises that floor to the release date, `systemd-timesyncd` then
synchronises it, and `stephania-time-sync.service` holds the engine back
until it has, for at most 45 s, so RTMPS certificate checks run on a real
clock when NTP is reachable. Without NTP the device still starts after
the bound.

## QEMU smoke test (`ci/qemu-smoke.sh`)
Direct kernel boot (Debian's generic `linux-image-<arch>` kernel, virtio disk,
`stephania.smoke=1`) runs `stephania-qemu-smoke.service`, which checks: overlay
over read-only SquashFS and that the lower root cannot be written; data
partition and bind mount; first-boot token; no compiler; engine, control and
first-boot units active; stephania-ble installed and enabled with its BlueZ-only
D-Bus policy; systemd-timesyncd enabled and running; boot selector root-only
writable; nftables limiting 22, 9443 and UDP 40100 to LAN sources; control API with the bootstrap token and through
to the engine over IPC; Studio served; engine health via `stephania-bootctl`;
device PKI endpoint; LAN TLS listener verified against the device CA and
refusing without an issued token. It powers off and the harness requires
`SMOKE RESULT PASS`.

## Not covered (yet)
- **Booting the Raspberry Pi variant** on hardware (Q-BOOT). The variant
  itself exists: `STEPHANIA_IMAGE_VARIANT=pi` installs Raspberry Pi's
  `linux-image-rpi-2712` (Pi 5 kernel), puts `vmlinuz`, `initrd.img`,
  `bcm2712-rpi-5-b.dtb`, `overlays/` (from `/usr/lib/linux-image-<ver>`),
  `config.txt` and a per-slot `cmdline.txt` into each boot slot, and is
  checked structurally by `ci/pi-image-check.sh` (nightly job `image-pi`,
  and every release). Raspberry Pi OS Lite's extra tools (`rpi-eeprom`,
  `raspi-config`) are not included: the appliance updates through RAUC
  and has no interactive configuration.
- Anything hardware: VideoCore, RP1, HDMI capture, radios, tryboot
  (docs/QUALIFICATION.md).
