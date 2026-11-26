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
import numpy as np
from collections import defaultdict
scene_quality=defaultdict(list)
manifest=json.loads((p/'fixture.json').read_text())
scenes=[(0,60,'generated_text'),(60,120,'generated_motion')]
end=120
for source in manifest.get('sources',[]):
    scenes.append((end,end+source['frames'],source['name']));end+=source['frames']
assert end==ref_count

for f in sorted(p.glob('session-*.flv')):
    seq=[]
    arr=json.loads((p/(f.stem+'.arrivals.json')).read_text())
    probe=json.loads((p/(f.stem+'.probe.json')).read_text())
    v=next(x for x in probe['streams'] if x['codec_name']=='h264')
    assert v.get('has_b_frames',0)==0, 'marker alignment requires no B frames'
    assert int(v['nb_read_frames'])==len(arr)
    a=array.array('h');a.frombytes(subprocess.check_output(['ffmpeg','-v','error','-xerror','-i',str(f),'-vn','-ac','1','-ar','48000','-f','s16le','-']))
    if sys.byteorder!='little':a.byteswap()
    start=json.loads(subprocess.check_output(['ffprobe','-v','error','-select_streams','a:0','-show_entries','stream=start_time','-of','json',str(f)]))['streams'][0].get('start_time','0')
    pulses=[];active=False
    for i in range(0,len(a)-480,480):
        loud=max(abs(x) for x in a[i:i+480])>2000
        if loud and not active:pulses.append(float(start)+i/48000)
        active=loud
    # Independent full-resolution grayscale luma comparison, excluding marker row.
    proc=subprocess.Popen(['ffmpeg','-v','error','-xerror','-i',str(f),'-an','-vf','extractplanes=y','-fps_mode','passthrough','-f','rawvideo','-'],stdout=subprocess.PIPE)
    import numpy as np
    with reference.open('rb') as ref:
        for j in range(len(arr)):
            data=proc.stdout.read(1920*1080)
            assert len(data)==1920*1080
            full=np.frombuffer(data,dtype=np.uint8).reshape(1080,1920)
            bits=full[:16,:512].reshape(16,32,16).mean(axis=(0,2))>128
            n=sum(int(bit)<<i for i,bit in enumerate(bits))
            seq.append(n)
            ref.seek((n%ref_count)*frame_bytes)
            src=np.frombuffer(ref.read(frame_bytes),dtype=np.uint8)[::2].reshape(1080,1920)[16:,:]
            dst=full[16:,:]
            err=np.mean((src.astype(np.float32)-dst.astype(np.float32))**2)
            score=100.0 if err==0 else float(10*np.log10(255**2/err))
            quality.append(score)
            scene=next(name for lo,hi,name in scenes if lo<=n%ref_count<hi)
            scene_quality[scene].append(score)
            # fixed known text ROIs, compare to ground truth not OCR of input.
            if n%ref_count==30 and len(text_samples)<12:
                for lang,roi,expected in [('eng',(60,100,650,55),'fnmain0123456789'),('jpn',(60,170,650,55),'技術発表日本語の文字品質')]:
                    x,y,w,h=roi
                    # Identical reference/decoded OCR pipeline, including scaling.
                    def ocr(img):
                        crop=img[y-16:y-16+h,x:x+w]
                        enlarged=np.repeat(np.repeat(crop,3,axis=0),3,axis=1)
                        pgm=f'P5\n{w*3} {h*3}\n255\n'.encode()+enlarged.tobytes()
                        return subprocess.check_output(['tesseract','stdin','stdout','-l',lang,'--psm','7'],input=pgm,stderr=subprocess.DEVNULL).decode().strip()
                    import difflib
                    normal=lambda s: ''.join(c.lower() for c in s if c.isalnum())
                    sim=lambda a,b: difflib.SequenceMatcher(None,normal(a),normal(b),autojunk=False).ratio()
                    actual=ocr(dst); baseline=ocr(src)
                    text_samples.append(dict(language=lang,sequence=n,expected=expected,actual=actual,reference_ocr=baseline,reference_similarity=sim(expected,baseline),similarity=sim(expected,actual),retention=sim(baseline,actual)))

    assert not proc.stdout.read(1), 'unexpected additional decoded video'
    assert proc.wait()==0
    assert len(seq)>1 and all(b>a for a,b in zip(seq,seq[1:])), 'invalid decoded marker ordering'
    video=[pts/1000 for (pts,_),n in zip(arr,seq) if n%30==0]
    offsets=[min(pulses,key=lambda a:abs(a-v))-v for v in video if pulses and min(abs(a-v) for a in pulses)<.5]
    delays=[t*1000-epoch/1e6-n*1000/30 for n,(_,t) in zip(seq,arr)]
    assert min(delays)>=-3, 'negative source age'
    rows.append(dict(absolute_source_to_receive_p95_ms=p95(delays),absolute_source_to_receive_max_ms=max(delays),session=f.stem,frames=len(seq),first_sequence=seq[0],last_sequence=seq[-1],exact_missing=seq[-1]-seq[0]+1-len(seq),matched_pulses=len(offsets),av_offset_ms=[round(x*1000,2) for x in offsets]))
(p/'markers.json').write_text(json.dumps(rows,indent=2));print(json.dumps(rows))
assert rows and sum(x['matched_pulses'] for x in rows)>=3, 'insufficient matched pulse evidence'

result=dict(luma_psnr_mean_db=statistics.mean(quality),luma_psnr_p05_db=sorted(quality)[int(.05*(len(quality)-1))],ocr=text_samples,source_clock_uncertainty_us=uncertainty,scenes={k:dict(frames=len(v),p05_db=sorted(v)[int(.05*(len(v)-1))],mean_db=statistics.mean(v)) for k,v in scene_quality.items()})
(p/'quality.json').write_text(json.dumps(result,ensure_ascii=False,indent=2))
print(json.dumps(result,ensure_ascii=False))
assert quality and text_samples, 'missing quality evidence'
# Preserve existing integrity floors; proposed tighter latency/sync targets
# are recorded, not silently treated as validated release criteria.
assert result['luma_psnr_p05_db']>=30

# Fail an invalid reference OCR separately from compression-induced degradation.
assert all(any(x['language']==lang for x in text_samples) for lang in ('eng','jpn'))
assert all(x['reference_similarity']>=.90 for x in text_samples), 'OCR reference itself unreadable: measurement invalid'
assert all(x['similarity']>=.90 and x['retention']>=.90 for x in text_samples), 'decoded OCR regression'
assert set(scene_quality)=={x[2] for x in scenes}, 'missing scene coverage'
assert all(v['p05_db']>=30 for v in result['scenes'].values()), 'scene quality regression'
assert all(abs(x)<=50 for row in rows for x in row['av_offset_ms']), 'decoded A/V offset'
assert rows[-1]['absolute_source_to_receive_p95_ms']<=250, 'final session freshness'
