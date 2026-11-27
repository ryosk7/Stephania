#!/usr/bin/env bash
# Real libwebrtc offer from the iOS sender core (apps/mobile/modules/
# stephania-camera/ios/CameraSender, produced by `swift test` on macOS with
# STEPHANIA_OFFER_OUT) through control signaling into stephania-phone: the device
# must accept it and answer H.264 constrained baseline (any level) for
# both tracks.
# Media is not exchanged here (the offer's candidates are the macOS
# runner's); tests/phone/run-e2e.sh covers media with aiortc.
# Usage: tests/phone/ios-offer.sh <target-dir> <offer.json>
set -euo pipefail
target=$(cd "$1" && pwd)
offer=$(realpath "$2")
root=$(cd "$(dirname "$0")/../.." && pwd)
w=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; wait 2>/dev/null || true; }
trap cleanup EXIT
fail() { echo "FAIL $*"; for f in "$w"/*.log; do echo "--- $f"; tail -40 "$f"; done; exit 1; }
port=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
token=ios-offer-token-0123456789abcdef01234

STEPHANIA_PHONE_SOCKET="$w/phone.sock" STEPHANIA_MEDIA_SOCKET="$w/media.sock" STEPHANIA_PHONE_ADDRS=127.0.0.1 \
  STEPHANIA_PHONE_UDP_PORT=0 "$target/stephania-phone" >"$w/phone.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do [ -S "$w/phone.sock" ] && break; sleep 0.1; done
[ -S "$w/phone.sock" ] || fail "stephania-phone socket"
(cd "$root/services/control" && STEPHANIA_CONTROL_PORT=$port STEPHANIA_CONTROL_TOKEN=$token STEPHANIA_ENGINE_SOCKET="$w/none.sock" \
  STEPHANIA_PHONE_SOCKET="$w/phone.sock" STEPHANIA_CONTROL_DB="$w/c.db" RACK_ENV=production \
  bundle exec puma -C config/puma.rb config.ru >"$w/control.log" 2>&1) & pids+=($!)
for _ in $(seq 100); do curl -s -o /dev/null "http://127.0.0.1:$port/" && break; sleep 0.2; done

python3 - "$port" "$token" "$offer" <<'PY' || fail "offer check"
import json, re, sys, urllib.request
port, token, path = sys.argv[1:]
offer = json.load(open(path))
assert len(offer["tracks"]) == 2 and sorted(offer["tracks"].values()) == ["front", "rear"], offer["tracks"]
def call(method, body=None):
    req = urllib.request.Request(f"http://127.0.0.1:{port}/api/v1/cameras/session" + ("/offer" if body else ""), method=method,
                                 data=json.dumps(body).encode() if body else None,
                                 headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=15) as r:
        return r.status, (json.loads(r.read() or b"null"))
status, answer = call("POST", {"sdp": offer["sdp"], "tracks": offer["tracks"]})
assert status == 200, status
sdp = answer["sdp"]
video = [s for s in re.split(r"\r?\n(?=m=)", sdp) if s.startswith("m=video")]
assert len(video) == 2, f"expected 2 answered video sections:\n{sdp}"
for s in video:
    mline = s.splitlines()[0].split()
    pts = mline[3:]
    assert mline[1] != "0" and pts, f"rejected section:\n{s}"
    fmtp = [l for l in s.splitlines() if l.startswith(f"a=fmtp:{pts[0]} ")]
    plid = re.search(r"profile-level-id=([0-9a-fA-F]{6})", fmtp[0] if fmtp else "")
    # Constrained baseline at any level (profile_idc 0x42, constraint_set1).
    assert plid and plid.group(1)[:2] == "42" and int(plid.group(1)[2:4], 16) & 0x40, s
    assert "packetization-mode=1" in fmtp[0], s
    assert "a=recvonly" in s, s
assert "a=candidate:" in sdp, "answer without candidates"
status, st = call("GET")
assert status == 200 and st["state"] != "none", st
print("ios offer accepted:", {k: v for k, v in st.items() if k != "age_ms"})
PY
echo "PASS iOS sender offer accepted by stephania-phone"
