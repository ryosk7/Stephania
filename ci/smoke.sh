#!/usr/bin/env bash
set -Eeuo pipefail
cd "${T3BOX_ROOT:-/work}"
out="$(realpath -m "${T3BOX_SMOKE_OUT:-transport-results/controller}")"
mkdir -p "$out"
work="$(mktemp -d)"
file target/release/t3box
export T3BOX_TEST_SOURCES=1 T3BOX_WIDTH=640 T3BOX_HEIGHT=360 T3BOX_FPS=30 T3BOX_BITRATE_KBPS=700
export T3BOX_OUTPUT="file:$work/capture.flv"
target/release/t3box >"$out/server.log" 2>&1 &
pid=$!
cleanup() {
  curl --max-time 2 -s -X POST http://127.0.0.1:8787/stream/stop >/dev/null || true
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 124' TERM INT
api() { curl --max-time 3 --fail --silent "$@"; }
for i in $(seq 1 30); do
  if api http://127.0.0.1:8787/health >"$out/health.json"; then break; fi
  kill -0 "$pid"
  sleep 0.1
done
python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["ok"] is True' "$out/health.json"
api http://127.0.0.1:8787/status >"$out/initial.json"
python3 -c 'import json,sys; s=json.load(open(sys.argv[1])); assert not s["streaming_requested"] and not s["ffmpeg_process_running"]' "$out/initial.json"
code="$(curl --max-time 3 -s -o "$out/denied.json" -w '%{http_code}' -H 'Origin: https://example.invalid' -X POST http://127.0.0.1:8787/stream/start)"
test "$code" = 403
code="$(curl --max-time 3 -s -o "$out/unknown.json" -w '%{http_code}' http://127.0.0.1:8787/unknown)"
test "$code" = 404
api -X POST http://127.0.0.1:8787/stream/start >"$out/start.json"
api -X POST http://127.0.0.1:8787/stream/start >"$out/start-repeat.json"
for i in $(seq 1 30); do
  if [[ -s "$work/capture.flv" ]] && (( $(wc -c < "$work/capture.flv") > 20000 )); then break; fi
  sleep 0.1
done
sleep 1
api http://127.0.0.1:8787/status >"$out/running.json"
api -X POST http://127.0.0.1:8787/stream/stop >"$out/stop.json"
api -X POST http://127.0.0.1:8787/stream/stop >"$out/stop-repeat.json"
api http://127.0.0.1:8787/status >"$out/stopped.json"
ffprobe -v error -count_frames -show_streams -of json "$work/capture.flv" >"$out/probe.json"
ffmpeg -v error -xerror -i "$work/capture.flv" -f null -
python3 - "$out" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1])
def read(name): return json.loads((p/name).read_text())
for name in ("start.json","start-repeat.json"):
    assert read(name)["streaming_requested"] is True
for name in ("stop.json","stop-repeat.json"):
    assert read(name)["streaming_requested"] is False
s=read("running.json")
assert s["streaming_requested"] and s["ffmpeg_process_running"] and not s["youtube_ingest_confirmed"]
s=read("stopped.json")
assert not s["streaming_requested"] and not s["ffmpeg_process_running"]
streams=read("probe.json")["streams"]
v=next(s for s in streams if s["codec_name"]=="h264")
a=next(s for s in streams if s["codec_name"]=="aac")
assert (v["width"],v["height"])==(640,360) and int(v["nb_read_frames"])>=15
assert int(a["sample_rate"])==48000 and int(a["nb_read_frames"])>=10
print("Controller HTTP, start/stop idempotence, cross-origin rejection and decoded media PASS")
PY
