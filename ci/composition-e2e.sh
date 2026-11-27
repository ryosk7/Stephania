#!/usr/bin/env bash
# Live composition end-to-end: controller binary with ci-virtual HDMI input,
# a virtual phone:rear track and the default compositor, publishing RTMPS to the
# local test server, while scenes are applied over the engine IPC socket.
# Usage: ci/composition-e2e.sh <t3box binary> [out-dir]
set -Eeuo pipefail
bin=${1:?t3box binary}
out=$(realpath -m "${2:-composition-results}")
mkdir -p "$out"
cert=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; wait 2>/dev/null || true; rm -rf "$cert"; }
trap cleanup EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=T3Box-CI-CA -keyout "$cert/ca.key" -out "$cert/ca.pem" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj /CN=t3box-ci.test -keyout "$cert/server.key" -out "$cert/server.csr" 2>/dev/null
printf 'subjectAltName=DNS:t3box-ci.test\nextendedKeyUsage=serverAuth\n' > "$cert/ext"
openssl x509 -req -in "$cert/server.csr" -CA "$cert/ca.pem" -CAkey "$cert/ca.key" -CAcreateserial -days 1 -extfile "$cert/ext" -out "$cert/server.pem" 2>/dev/null
openssl x509 -in "$cert/ca.pem" -outform der -out "$cert/ca.der"
grep -q '127.0.0.1 t3box-ci.test' /etc/hosts || printf '127.0.0.1 t3box-ci.test\n' | sudo tee -a /etc/hosts >/dev/null
T3BOX_CI_SESSIONS=1 T3BOX_CI_FIRST_LIMIT=1000000 python3 ci/rtmps-test-server.py "$cert" > "$out/server.log" 2>&1 & server=$!; pids+=($server)
for _ in $(seq 50); do [[ -f "$cert/ready" ]] && break; sleep 0.1; done
test -f "$cert/ready"
w=640; h=360
T3BOX_ASSETS_DIR="$cert/assets" T3BOX_CI_VIRTUAL=1 T3BOX_CI_VIRTUAL_PHONE=1 T3BOX_CI_CA_DER="$cert/ca.der" \
  T3BOX_OUTPUT=rtmps://t3box-ci.test:19443/live/test T3BOX_WIDTH=$w T3BOX_HEIGHT=$h T3BOX_FPS=30 T3BOX_BITRATE_KBPS=1500 \
  T3BOX_ENGINE_SOCKET="$cert/engine.sock" "$bin" > "$out/engine.log" 2>&1 & pids+=($!)
for _ in $(seq 100); do [[ -S "$cert/engine.sock" ]] && break; sleep 0.1; done
python3 ci/composition_ipc.py "$cert/engine.sock" "$out/acks.json" "$cert/assets" || { cat "$out/engine.log"; exit 1; }
for _ in $(seq 100); do ls "$cert"/session-1.flv >/dev/null 2>&1 && ! kill -0 "$server" 2>/dev/null && break; sleep 0.1; done
cp "$cert"/session-1.* "$out/" 2>/dev/null || true
test -s "$out/session-1.flv" || { echo "no recording"; cat "$out/server.log" "$out/engine.log"; exit 1; }
python3 ci/composition_check.py "$out/session-1.flv" "$out/acks.json" $w $h
echo "composition end-to-end: passed"
