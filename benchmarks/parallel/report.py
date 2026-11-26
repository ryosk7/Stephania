import json,pathlib,sys,math,re
p=pathlib.Path(sys.argv[1])
target,width,height=map(int,sys.argv[2:5])
rows=[]
failures=[]
previous=None
for i in (1,2):
    c=json.loads((p/f"session-{i}.json").read_text())
    streams=json.loads((p/f"session-{i}.probe.json").read_text())["streams"]
    v=next(s for s in streams if s["codec_name"]=="h264")
    a=next(s for s in streams if s["codec_name"]=="aac")
    span=c["video_media_span_ms"]/1000
    arrival=c["last_video_arrival_s"]-c["first_video_arrival_s"]
    fps=(c["video"]-1)/max(arrival,.001)
    expected=round(span*30)+1
    drop=max(0,expected-c["video"])/max(expected,1)
    skew=abs(c["last_pts_ms"]["audio"]-c["last_pts_ms"]["video"])
    recovery=None if previous is None else c["first_video_arrival_s"]-previous
    previous=c["last_video_arrival_s"]
    checks={"decoded_video":int(v["nb_read_frames"])==c["video"],
            "decoded_audio":int(a["nb_read_frames"])==c["audio"],
            "format":(v["width"],v["height"])==(width,height) and a["sample_rate"]=="48000" and a["channels"]==2,
            "positive_span":span>0 and arrival>0,
            "finite":math.isfinite(fps)}
    if i==2:
        checks.update(frames=c["video"]>=target, receiver_fps=fps>=29.7, estimated_drop=drop<=.03,
                      max_gap=c["video_max_gap_ms"]<=134, end_skew=skew<=100,
                      packet_skew=c["max_packet_timestamp_skew_ms"]<=150,
                      audio_coverage=c["audio"]>=int(span*48000/1024*.9),
                      recovery=recovery is not None and 0<=recovery<=10)
    failures.extend(f"session {i}: {k}" for k,val in checks.items() if not val)
    rows.append(dict(session=i,video=c["video"],audio=c["audio"],receiver_fps=fps,estimated_drop=drop,
                     end_skew_ms=skew,max_packet_skew_ms=c["max_packet_timestamp_skew_ms"],
                     recovery_s=recovery,max_video_gap_ms=c["video_max_gap_ms"],checks=checks))
log=(p/"client.log").read_text()
reconnects=len(re.findall(r"native A/V session failed",log))
if reconnects!=1: failures.append(f"expected one reconnect, got {reconnects}")
result=dict(sessions=rows,reconnects=reconnects,failures=failures)
(p/"integration-report.json").write_text(json.dumps(result,indent=2))
print(json.dumps(result))
sys.exit(bool(failures))
