#!/usr/bin/env bash
# Phone camera path end to end: aiortc "phone" → control signaling →
# stephania-phone (WebRTC) → engine ingest → H.264 decode → compositor sources.
# Usage: tests/phone/run-e2e.sh <target-dir>   (needs stephania with native-aac,
# stephania-phone; bundle-installed services/control; python3 aiortc)
set -euo pipefail
target=$(cd "$1" && pwd)
root=$(cd "$(dirname "$0")/../.." && pwd)
w=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; wait 2>/dev/null || true; }
trap cleanup EXIT
fail() { echo "FAIL $*"; for f in "$w"/*.log; do echo "--- $f"; tail -40 "$f"; done; exit 1; }
port=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
token=phone-e2e-token-0123456789abcdef0123

STEPHANIA_ENGINE_SOCKET="$w/engine.sock" STEPHANIA_MEDIA_SOCKET="$w/media.sock" STEPHANIA_OUTPUT= \
  "$target/stephania" >"$w/engine.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do [ -S "$w/media.sock" ] && [ -S "$w/engine.sock" ] && break; sleep 0.1; done
[ -S "$w/media.sock" ] || fail "engine ingest socket"
STEPHANIA_PHONE_SOCKET="$w/phone.sock" STEPHANIA_MEDIA_SOCKET="$w/media.sock" STEPHANIA_PHONE_ADDRS=127.0.0.1 \
  STEPHANIA_PHONE_UDP_PORT=0 "$target/stephania-phone" >"$w/phone.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do [ -S "$w/phone.sock" ] && break; sleep 0.1; done
(cd "$root/services/control" && STEPHANIA_CONTROL_PORT=$port STEPHANIA_CONTROL_TOKEN=$token STEPHANIA_ENGINE_SOCKET="$w/engine.sock" \
  STEPHANIA_PHONE_SOCKET="$w/phone.sock" STEPHANIA_CONTROL_DB="$w/c.db" RACK_ENV=production \
  ${STEPHANIA_CONTROL_CMD:-bundle exec puma -C config/puma.rb config.ru} >"$w/control.log" 2>&1) & pids+=($!)
for _ in $(seq 100); do curl -s -o /dev/null "http://127.0.0.1:$port/" && break; sleep 0.2; done
python3 "$root/tests/phone/sender.py" "http://127.0.0.1:$port" "$token" 8 || fail "sender"
