# A/B updates (RAUC + Raspberry Pi 5 tryboot)

Implements docs/MASTER-ARCHITECTURE.md §8. RAUC does everything it is good
at (X.509-signed verity bundles, compatibility, slot selection, writing,
handlers); Stephania adds only a RAUC custom bootloader backend for the Pi
firmware and a boot-time confirmation.

## Layout (GPT, provisional sizes)
| # | Label | Content |
|---|---|---|
| 1 | `selector` | FAT, `autoboot.txt` only (mounted at `/boot/selector`) |
| 2 | `boot-a` | FAT: kernel, DTBs, `cmdline.txt` (`root=PARTLABEL=rootfs-a rauc.slot=A`) |
| 3 | `boot-b` | same for B |
| 4 | `rootfs-a` | SquashFS, read-only |
| 5 | `rootfs-b` | SquashFS, read-only |
| 6 | `data` | ext4, `/data/stephania` (persistent, shared by both slots) |

RAUC slot groups pair `boot.N` with `rootfs.N`; bootnames `A`/`B` map to
firmware partitions 2/3 (`os/rauc/system.conf`).

Each boot partition's `cmdline.txt` names its own pair, so the update
bundle carries both boot images (`boot-a.vfat`, `boot-b.vfat`). RAUC has
one image per slot class; the bundle's slot-install hook
(`os/rauc/bundle-hook`, `[image.boot] hooks=install`) writes the image
whose cmdline matches the target slot's bootname. A B install therefore
boots `root=PARTLABEL=rootfs-b rauc.slot=B`. `os/release/assemble.sh`
refuses to bundle boot images that do not name their slot, and
`os/release/verify.sh` checks the same in the published bundle.

## Flow
1. `stephania-update bundle.raucb` → `rauc install`. The pre-install handler
   (`stephania-bootctl guard`) refuses while the engine reports
   `streaming_requested` (§8: never update while streaming), and
   `stephania-bootctl check-version` refuses a bundle whose `[update] version`
   is older than `/etc/stephania-version` (anti-rollback: an old, signed but
   vulnerable release cannot be installed again; the same version can be
   reinstalled). Going back to an older release needs the installer.
   Images without a release version (development, CI) have no baseline.
2. RAUC verifies signature (`check-purpose=codesign`) and `compatible`,
   writes the inactive pair, then calls `set-primary B`. The backend does
   **not** switch permanently: it records B as pending and writes
   `[tryboot] boot_partition=3`; `[all]` still names A.
3. `systemctl reboot '0 tryboot'`: the firmware boots partition 3 once.
4. `stephania-update-confirm.service` (`stephania-bootctl confirm`): booted from
   the pending slot via tryboot → local health (engine IPC answers, data
   writable; no network/HDMI dependency) → `rauc status mark-good booted`
   → backend commits `[all] boot_partition=3`. Unhealthy → `mark-bad
   booted` and reboot → firmware returns to A.
5. Any crash, watchdog reset or power loss during the trial: the next
   boot is a normal boot, so the firmware uses `[all]` (A). `confirm` sees
   a pending trial while booted elsewhere and marks it bad.

6. Persistent data: the pre-install handler also takes an online SQLite
   backup of the control database (`stephania-bootctl snapshot`) into
   `/data/stephania/update/snapshot` (directory 0700, file 0600: it holds
   credential hashes). When a trial that booted is rejected (health
   failure, crash, deadline), `stephania-data-restore.service` restores it
   on the committed slot **before** the control plane starts (the old
   version cannot read a schema the trial migrated). It runs on every boot
   and records on the trial boot that the trial ran; an install followed
   by a normal reboot never ran the new version and keeps its data. A
   crashed trial is restored once, also when the later mark-bad fails or
   never runs; the mark-bad does not restore again over data written
   since. The restore never follows links in the control plane's
   directory (new file with O_EXCL|O_NOFOLLOW) and gives the database the
   state directory's owner, so the control plane can write it. The
   snapshot is discarded once restored, after a commit, and after a
   trial rejected without having run.
7. Deadline and watchdog: `stephania-update-deadline.timer` rejects a trial not
   confirmed 10 min after boot; the hardware watchdog
   (`dtparam=watchdog=on`, `os/systemd/10-stephania-watchdog.conf`) turns a
   hung system into a reset, i.e. a normal boot onto the committed slot.

`set-state X good` commits only while actually booted from X, so nothing
but a confirmed trial can change the permanent selection. RAUC's own D-Bus
policy lets every local user call every method; the image narrows it
(`systemd/stephania-maint-rauc.dbus.conf`): other users may only read the
status, `stephania-maint` may also call `InstallBundle`, and only root may
`Mark` slots (stephania-bootctl after its health check). State
(`pending`, bad slots) is a small JSON file on the data partition; every
file is replaced atomically (write, fsync, rename, fsync dir).

## Verified in CI (`.github/workflows/os.yml`, `ci/update-e2e.sh`)
Real RAUC v1.15 (built from source) on a 6.8 kernel. Found while setting this up: on kernels whose dm-verity status carries an FEC field (`V -`), RAUC v1.15 refuses to mount verity bundles; the image must pin a kernel/RAUC pair that agree (risk register) with verity bundles on loop devices, test-only
signing PKI generated per run, emulated tryboot firmware:
untrusted signer refused; incompatible bundle refused; install refused
while streaming; older release refused (anti-rollback); RAUC killed
mid-write leaves A committed and bootable; each boot slot receives the
boot image naming its own rootfs and slot (production bundle hook);
snapshot private (0700/0600); healthy trial committed; unhealthy trial
rolled back and marked bad;
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
