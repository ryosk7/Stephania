#!/usr/bin/env bash
# Boots the reference image in QEMU (generic kernel, virtio disk) and runs
# the in-image smoke test. Usage: ci/qemu-smoke.sh <image-out-dir> <arm64|amd64>
set -euo pipefail
d=$1 arch=$2
log="$d/serial.log"
cmd=(-m 1536 -smp 2 -nographic -no-reboot
  -kernel "$d/vmlinuz" -initrd "$d/initrd.img"
  -drive "file=$d/stephania.img,if=virtio,format=raw"
  -netdev user,id=n0 -device virtio-net-pci,netdev=n0)
append="root=/dev/vda4 ro rootfstype=squashfs init=/usr/local/sbin/stephania-overlay-init rauc.slot=A stephania.smoke=1 panic=10"
accel=tcg; [ -w /dev/kvm ] && accel=kvm
case $arch in
  arm64) q=qemu-system-aarch64; cmd=(-M virt -cpu "$([ $accel = kvm ] && echo host || echo max)" -accel $accel "${cmd[@]}"); append="$append console=ttyAMA0" ;;
  amd64) q=qemu-system-x86_64; cmd=(-M q35 -accel $accel "${cmd[@]}"); append="$append console=ttyS0" ;;
esac
echo "booting with $q ($accel)"
timeout 1500 "$q" "${cmd[@]}" -append "$append" >"$log" 2>&1 || true
grep -E "SMOKE (PASS|FAIL|INFO|RESULT)" "$log" || { tail -80 "$log"; exit 1; }
grep -q "SMOKE RESULT PASS" "$log" || { echo "--- serial tail"; tail -60 "$log"; exit 1; }
