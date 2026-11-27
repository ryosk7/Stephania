#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p soak-results
out="$(realpath soak-results)"
work="$(mktemp -d)"
trap 'rm -rf "$work"; docker rm -f stephania-soak >/dev/null 2>&1 || true' EXIT
trap 'exit 124' TERM INT
export LC_ALL=C
ffmpeg -v error -y -f lavfi -i 'testsrc2=size=1920x1080:rate=30' -frames:v 120 -threads 1 -pix_fmt yuyv422 -f rawvideo "$work/input.yuv"
chmod 755 "$work"
cpus="$(nproc)"
if (( cpus > 4 )); then cpus=4; fi
printf "%s\n" "$cpus" > "$out/cpu-quota.txt"
bash ci/images/benchmark.sh
docker run --name stephania-soak --cpus="$cpus" --memory=4g --memory-swap=4g --network=none \
  --user "$(id -u):$(id -g)" -e STEPHANIA_CI_SOAK=1 -v "$work:/fixture:ro" -v "$out:/results" stephania-bench \
  /usr/bin/time -v -o /results/resources.txt /app/stephania-ci-encode /fixture/input.yuv /dev/null 324000 6000 ultrafast /dev/null \
  > "$out/performance.json" 2> "$out/progress.log" &
pid=$!
while kill -0 "$pid" 2>/dev/null; do
  docker stats --no-stream --format '{{.CPUPerc}},{{.MemUsage}},{{.PIDs}}' stephania-soak >> "$out/stats.csv" 2>/dev/null || true
  sleep 30
done
rc=0
wait "$pid" || rc=$?
docker inspect stephania-soak > "$out/container.json"
python3 - "$out" "$rc" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1]); assert int(sys.argv[2])==0
x=json.loads((p/'performance.json').read_text()); state=json.loads((p/'container.json').read_text())[0]['State']
assert x['frames']==324000 and 506250<=x['audio_packets']<=506252
assert 10790<=x['seconds']<=10815 and not state['OOMKilled'] and state['ExitCode']==0
assert len((p/'stats.csv').read_text().splitlines())>=300
print('Paced codec soak passed; not transport or leak proof')
PY
