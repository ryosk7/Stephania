#!/usr/bin/env bash
# Boot a separate ARM64 Linux kernel without touching the runner's kernel.
set -Eeuo pipefail
root="$(pwd)"
mkdir -p vm-evidence
out="$(realpath vm-evidence)"
work="$(mktemp -d)"
pid=
cleanup() {
  if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT
url=https://cloud-images.ubuntu.com/noble/current
curl -fL --retry 3 --max-time 300 "$url/noble-server-cloudimg-arm64.img" -o "$work/base.img"
curl -fL --retry 3 --max-time 60 "$url/SHA256SUMS" -o "$work/SHA256SUMS"
(cd "$work"; grep -E ' [*]?noble-server-cloudimg-arm64.img$' SHA256SUMS | sed 's/noble-server-cloudimg-arm64.img$/base.img/' | sha256sum -c -)
sha256sum "$work/base.img" > "$out/image.sha256"
qemu-img create -f qcow2 -F qcow2 -b "$work/base.img" "$work/guest.qcow2" 16G
ssh-keygen -q -t ed25519 -N '' -f "$work/key"
cat > "$work/user-data" <<EOF
#cloud-config
users:
  - name: lab
    groups: [sudo, audio, video]
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    ssh_authorized_keys:
      - $(cat "$work/key.pub")
ssh_pwauth: false
disable_root: true
EOF
printf 'instance-id: t3box-lab\nlocal-hostname: t3box-lab\n' > "$work/meta-data"
cloud-localds "$work/seed.img" "$work/user-data" "$work/meta-data"
firmware=/usr/share/AAVMF/AAVMF_CODE.fd
test -f "$firmware"
accel=tcg
cpu=cortex-a76
if [[ -c /dev/kvm ]] && [[ "$(uname -m)" == aarch64 ]] && [[ -r /dev/kvm && -w /dev/kvm ]]; then accel=kvm; cpu=host; fi
printf 'accel=%s\ncpu=%s\n' "$accel" "$cpu" | tee "$out/virtualization.txt"
qemu-system-aarch64 -machine virt,accel="$accel" -cpu "$cpu" -smp 2 -m 4096 \
  -bios "$firmware" -nographic -no-reboot \
  -drive if=virtio,file="$work/guest.qcow2",format=qcow2 \
  -drive if=virtio,file="$work/seed.img",format=raw,readonly=on \
  -netdev user,id=net,hostfwd=tcp:127.0.0.1:2222-:22 -device virtio-net-pci,netdev=net \
  > "$out/serial.log" 2>&1 &
pid=$!
ssh_opts=(-i "$work/key" -o UserKnownHostsFile="$work/known_hosts" -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 -o BatchMode=yes -p 2222)
guest() { ssh "${ssh_opts[@]}" lab@127.0.0.1 "$@"; }
ready=0
for i in $(seq 1 180); do
  kill -0 "$pid"
  if guest true 2>/dev/null; then ready=1; break; fi
  sleep 5
done
test "$ready" = 1
guest 'sudo cloud-init status --wait'
guest 'sudo apt-get update -qq && sudo apt-get install -y linux-image-generic linux-modules-extra-$(uname -r) ffmpeg openssl v4l-utils alsa-utils kmod curl python3 libx264-164 libavcodec60 libavutil58 libasound2t64' > "$out/guest-install.log" 2>&1
guest 'uname -a; modinfo vivid; modinfo snd-aloop; sudo modprobe vivid n_devs=1 multiplanar=1; sudo modprobe snd-aloop; v4l2-ctl --list-devices; arecord -l' | tee "$out/guest-devices.txt"
# Use the same host-built ARM64 binary on the same Ubuntu release. No Rust
# compilation or performance claims inside TCG.
tar -czf - tests/virtual-lab target/release/t3box | guest 'mkdir -p ~/lab; tar -xzf - -C ~/lab'
guest 'cd ~/lab && ldd target/release/t3box' | tee "$out/guest-libs.txt"
! grep -q 'not found' "$out/guest-libs.txt"
rc=0
guest 'cd ~/lab && LAB_INPUT=kernel LAB_WIDTH=640 LAB_HEIGHT=360 LAB_BITRATE=1000 LAB_SECONDS=15 bash tests/virtual-lab/run.sh' > "$out/guest-test.log" 2>&1 || rc=$?
guest 'cd ~/lab && tar -czf - evidence' | tar -xzf - -C "$out"
cat "$out/guest-test.log"
exit "$rc"
