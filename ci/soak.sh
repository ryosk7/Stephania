#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p soak-results
out="$(realpath soak-results)"
work="$(mktemp -d)"
trap 'rm -rf "$work"; docker rm -f t3box-soak >/dev/null 2>&1 || true' EXIT
ffmpeg -v error -y -f lavfi -i 'testsrc2=size=1920x1080:rate=30' -frames:v 120 -threads 1 -pix_fmt yuyv422 -f rawvideo "$work/input.yuv"
chmod 755 "$work"
docker build -t t3box-bench -f ci/Dockerfile.benchmark .
docker run --name t3box-soak --cpus=4 --memory=4g --memory-swap=4g --network=none \
  --user "$(id -u):$(id -g)" -e T3BOX_CI_SOAK=1 -v "$work:/fixture:ro" -v "$out:/results" t3box-bench \
  /usr/bin/time -v -o /results/resources.txt /app/t3box-ci-encode /fixture/input.yuv /dev/null 324000 6000 ultrafast /dev/null \
  > "$out/performance.json" 2> "$out/progress.log" &
pid=$!
while kill -0 "$pid" 2>/dev/null; do
  docker stats --no-stream --format '{{.CPUPerc}},{{.MemUsage}},{{.PIDs}}' t3box-soak >> "$out/stats.csv" 2>/dev/null || true
  sleep 30
done
wait "$pid"
docker inspect t3box-soak > "$out/container.json"
