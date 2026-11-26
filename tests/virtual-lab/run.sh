#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p evidence
out="$(realpath evidence)"
work="$(mktemp -d)"
peer= controller= feeder=
cleanup() {
  for pid in "$controller" "$peer" "$feeder"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
  for f in "$work"/session-*; do [[ ! -f "$f" ]] || cp "$f" "$out/"; done
  rm -rf "$work"
}
trap cleanup EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=T3Box-Lab-CA -keyout "$work/ca.key" -out "$work/ca.pem" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj /CN=t3box-ci.test -keyout "$work/server.key" -out "$work/server.csr" 2>/dev/null
printf 'subjectAltName=DNS:t3box-ci.test\nextendedKeyUsage=serverAuth\n' > "$work/ext"
openssl x509 -req -in "$work/server.csr" -CA "$work/ca.pem" -CAkey "$work/ca.key" -CAcreateserial -days 1 -extfile "$work/ext" -out "$work/server.pem"
openssl x509 -in "$work/ca.pem" -outform der -out "$work/ca.der"
printf '127.0.0.1 t3box-ci.test\n' | sudo tee -a /etc/hosts >/dev/null
T3BOX_CI_SESSIONS=1 python3 tests/virtual-lab/receiver.py "$work" > "$out/receiver.log" 2>&1 &
peer=$!
for i in $(seq 1 50); do [[ -f "$work/ready" ]] && break; sleep .1; done
test -f "$work/ready"
export T3BOX_CI_CA_DER="$work/ca.der" T3BOX_OUTPUT=rtmps://t3box-ci.test:19443/live/lab
export T3BOX_WIDTH=1920 T3BOX_HEIGHT=1080 T3BOX_FPS=30 T3BOX_BITRATE_KBPS=6000
export T3BOX_PARALLEL_MEDIA=1 T3BOX_ADAPTIVE_RATE=1 T3BOX_DEADLINE_TRANSPORT=1 T3BOX_CONTENT_AWARE=1
if [[ "${LAB_INPUT:-virtual}" == kernel ]]; then
  unset T3BOX_CI_VIRTUAL
  sudo modprobe vivid n_devs=1 multiplanar=1
  sudo modprobe snd-aloop
  export T3BOX_VIDEO="$(v4l2-ctl --list-devices | awk '/vivid/{f=1;next} f && /\/dev\/video/{print $1;exit}')"
  test -n "$T3BOX_VIDEO"
  sudo chmod a+rw "$T3BOX_VIDEO" /dev/snd/*
  v4l2-ctl -d "$T3BOX_VIDEO" --set-fmt-video=width=1920,height=1080,pixelformat=YUYV --set-parm=30
  v4l2-ctl -d "$T3BOX_VIDEO" --get-fmt-video --get-parm > "$out/video-format.txt"
  export T3BOX_AUDIO=hw:Loopback,1,0
  ffmpeg -nostdin -re -f lavfi -i 'sine=frequency=440:sample_rate=48000' -ac 2 -f alsa hw:Loopback,0,0 > "$out/audio-feed.log" 2>&1 &
  feeder=$!
else
  export T3BOX_CI_VIRTUAL=1
fi
target/release/t3box > "$out/controller.log" 2>&1 &
controller=$!
api() { curl --fail --silent --max-time 4 "$@"; }
for i in $(seq 1 50); do api http://127.0.0.1:8787/health > "$out/health.json" && break; sleep .1; done
api http://127.0.0.1:8787/status > "$out/initial.json"
test "$(curl -s -o /dev/null -w '%{http_code}' -H 'Origin: https://example.invalid' -X POST http://127.0.0.1:8787/stream/start)" = 403
api -X POST http://127.0.0.1:8787/stream/start > "$out/start.json"
api -X POST http://127.0.0.1:8787/stream/start > "$out/start-repeat.json"
for i in $(seq 1 100); do
  [[ -f "$work/session-1.flv" ]] && [[ $(stat -c%s "$work/session-1.flv") -gt 20000 ]] && break
  kill -0 "$controller"
  sleep .1
done
test -s "$work/session-1.flv"
sleep 8
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
wait "$peer"
peer=
cp "$work"/session-1.* "$out/"
ffprobe -v error -count_frames -show_streams -of json "$out/session-1.flv" > "$out/probe.json"
ffmpeg -v error -xerror -i "$out/session-1.flv" -f null -
python3 tests/virtual-lab/report.py "$out"
