#!/usr/bin/env bash
set -Eeuo pipefail
cd /work
file target/release/t3box
cargo test
# Prove that the CLI and HTTP controller execute on ARM64 and run a synthetic
# H.264/AAC muxing pipeline. No HDMI or microphone hardware is emulated.
export T3BOX_TEST_SOURCES=1
export T3BOX_WIDTH=640
export T3BOX_HEIGHT=360
export T3BOX_BITRATE_KBPS=700
export T3BOX_OUTPUT=file:/tmp/t3box-ci.flv
target/release/t3box >/tmp/t3box-server.log 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true' EXIT
for i in $(seq 1 30); do
  if curl --fail --silent http://127.0.0.1:8787/health >/dev/null; then break; fi
  sleep 1
done
curl --fail --silent http://127.0.0.1:8787/status | grep -q '"streaming_requested":false'
curl --fail --silent -X POST http://127.0.0.1:8787/stream/start | grep -q '"streaming_requested":true'
sleep 5
curl --fail --silent http://127.0.0.1:8787/status | grep -q '"ffmpeg_process_running":true'
curl --fail --silent -X POST http://127.0.0.1:8787/stream/stop | grep -q '"streaming_requested":false'
test -s /tmp/t3box-ci.flv
test "$(wc -c < /tmp/t3box-ci.flv)" -gt 1000
echo 'ARM64 synthetic video/audio + controller smoke test PASS'
