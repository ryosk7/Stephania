# A/B updates (RAUC + Raspberry Pi 5 tryboot)

Implements docs/MASTER-ARCHITECTURE.md §8. RAUC does everything it is good
at (X.509-signed verity bundles, compatibility, slot selection, writing,
handlers); T3Box adds only a RAUC custom bootloader backend for the Pi
firmware and a boot-time confirmation.

## Layout (GPT, provisional sizes)
| # | Label | Content |
|---|---|---|
| 1 | `selector` | FAT, `autoboot.txt` only (mounted at `/boot/selector`) |
| 2 | `boot-a` | FAT: kernel, DTBs, `cmdline.txt` (`root=PARTLABEL=rootfs-a rauc.slot=A`) |
| 3 | `boot-b` | same for B |
| 4 | `rootfs-a` | SquashFS, read-only |
| 5 | `rootfs-b` | SquashFS, read-only |
| 6 | `data` | ext4, `/data/t3box` (persistent, shared by both slots) |

RAUC slot groups pair `boot.N` with `rootfs.N`; bootnames `A`/`B` map to
firmware partitions 2/3 (`os/rauc/system.conf`).

## Flow
1. `t3box-update bundle.raucb` → `rauc install`. The pre-install handler
   (`t3box-bootctl guard`) refuses while the engine reports
   `streaming_requested` (§8: never update while streaming).
2. RAUC verifies signature (`check-purpose=codesign`) and `compatible`,
   writes the inactive pair, then calls `set-primary B`. The backend does
   **not** switch permanently: it records B as pending and writes
   `[tryboot] boot_partition=3`; `[all]` still names A.
3. `systemctl reboot '0 tryboot'`: the firmware boots partition 3 once.
4. `t3box-update-confirm.service` (`t3box-bootctl confirm`): booted from
   the pending slot via tryboot → local health (engine IPC answers, data
   writable; no network/HDMI dependency) → `rauc status mark-good booted`
   → backend commits `[all] boot_partition=3`. Unhealthy → `mark-bad
   booted` and reboot → firmware returns to A.
5. Any crash, watchdog reset or power loss during the trial: the next
   boot is a normal boot, so the firmware uses `[all]` (A). `confirm` sees
   a pending trial while booted elsewhere and marks it bad.

6. Persistent data: the pre-install handler also takes an online SQLite
   backup of the control database (`t3box-bootctl snapshot`). When a trial
   is rejected (health failure, crash, deadline), `t3box-data-restore.service`
   restores it on the committed slot **before** the control plane starts
   (the old version cannot read a schema the trial migrated). A crashed
   trial is restored once; the later mark-bad does not restore again over
   data written since. A committed trial discards the snapshot.
7. Deadline and watchdog: `t3box-update-deadline.timer` rejects a trial not
   confirmed 10 min after boot; the hardware watchdog
   (`dtparam=watchdog=on`, `os/systemd/10-t3box-watchdog.conf`) turns a
   hung system into a reset, i.e. a normal boot onto the committed slot.

`set-state X good` commits only while actually booted from X, so nothing
but a confirmed trial can change the permanent selection. State
(`pending`, bad slots) is a small JSON file on the data partition; every
file is replaced atomically (write, fsync, rename, fsync dir).

## Verified in CI (`.github/workflows/os.yml`, `ci/update-e2e.sh`)
Real RAUC v1.15 (built from source) on a 6.8 kernel. Found while setting this up: on kernels whose dm-verity status carries an FEC field (`V -`), RAUC v1.15 refuses to mount verity bundles; the image must pin a kernel/RAUC pair that agree (risk register) with verity bundles on loop devices, test-only
signing PKI generated per run, emulated tryboot firmware:
untrusted signer refused; incompatible bundle refused; install refused
while streaming; RAUC killed mid-write leaves A committed and bootable;
healthy trial committed; unhealthy trial rolled back and marked bad;
crashed trial falls back by the firmware rule; data migrated by a rejected
or crashed trial restored to the pre-update snapshot (exactly once),
committed data kept; unconfirmed trial rejected at the deadline. The
watchdog itself needs hardware (Q-BOOT).

## Not yet done
- Image build producing this layout, release signing infrastructure, and
  booting the image (QEMU cannot emulate Pi firmware tryboot; on-device
  tryboot behaviour is a physical qualification item, docs/ACCEPTANCE.md).
- Snapshots cover the control database only; assets/scenes directories
  join when they exist. Watchdog timing is unqualified.
