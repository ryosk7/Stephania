#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p "${LAB_OUT:-evidence}"
out="$(realpath "${LAB_OUT:-evidence}")"
[[ -z "$(ls -A "$out")" ]] || { echo "evidence must be empty"; exit 2; }
work="$(mktemp -d)"
peer= controller= feeder= monitor=
cleanup() {
  for pid in "$monitor" "$controller" "$peer" "$feeder"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
  for f in "$work"/session-*; do [[ ! -f "$f" ]] || cp "$f" "$out/"; done
  if [[ "${LAB_NETWORK:-clean}" == venue ]]; then sudo tc qdisc del dev lo root 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=T3Box-Lab-CA -keyout "$work/ca.key" -out "$work/ca.pem" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj /CN=t3box-ci.test -keyout "$work/server.key" -out "$work/server.csr" 2>/dev/null
printf 'subjectAltName=DNS:t3box-ci.test\nextendedKeyUsage=serverAuth\n' > "$work/ext"
openssl x509 -req -in "$work/server.csr" -CA "$work/ca.pem" -CAkey "$work/ca.key" -CAcreateserial -days 1 -extfile "$work/ext" -out "$work/server.pem"
openssl x509 -in "$work/ca.pem" -outform der -out "$work/ca.der"
grep -q '127.0.0.1 t3box-ci.test' /etc/hosts || printf '127.0.0.1 t3box-ci.test\n' | sudo tee -a /etc/hosts >/dev/null
python3 tests/virtual-lab/receiver.py "$work" > "$out/receiver.log" 2>&1 &
peer=$!
for i in $(seq 1 50); do [[ -f "$work/ready" ]] && break; sleep .1; done
test -f "$work/ready"
export T3BOX_CI_CA_DER="$work/ca.der" T3BOX_OUTPUT=rtmps://t3box-ci.test:19443/live/lab
export T3BOX_WIDTH="${LAB_WIDTH:-1920}" T3BOX_HEIGHT="${LAB_HEIGHT:-1080}" T3BOX_FPS=30 T3BOX_BITRATE_KBPS="${LAB_BITRATE:-6000}"
export T3BOX_PARALLEL_MEDIA=1 T3BOX_ADAPTIVE_RATE=1 T3BOX_DEADLINE_TRANSPORT=1 T3BOX_CONTENT_AWARE=1
if [[ "${LAB_INPUT:-virtual}" == physical ]]; then
  unset T3BOX_CI_VIRTUAL
  export T3BOX_VIDEO="${T3BOX_VIDEO:-/dev/video0}" T3BOX_AUDIO="${T3BOX_AUDIO:-plughw:1,0}"
elif [[ "${LAB_INPUT:-virtual}" == kernel ]]; then
  unset T3BOX_CI_VIRTUAL
  sudo modprobe vivid n_devs=1 multiplanar=1
  sudo modprobe snd-aloop
  export T3BOX_VIDEO="$(v4l2-ctl --list-devices | awk '/vivid/{f=1;next} f && /\/dev\/video/{print $1;exit}')"
  test -n "$T3BOX_VIDEO"
  test -r "$T3BOX_VIDEO" && test -w "$T3BOX_VIDEO"
  arecord -l > "$out/audio-devices.txt"
  v4l2-ctl --list-devices > "$out/video-devices.txt"
  v4l2-ctl -d "$T3BOX_VIDEO" --set-fmt-video=width="$T3BOX_WIDTH",height="$T3BOX_HEIGHT",pixelformat=YUYV --set-parm=30
  v4l2-ctl -d "$T3BOX_VIDEO" --get-fmt-video --get-parm > "$out/video-format.txt"
  export T3BOX_AUDIO=hw:Loopback,1,0
  ffmpeg -nostdin -re -f lavfi -i 'sine=frequency=440:sample_rate=48000' -ac 2 -f alsa hw:Loopback,0,0 > "$out/audio-feed.log" 2>&1 &
  feeder=$!
  sleep 1
  kill -0 "$feeder"
else
  export T3BOX_CI_VIRTUAL=1
  if [[ "${LAB_MEDIA:-synthetic}" != synthetic ]]; then
    python3 tests/virtual-lab/fixture.py "$work"
    export T3BOX_LAB_VIDEO="$work/mixed.yuyv" T3BOX_LAB_MARKERS=1
    cp "$work/fixture.json" "$out/"
  fi
fi
if [[ "${LAB_NETWORK:-clean}" == venue ]]; then sudo tc qdisc replace dev lo root netem delay 40ms 10ms loss 0.2% rate 12mbit; fi
target/release/t3box > "$out/controller.log" 2>&1 &
controller=$!
python3 tests/virtual-lab/monitor.py "$controller" "$out" &
monitor=$!
api() { curl --fail --silent --max-time 4 "$@"; }
for i in $(seq 1 50); do api http://127.0.0.1:8787/health > "$out/health.json" && break; sleep .1; done
api http://127.0.0.1:8787/status > "$out/initial.json"
test "$(curl -s -o /dev/null -w '%{http_code}' -H 'Origin: https://example.invalid' -X POST http://127.0.0.1:8787/stream/start)" = 403
test "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8787/unknown)" = 404
api -X POST http://127.0.0.1:8787/stream/start > "$out/start.json"
api -X POST http://127.0.0.1:8787/stream/start > "$out/start-repeat.json"
for i in $(seq 1 100); do
  [[ -f "$work/session-1.flv" ]] && [[ $(stat -c%s "$work/session-1.flv") -gt 20000 ]] && break
  kill -0 "$controller"
  sleep .1
done
test -s "$work/session-1.flv"
sleep "${LAB_SECONDS:-15}"
api http://127.0.0.1:8787/status > "$out/running.json"
api -X POST http://127.0.0.1:8787/stream/stop > "$out/stop.json"
api -X POST http://127.0.0.1:8787/stream/stop > "$out/stop-repeat.json"
for i in $(seq 1 60); do
  api http://127.0.0.1:8787/status > "$out/stopped.json"
  python3 -c 'import json,sys;s=json.load(open(sys.argv[1]));assert not s["native_worker_running"] and not s["streaming_requested"]' "$out/stopped.json" 2>/dev/null && break
  sleep .1
done
for i in $(seq 1 100); do [[ -f "$work/session-1.json" ]] && break; sleep .1; done
test -f "$work/session-1.json"
touch "$work/done"
wait "$peer"
peer=
cp "$work"/session-* "$out/"
for f in "$out"/session-*.flv; do
  ffprobe -v error -count_frames -show_streams -of json "$f" > "${f%.flv}.probe.json"
  ffmpeg -v error -xerror -i "$f" -f null -
done
rc=0
python3 tests/virtual-lab/report.py "$out" || rc=1
if [[ "${LAB_MEDIA:-synthetic}" != synthetic ]]; then python3 tests/virtual-lab/markers.py "$out" "$work/mixed.yuyv" || rc=1; fi
exit "$rc"
