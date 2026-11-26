#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p "${T3BOX_CI_OUT:-e2e-results}"
out="$(realpath "${T3BOX_CI_OUT:-e2e-results}")"
cert="$(mktemp -d)"
server=""
impairment=""
load=""
cleanup() {
  if [[ -n "$impairment" ]]; then kill "$impairment" 2>/dev/null || true; wait "$impairment" 2>/dev/null || true; fi
  if [[ -n "$load" ]]; then kill "$load" 2>/dev/null || true; wait "$load" 2>/dev/null || true; fi
  if [[ -n "$server" ]]; then kill "$server" 2>/dev/null || true; wait "$server" 2>/dev/null || true; fi
  for file in "$cert"/session-*.flv "$cert"/session-*.json; do [[ ! -f "$file" ]] || cp "$file" "$out/"; done
  sudo tc qdisc del dev lo root 2>/dev/null || true
  rm -rf "$cert"
}
trap cleanup EXIT
trap 'exit 124' TERM INT
export LC_ALL=C
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=T3Box-CI-CA -keyout "$cert/ca.key" -out "$cert/ca.pem" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj /CN=t3box-ci.test -keyout "$cert/server.key" -out "$cert/server.csr" 2>/dev/null
printf 'subjectAltName=DNS:t3box-ci.test\nextendedKeyUsage=serverAuth\n' > "$cert/ext"
openssl x509 -req -in "$cert/server.csr" -CA "$cert/ca.pem" -CAkey "$cert/ca.key" -CAcreateserial -days 1 -extfile "$cert/ext" -out "$cert/server.pem"
openssl x509 -in "$cert/ca.pem" -outform der -out "$cert/ca.der"
grep -q '127.0.0.1 t3box-ci.test' /etc/hosts || printf '127.0.0.1 t3box-ci.test\n' | sudo tee -a /etc/hosts >/dev/null
python3 ci/rtmps-test-server.py "$cert" > "$out/server.log" 2>&1 &
server=$!
for i in $(seq 1 50); do [[ -f "$cert/ready" ]] && break; sleep 0.1; done
test -f "$cert/ready"
# Restrict shaping to local test traffic on the ephemeral CI runner.
sudo tc qdisc add dev lo root netem delay 20ms 5ms loss 0.1% rate 15mbit
sudo tc -s qdisc show dev lo > "$out/netem.txt"
width="${T3BOX_CI_WIDTH:-640}"
height="${T3BOX_CI_HEIGHT:-360}"
bitrate="${T3BOX_CI_BITRATE:-1000}"
frames="${T3BOX_CI_FRAMES:-120}"
if [[ "${T3BOX_CI_DYNAMIC:-0}" == 1 ]]; then
  (
    sleep 2
    sudo tc qdisc change dev lo root netem delay 60ms 15ms loss 0.3% rate 12mbit
    sudo tc -s qdisc show dev lo
    sleep 3
    sudo tc qdisc change dev lo root netem delay 20ms 5ms loss 0.1% rate 15mbit
    sudo tc -s qdisc show dev lo
  ) > "$out/dynamic-netem.txt" 2>&1 &
  impairment=$!
fi
if [[ "${T3BOX_CI_LOAD:-0}" == 1 ]]; then
  stress-ng --cpu 1 --cpu-load 25 --vm 1 --vm-bytes 128M --timeout 90s > "$out/stress.log" 2>&1 &
  load=$!
fi
T3BOX_CI_VIRTUAL=1 T3BOX_CI_CA_DER="$cert/ca.der" \
T3BOX_OUTPUT=rtmps://t3box-ci.test:19443/live/test \
T3BOX_WIDTH="$width" T3BOX_HEIGHT="$height" T3BOX_FPS=30 T3BOX_BITRATE_KBPS="$bitrate" T3BOX_MAX_FRAMES="$frames" \
/usr/bin/time -v -o "$out/client-resource.txt" timeout 45 target/release/t3box-native-av > "$out/client.log" 2>&1
wait "$server"
server=""
if [[ -n "$impairment" ]]; then wait "$impairment"; impairment=""; fi
if [[ -n "$load" ]]; then kill -0 "$load"; fi
sudo tc -s qdisc show dev lo > "$out/netem-final.txt"
grep -q 'reconnect in' "$out/client.log"
for i in 1 2; do
  cp "$cert/session-$i.flv" "$cert/session-$i.json" "$out/"
  ffprobe -v error -count_frames -show_streams -of json "$out/session-$i.flv" > "$out/session-$i.probe.json"
  ffmpeg -v error -xerror -i "$out/session-$i.flv" -f null -
done
python3 ci/report-transport.py "$out" "$frames" "$width" "$height"
