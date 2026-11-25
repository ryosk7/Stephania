#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p e2e-results
out="$(realpath e2e-results)"
cert="$(mktemp -d)"
server=""
cleanup() {
  if [[ -n "$server" ]]; then kill "$server" 2>/dev/null || true; fi
  sudo tc qdisc del dev lo root 2>/dev/null || true
  rm -rf "$cert"
}
trap cleanup EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=T3Box-CI-CA -keyout "$cert/ca.key" -out "$cert/ca.pem" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj /CN=t3box-ci.test -keyout "$cert/server.key" -out "$cert/server.csr" 2>/dev/null
printf 'subjectAltName=DNS:t3box-ci.test\nextendedKeyUsage=serverAuth\n' > "$cert/ext"
openssl x509 -req -in "$cert/server.csr" -CA "$cert/ca.pem" -CAkey "$cert/ca.key" -CAcreateserial -days 1 -extfile "$cert/ext" -out "$cert/server.pem"
openssl x509 -in "$cert/ca.pem" -outform der -out "$cert/ca.der"
printf '127.0.0.1 t3box-ci.test\n' | sudo tee -a /etc/hosts >/dev/null
python3 ci/rtmps-test-server.py "$cert" > "$out/server.log" 2>&1 &
server=$!
for i in $(seq 1 50); do [[ -f "$cert/ready" ]] && break; sleep 0.1; done
test -f "$cert/ready"
# Restrict shaping to local test traffic on the ephemeral CI runner.
sudo tc qdisc add dev lo root netem delay 20ms 5ms loss 0.1% rate 15mbit
sudo tc -s qdisc show dev lo > "$out/netem.txt"
T3BOX_CI_VIRTUAL=1 T3BOX_CI_CA_DER="$cert/ca.der" \
T3BOX_OUTPUT=rtmps://t3box-ci.test:19443/live/test \
T3BOX_WIDTH=640 T3BOX_HEIGHT=360 T3BOX_FPS=30 T3BOX_BITRATE_KBPS=1000 T3BOX_MAX_FRAMES=120 \
timeout 100 target/release/t3box-native-av > "$out/client.log" 2>&1
wait "$server"
server=""
grep -q 'reconnect in' "$out/client.log"
for i in 1 2; do
  cp "$cert/session-$i.flv" "$cert/session-$i.json" "$out/"
  ffprobe -v error -show_streams -of json "$out/session-$i.flv" > "$out/session-$i.probe.json"
  ffmpeg -v error -xerror -i "$out/session-$i.flv" -f null -
done
python3 - "$out" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
for i in (1,2):
    streams = json.loads((p/f"session-{i}.probe.json").read_text())["streams"]
    assert {s["codec_name"] for s in streams} == {"h264","aac"}
    counts = json.loads((p/f"session-{i}.json").read_text())
    assert counts["video"] >= (30 if i == 1 else 120)
    assert counts["audio"] >= 20
print("PASS: verified TLS, forced disconnect, fresh AVC/AAC headers, keyframe and decoded recovery")
PY
