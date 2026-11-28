#!/usr/bin/env bash
# Structural checks of the Raspberry Pi 5 image variant (it cannot boot in
# QEMU): boot slots carry what the Pi firmware loads, and the kernel,
# device tree and command line are the ones the slot expects.
# Usage: ci/pi-image-check.sh <image-out-dir>
set -euo pipefail
d=$(cd "$1" && pwd)
fail() { echo "FAIL $*"; exit 1; }
for slot in a b; do
  upper=$(echo "$slot" | tr a-z A-Z)
  files=$(mdir -b -i "$d/boot-$slot.vfat" ::/)
  for f in vmlinuz initrd.img bcm2712-rpi-5-b.dtb config.txt cmdline.txt overlays; do
    # mdir -b lists a directory with a trailing slash.
    echo "$files" | grep -qxE "::/$f/?" || fail "boot-$slot lacks $f"
  done
  n=$(mdir -b -i "$d/boot-$slot.vfat" ::/overlays | grep -c '\.dtbo$' || true)
  [ "$n" -gt 0 ] || fail "boot-$slot overlays directory is empty"
  cfg=$(mtype -i "$d/boot-$slot.vfat" ::/config.txt)
  for l in arm_64bit=1 kernel=vmlinuz 'initramfs initrd.img followkernel' dtparam=watchdog=on; do
    echo "$cfg" | grep -qx "$l" || fail "boot-$slot config.txt lacks '$l'"
  done
  cmd=$(mtype -i "$d/boot-$slot.vfat" ::/cmdline.txt)
  echo "$cmd" | grep -q "root=PARTLABEL=rootfs-$slot " && echo "$cmd" | grep -q "rauc.slot=$upper" && echo "$cmd" | grep -q rootwait \
    || fail "boot-$slot cmdline: $cmd"
done
echo "PASS both boot slots carry kernel, initramfs, Pi 5 DTB, overlays, config.txt and their own cmdline"
python3 - "$d" <<'PY'
import gzip, sys
d = sys.argv[1]
raw = open(f"{d}/vmlinuz", "rb").read()
img = gzip.decompress(raw) if raw[:2] == b"\x1f\x8b" else raw
assert img[56:60] == b"ARM\x64", "vmlinuz is not an arm64 Image"
dtb = open(f"{d}/bcm2712-rpi-5-b.dtb", "rb").read()
assert dtb[:4] == b"\xd0\x0d\xfe\xed" and b"raspberrypi,5-model-b" in dtb, "not a Pi 5 device tree"
print("PASS kernel is an arm64 Image; DTB is the Raspberry Pi 5 Model B tree")
PY
# Raspberry Pi OS reference: the Pi 5 kernel from the Raspberry Pi archive
# (versions carry "+rpt"), on Debian trixie.
awk -F'\t' '$1 ~ /^linux-image-.*rpi-2712$/ && $2 ~ /rpt/ {found=1} END {exit !found}' "$d/dpkg.tsv" || fail "Raspberry Pi 2712 kernel package not installed"
grep -q $'^base-files\t13' "$d/dpkg.tsv" || fail "not Debian 13 (trixie) base-files"
echo "PASS Raspberry Pi 5 kernel from the Raspberry Pi archive on Debian 13"
# First-run path and clock: the BLE adapter and the NTP client ship enabled.
r="$d/rootfs"
[ -x "$r/usr/local/bin/stephania-ble" ] || fail "stephania-ble not installed"
[ -L "$r/etc/systemd/system/multi-user.target.wants/stephania-ble.service" ] || fail "stephania-ble.service not enabled"
awk -F'\t' '$1 == "systemd-timesyncd" {found=1} END {exit !found}' "$d/dpkg.tsv" || fail "systemd-timesyncd not installed"
ls "$r/etc/systemd/system/"*.wants/systemd-timesyncd.service >/dev/null 2>&1 || fail "systemd-timesyncd not enabled"
echo "PASS stephania-ble and systemd-timesyncd installed and enabled"
