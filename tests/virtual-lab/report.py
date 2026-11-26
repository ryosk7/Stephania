import json, pathlib, sys
p=pathlib.Path(sys.argv[1])
def read(n): return json.loads((p/n).read_text())
i,r,s=(read(n+".json") for n in ("initial","running","stopped"))
c=read("session-1.json")
streams=read("probe.json")["streams"]
v=next(x for x in streams if x["codec_name"]=="h264")
a=next(x for x in streams if x["codec_name"]=="aac")
fps=(c["video"]-1)/(c["last_video_arrival_s"]-c["first_video_arrival_s"])
expected=round(c["video_media_span_ms"]*30/1000)+1
missing=max(0,expected-c["video"])/expected
checks=dict(initial=not i["streaming_requested"] and not i["native_worker_running"],
    running=r["streaming_requested"] and r["native_worker_running"] and not r["ffmpeg_process_running"],
    stopped=not s["streaming_requested"] and not s["native_worker_running"],
    starts=all(read(n+".json")["streaming_requested"] for n in ("start","start-repeat")),
    stops=all(not read(n+".json")["streaming_requested"] for n in ("stop","stop-repeat")),
    decoded_video=int(v["nb_read_frames"])==c["video"]>=210,
    decoded_audio=int(a["nb_read_frames"])==c["audio"]>=300,
    geometry=(v["width"],v["height"])==(1920,1080),
    audio_format=int(a["sample_rate"])==48000 and a["channels"]==2,
    fps=fps>=29.7, missing=missing<=.03, skew=c["max_packet_timestamp_skew_ms"]<=250)
result=dict(checks=checks,receiver_fps=fps,estimated_missing=missing,received_video=c["video"],received_audio=c["audio"],max_packet_skew_ms=c["max_packet_timestamp_skew_ms"])
(p/"report.json").write_text(json.dumps(result,indent=2))
print(json.dumps(result))
sys.exit(not all(checks.values()))
