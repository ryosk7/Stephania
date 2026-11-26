#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p e2e-results
out="$(realpath e2e-results)"
cert="$(mktemp -d)"
server=""
impairment=""
load=""
cleanup() {
  if [[ -n "$impairment" ]]; then kill "$impairment" 2>/dev/null || true; fi
  if [[ -n "$load" ]]; then kill "$load" 2>/dev/null || true; fi
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
grep -q 'reconnect in' "$out/client.log"
for i in 1 2; do
  cp "$cert/session-$i.flv" "$cert/session-$i.json" "$out/"
  ffprobe -v error -count_frames -show_streams -of json "$out/session-$i.flv" > "$out/session-$i.probe.json"
  ffmpeg -v error -xerror -i "$out/session-$i.flv" -f null -
done
python3 - "$out" "$frames" "$width" "$height" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
rows = []
errors = []
data = []
previous = None
for i in (1,2):
    streams = json.loads((p/f"session-{i}.probe.json").read_text())["streams"]
    counts = json.loads((p/f"session-{i}.json").read_text())
    video = next(s for s in streams if s["codec_name"] == "h264")
    audio = next(s for s in streams if s["codec_name"] == "aac")
    span = counts["video_media_span_ms"]/1000
    arrival = counts["last_video_arrival_s"]-counts["first_video_arrival_s"]
    fps = (counts["video"]-1)/max(span, 0.001)
    receive_fps = (counts["video"]-1)/max(arrival, 0.001)
    expected = round(span*30)+1
    missing = max(0, expected-counts["video"])
    drop = missing/max(expected,1)
    end_diff = abs(counts["last_pts_ms"]["audio"]-counts["last_pts_ms"]["video"])
    recovery = counts["first_video_arrival_s"]-previous if previous is not None else None
    previous = counts["last_video_arrival_s"]
    checks = {
        "format": (video["width"],video["height"]) == (int(sys.argv[3]),int(sys.argv[4])) and int(audio["sample_rate"])==48000 and audio["channels"]==2,
        "frames": counts["video"] >= (30 if i==1 else int(sys.argv[2])) and int(video["nb_read_frames"])==counts["video"],
        "audio": counts["audio"]>=20,
        "fps": fps>=29,
        "receiver_fps": receive_fps>=27,
        "estimated_drop": drop<=0.03,
        "timestamp_skew": end_diff<=100 and counts["max_packet_timestamp_skew_ms"]<=250,
        "recovery": recovery is None or recovery<=10,
    }
    bad = [k for k,v in checks.items() if not v]
    errors.extend(f"session {i}: {k}" for k in bad)
    data.append(dict(session=i, media_fps=fps, receiver_fps=receive_fps, estimated_drop_ratio=drop,
                     end_timestamp_diff_ms=end_diff, max_timestamp_skew_ms=counts["max_packet_timestamp_skew_ms"],
                     recovery_s=recovery, media_payload_mbps=counts["video_payload_bytes"]*8/max(span,0.001)/1e6, checks=checks))
    rows.append(f"| {i} | {fps:.2f} | {receive_fps:.2f} | {drop:.2%} | {end_diff} | {counts['max_packet_timestamp_skew_ms']} | {recovery if recovery is not None else 'N/A'} | {'PASS' if not bad else 'FAIL'} |")
report = "\n".join(["# RTMPS receive performance", "", "| Session | Media FPS | Receiver FPS | Est. missing | End skew ms | Max packet skew ms | Recovery s | Gate |",
    "|---|---:|---:|---:|---:|---:|---:|---|", *rows, "",
    "Missing frames are estimated from source timestamp span, not device counters. Recovery is receiver last-to-first video gap. Skew is not lip-sync.",
    *[f"- FAIL: {e}" for e in errors]])+"\n"
(p/"summary.md").write_text(report)
(p/"transport-report.json").write_text(json.dumps({"sessions":data,"failures":errors}, indent=2))
print(report)
sys.exit(bool(errors))
PY
