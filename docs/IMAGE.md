# Reference image

`os/image/build.sh` builds the reference appliance image in the A/B layout
of docs/UPDATE.md. CI (`.github/workflows/os.yml`, job `image-qemu`) builds
it on arm64 and boots it in QEMU.

## What it contains
- mmdebstrap root filesystem (Ubuntu 24.04 "noble", arm64): systemd,
  NetworkManager, BlueZ, Avahi, RAUC (+ D-Bus service), SQLite, Ruby, the
  native engine's runtime libraries (x264, libavcodec, ALSA).
- T3Box binaries built in the same distribution as the image (CI build
  image `ci/images/build.Dockerfile`, `ubuntu:24.04`) so library sonames
  match: `t3box` (all native features), `t3box-provision`, `t3box-pki`,
  `t3box-bootctl`; control plane in `/opt/t3box/control` with production
  gems; Studio in `/opt/t3box/studio`.
- Gems with native extensions are compiled with temporary build packages
  that are then purged; the build fails if `gcc`, `cc`, `g++` or `make`
  remain (§6: no compilers on the device).
- Units, sysusers/tmpfiles, RAUC `system.conf` (empty keyring: every bundle
  is refused until the release pipeline provisions the release CA),
  watchdog drop-in, BlueZ config.

## Host name and mDNS
Each device picks `t3box-<4 hex>` once and keeps it on `/data`
(`t3box-hostname.service`, before the network, Avahi and the PKI), so the
device certificate names a stable host and two boxes never collide on
`.local`. Avahi publishes the host name plus `_https._tcp` and
`_t3box._tcp` on port 9443 (`os/avahi/t3box.service`, TXT `api=/api/v1`,
`ca=/ca.pem`). The smoke test checks the persisted name, the certificate
host, the DNS-SD record and name resolution.

## Layout and root filesystem
Fixed slot sizes from `os/layout.env` (selector 64 MiB, boot 256 MiB,
root 1 GiB, data ≥ 512 MiB), shared with the installer (docs/INSTALL.md).
GPT: selector (autoboot.txt, `t3box-setup.txt`), boot-a, boot-b, rootfs-a (SquashFS, zstd),
rootfs-b (empty until the first update), data (ext4). The root is never
written: `init=/usr/local/sbin/t3box-overlay-init` mounts a tmpfs overlay
over the read-only SquashFS before exec'ing systemd. Persistent state is on
`/data` (fstab), and `/var/lib/t3box` is bind-mounted from it. The first
boot creates the loopback bootstrap token on `/data`.

## QEMU smoke test (`ci/qemu-smoke.sh`)
Direct kernel boot (generic `linux-image-virtual` kernel, virtio disk,
`t3box.smoke=1`) runs `t3box-qemu-smoke.service`, which checks: overlay
over read-only SquashFS and that the lower root cannot be written; data
partition and bind mount; first-boot token; no compiler; engine, control and
first-boot units active; control API with the bootstrap token and through
to the engine over IPC; Studio served; engine health via `t3box-bootctl`;
device PKI endpoint; LAN TLS listener verified against the device CA and
refusing without an issued token. It powers off and the harness requires
`SMOKE RESULT PASS`.

## Not covered (yet)
- **Booting the Raspberry Pi variant** on hardware (Q-BOOT). The variant
  itself exists: `T3BOX_IMAGE_VARIANT=pi` uses Ubuntu's `linux-image-raspi`
  (Pi 5 support), puts `vmlinuz`, `initrd.img`, `bcm2712-rpi-5-b.dtb`,
  `overlays/`, `config.txt` and a per-slot `cmdline.txt` into each boot
  slot, and is checked structurally by `ci/pi-image-check.sh` (nightly
  job `image-pi`, and every release). It deviates from §6's Raspberry Pi
  OS reference so that the binaries keep the build image's library sonames.
- Anything hardware: VideoCore, RP1, HDMI capture, radios, tryboot
  (docs/QUALIFICATION.md).
