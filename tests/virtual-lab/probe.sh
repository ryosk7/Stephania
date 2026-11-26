#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p evidence
uname -a > evidence/kernel.txt
lscpu > evidence/cpu.txt
cat /proc/meminfo > evidence/memory.txt
for module in vivid v4l2loopback snd-aloop; do
  {
    modinfo "$module" || true
    sudo modprobe "$module"
  } > "evidence/$module.log" 2>&1 && status=available || status=unavailable
  printf '%s=%s\n' "$module" "$status" | tee -a evidence/capabilities.txt
done
v4l2-ctl --list-devices > evidence/video-devices.txt 2>&1 || true
arecord -l > evidence/audio-devices.txt 2>&1 || true
ls -l /dev/video* /dev/snd/* > evidence/device-nodes.txt 2>&1 || true
if grep -q '^vivid=available$' evidence/capabilities.txt && grep -q '^snd-aloop=available$' evidence/capabilities.txt; then
  echo "Kernel device prerequisites available; integration still required."
else
  echo "Kernel device integration unavailable on this runner."
  exit 1
fi
