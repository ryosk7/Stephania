import json, pathlib, sys, math
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
        "measurement": span > 0 and arrival > 0 and all(math.isfinite(x) for x in (span,arrival,fps,receive_fps)) and (recovery is None or recovery >= 0),
        "max_video_gap": counts["video_max_gap_ms"] <= 134,
        "audio_coverage": counts["audio"] >= max(20, int(span*48000/1024*0.90)) and int(audio["nb_read_frames"]) == counts["audio"],
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
