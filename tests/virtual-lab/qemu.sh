#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p vm-evidence
out="$(realpath vm-evidence)"
cache="$(realpath "${LAB_CACHE:-.qemu-lab}")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
(cd "$cache"; sha256sum -c manifest.txt)
mkdir -p "$work/root/bin"
bin=target/release/stephania-native-probe
install -m 755 "$bin" "$work/root/bin/stephania-native-probe"
while read -r lib; do [[ -z "$lib" ]] || cp -L --parents "$lib" "$work/root"; done < <(ldd "$bin" | awk '/=> \/|^[[:space:]]*\// {for(i=1;i<=NF;i++) if($i ~ /^\//) {print $i;break}}')
python3 - "$work/root/tone.raw" <<'PY'
import math,struct,sys
period=b''.join(struct.pack('<hh',int(10000*math.sin(2*math.pi*440*i/48000)),int(10000*math.sin(2*math.pi*440*i/48000))) for i in range(48000))
open(sys.argv[1],'wb').write(period*10)
PY
cat > "$work/root/init" <<'INIT'
#!/bin/sh
export PATH=/bin:/sbin:/usr/bin:/usr/sbin
set -eu
trap 'echo LAB_FAIL; busybox dmesg | busybox tail -80; poweroff -f' EXIT
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev
uname -a
/bin/modprobe vivid n_devs=1 multiplanar=1
/bin/modprobe snd-aloop
v4l2-ctl --list-devices
v4l2-ctl -d /dev/video0 --list-inputs
v4l2-ctl -d /dev/video0 --set-input=0
v4l2-ctl -d /dev/video0 --list-formats-ext
v4l2-ctl -d /dev/video0 --set-fmt-video=width=320,height=180,pixelformat=YUYV --set-parm=30
v4l2-ctl -d /dev/video0 --get-fmt-video --get-parm
aplay -l
aplay -D hw:Loopback,0,0 -t raw -f S16_LE -c 2 -r 48000 /tone.raw &
feed=$!
export STEPHANIA_VIDEO=/dev/video0 STEPHANIA_AUDIO=hw:Loopback,1,0 STEPHANIA_WIDTH=320 STEPHANIA_HEIGHT=180
timeout 12 stephania-native-probe >/tmp/probe.log 2>&1 || { cat /tmp/probe.log; exit 1; }
cat /tmp/probe.log
kill "$feed" 2>/dev/null || true
echo LAB_PASS
trap - EXIT
poweroff -f
INIT
chmod +x "$work/root/init"
(cd "$work/root"; find . -print0 | cpio --null -o -H newc --quiet | gzip -1) > "$work/probe.cpio.gz"
cat "$cache/root.cpio.gz" "$work/probe.cpio.gz" > "$work/initramfs"
accel=tcg
cpu=cortex-a76
if [[ -c /dev/kvm && -r /dev/kvm && -w /dev/kvm ]] && [[ "$(uname -m)" == aarch64 ]]; then accel=kvm; cpu=host; fi
printf 'accel=%s\ncpu=%s\n' "$accel" "$cpu" | tee "$out/virtualization.txt"
timeout --kill-after=2s 45s qemu-system-aarch64 -machine virt,accel="$accel" -cpu "$cpu" -smp 2 -m 512 -nographic -no-reboot -nic none -kernel "$cache/Image" -initrd "$work/initramfs" -append 'console=ttyAMA0 rdinit=/init panic=1 quiet' |& tee "$out/serial.log"
grep -q '^LAB_PASS' "$out/serial.log"
! grep -q '^LAB_FAIL' "$out/serial.log"
python3 - "$out/serial.log" <<'PY'
import re,sys
s=open(sys.argv[1]).read()
v=re.findall(r'^video seq=(\d+) time=(\d+)us 320x180 format=Yuyv422 bytes=(\d+)',s,re.M)
a=re.findall(r'^audio time=(\d+)us rate=48000 channels=2 samples=(\d+) peak=([-\d.]+)dBFS rms=([-\d.]+)dBFS clipped=(\d+)',s,re.M)
assert len(v)==20 and len(a)==40
assert all(int(x[2])==320*180*2 for x in v)
assert all(int(b[0])>int(a[0]) for a,b in zip(v,v[1:]))
assert all(int(x[1])>0 and int(x[4])==0 for x in a)
assert sum(float(x[3])>-40 for x in a)>=30
print('Native V4L2/ALSA payload checks PASS (functional only)')
PY
