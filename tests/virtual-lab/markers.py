"""Independent decoded sequence markers and PCM pulse measurement.
Audio-video offset is measured in decoded media PTS, not optical sync.
Absolute capture latency is not inferred without a common source epoch.
"""
import pathlib,sys,subprocess,json,array,statistics,re,math
p=pathlib.Path(sys.argv[1]);rows=[]
epochs=re.findall(r'lab source epoch_ns=(\d+) uncertainty_us=(\d+)',(p/'controller.log').read_text())
assert len(epochs)==1, 'source epoch missing or ambiguous'
epoch,uncertainty=map(int,epochs[0])
assert uncertainty<=2000
def p95(values): return sorted(values)[math.ceil(.95*len(values))-1]
reference=pathlib.Path(sys.argv[2])
frame_bytes=1920*1080*2
ref_count=reference.stat().st_size//frame_bytes
assert ref_count>0
quality=[]
text_samples=[]

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
    arr=json.loads((p/(f.stem+'.arrivals.json')).read_text())
    assert len(arr)==len(seq)
    delays=[t*1000-epoch/1e6-n*1000/30 for n,(_,t) in zip(seq,arr)]
    assert min(delays)>=-3, 'negative source age'
    # Independent full-resolution grayscale luma comparison, excluding marker row.
    proc=subprocess.Popen(['ffmpeg','-v','error','-i',str(f),'-an','-vf','extractplanes=y','-fps_mode','passthrough','-f','rawvideo','-'],stdout=subprocess.PIPE)
    import numpy as np
    with reference.open('rb') as ref:
        for j,n in enumerate(seq):
            data=proc.stdout.read(1920*1080)
            assert len(data)==1920*1080
            ref.seek((n%ref_count)*frame_bytes)
            src=np.frombuffer(ref.read(frame_bytes),dtype=np.uint8)[::2].reshape(1080,1920)[16:,:]
            dst=np.frombuffer(data,dtype=np.uint8).reshape(1080,1920)[16:,:]
            err=np.mean((src.astype(np.float32)-dst.astype(np.float32))**2)
            quality.append(100.0 if err==0 else float(10*np.log10(255**2/err)))
            # fixed known text ROIs, compare to ground truth not OCR of input.
            if n%ref_count==30 and len(text_samples)<12:
                for lang,roi,expected in [('eng',(60,100,650,55),'fnmain0123456789'),('jpn',(60,170,650,55),'技術発表日本語の文字品質')]:
                    x,y,w,h=roi
                    image=dst[y-16:y-16+h,x:x+w].tobytes()
                    pgm=f'P5\n{w} {h}\n255\n'.encode()+image
                    actual=subprocess.check_output(['tesseract','stdin','stdout','-l',lang,'--psm','7'],input=pgm,stderr=subprocess.DEVNULL).decode()
                    import difflib
                    normal=lambda s: ''.join(c.lower() for c in s if c.isalnum())
                    text_samples.append(dict(language=lang,sequence=n,expected=expected,actual=actual.strip(),similarity=difflib.SequenceMatcher(None,normal(expected),normal(actual),autojunk=False).ratio()))
    assert proc.wait()==0
    rows.append(dict(absolute_source_to_receive_p95_ms=p95(delays),absolute_source_to_receive_max_ms=max(delays),session=f.stem,frames=len(seq),first_sequence=seq[0],last_sequence=seq[-1],exact_missing=seq[-1]-seq[0]+1-len(seq),matched_pulses=len(offsets),av_offset_ms=[round(x*1000,2) for x in offsets]))
(p/'markers.json').write_text(json.dumps(rows,indent=2));print(json.dumps(rows))
assert rows and sum(x['matched_pulses'] for x in rows)>=3, 'insufficient matched pulse evidence'

result=dict(luma_psnr_mean_db=statistics.mean(quality),luma_psnr_p05_db=sorted(quality)[int(.05*(len(quality)-1))],ocr=text_samples,source_clock_uncertainty_us=uncertainty)
(p/'quality.json').write_text(json.dumps(result,ensure_ascii=False,indent=2))
print(json.dumps(result,ensure_ascii=False))
assert quality and text_samples, 'missing quality evidence'
# Preserve existing integrity floors; proposed tighter latency/sync targets
# are recorded, not silently treated as validated release criteria.
assert result['luma_psnr_p05_db']>=30
