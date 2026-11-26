"""Independent decoded sequence markers and PCM pulse measurement.
Audio-video offset is measured in decoded media PTS, not optical sync.
Absolute capture latency is not inferred without a common source epoch.
"""
import pathlib,sys,subprocess,json,array,statistics
p=pathlib.Path(sys.argv[1]);rows=[]
for f in sorted(p.glob('session-*.flv')):
    raw=subprocess.check_output(['ffmpeg','-v','error','-i',str(f),'-an','-vf','crop=512:16:0:0,scale=32:1:flags=area,format=gray','-fps_mode','passthrough','-f','rawvideo','-'])
    seq=[sum((raw[i+b]>128)<<b for b in range(32)) for i in range(0,len(raw),32)]
    pts=json.loads(subprocess.check_output(['ffprobe','-v','error','-select_streams','v:0','-show_frames','-show_entries','frame=best_effort_timestamp_time','-of','json',str(f)]))['frames']
    assert len(seq)==len(pts) and len(seq)>1
    assert all(b>a for a,b in zip(seq,seq[1:])), 'invalid decoded marker ordering'
    video=[float(t['best_effort_timestamp_time']) for i,t in enumerate(pts) if seq[i]%30==0]
    a=array.array('h');a.frombytes(subprocess.check_output(['ffmpeg','-v','error','-i',str(f),'-vn','-ac','1','-ar','48000','-f','s16le','-']))
    if sys.byteorder!='little':a.byteswap()
    start=json.loads(subprocess.check_output(['ffprobe','-v','error','-select_streams','a:0','-show_entries','stream=start_time','-of','json',str(f)]))['streams'][0].get('start_time','0')
    pulses=[];active=False
    for i in range(0,len(a)-480,480):
        loud=max(abs(x) for x in a[i:i+480])>2000
        if loud and not active:pulses.append(float(start)+i/48000)
        active=loud
    offsets=[min(pulses,key=lambda a:abs(a-v))-v for v in video if pulses and min(abs(a-v) for a in pulses)<.5]
    rows.append(dict(session=f.stem,frames=len(seq),first_sequence=seq[0],last_sequence=seq[-1],exact_missing=seq[-1]-seq[0]+1-len(seq),matched_pulses=len(offsets),av_offset_ms=[round(x*1000,2) for x in offsets]))
(p/'markers.json').write_text(json.dumps(rows,indent=2));print(json.dumps(rows))
assert rows and sum(x['matched_pulses'] for x in rows)>=3, 'insufficient matched pulse evidence'
