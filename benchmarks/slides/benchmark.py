#!/usr/bin/env python3
"""Independent technical slide benchmark, using T3Box's native encoder."""
import argparse, csv, hashlib, json, math, re, shutil, subprocess, sys, tempfile, unicodedata
from pathlib import Path

W,H,FPS=1920,1080,30
PROFILES=[('ultrafast',6000),('ultrafast',8000),('veryfast',6000),('veryfast',8000)]
HOLD=45

def run(args, **kw):
    return subprocess.run([str(x) for x in args], check=True, **kw)

def ff(*args):
    run(['ffmpeg','-hide_banner','-loglevel','error','-nostdin','-y',*args],stdout=subprocess.DEVNULL)

def ocr(path,lang):
    s=run(['tesseract',path,'stdout','-l',lang,'--psm','11'],capture_output=True,text=True).stdout
    return ''.join(c for c in unicodedata.normalize('NFC',s) if not c.isspace())

def distance(a,b):
    prev=list(range(len(b)+1))
    for i,c in enumerate(a,1):
        cur=[i]
        for j,d in enumerate(b,1):
            cur.append(min(cur[-1]+1,prev[j]+1,prev[j-1]+(c!=d)))
        prev=cur
    return prev[-1]

def metrics(path,key):
    return [float(v) for v in re.findall(r'\b'+re.escape(key)+r':([\d.]+|inf)',path.read_text())]

def percentile(xs,q):
    v=sorted(xs)[int((len(xs)-1)*q)]
    return v if math.isfinite(v) else None

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--encoder',type=Path,required=True)
    p.add_argument('--only',required=True)
    p.add_argument('--out',type=Path,required=True)
    args=p.parse_args()
    s=next(s for s in json.loads(Path(__file__).with_name('sources.json').read_text()) if s['id']==args.only)
    out=args.out.resolve()
    out.mkdir(parents=True,exist_ok=False)
    enc=args.encoder.resolve()
    rows=[]
    errors=[]
    try:
        with tempfile.TemporaryDirectory(prefix='t3box-slide-') as tmp:
            work=Path(tmp)
            pdf=work/'source.pdf'
            run(['curl','--fail','--location','--retry','2','--max-time','90','--max-filesize','100000000','--user-agent','T3Box-slide-benchmark/1.0','--output',pdf,s['url']])
            if b'%PDF-' not in pdf.open('rb').read(1024):
                raise ValueError('Not a PDF')
            info=run(['pdfinfo',pdf],capture_output=True,text=True).stdout
            total=int(re.search(r'^Pages:\s+(\d+)',info,re.M).group(1))
            pages=sorted(set(max(1,min(total,round(1+(total-1)*q))) for q in (.2,.4,.6,.8)))
            with pdf.open('rb') as f:
                digest=hashlib.file_digest(f,'sha256').hexdigest()
            (out/'source.json').write_text(json.dumps({**s,'sha256':digest,'pages':pages},ensure_ascii=False,indent=2))
            raw=work/'input.yuyv'
            ref=work/'ref.i420'
            refs=[]
            with raw.open('wb') as stream:
                for page in pages:
                    prefix=work/f'p{page}'
                    run(['pdftoppm','-f',page,'-l',page,'-scale-to',1920,'-png','-singlefile',pdf,prefix])
                    image=work/f'reference-{page}.png'
                    plane=work/'page.yuyv'
                    ff('-i',prefix.with_suffix('.png'),'-vf',f'scale={W}:{H}:force_original_aspect_ratio=decrease:force_divisible_by=2,pad={W}:{H}:(ow-iw)/2:(oh-ih)/2:color=white,setsar=1','-frames:v','1','-threads','1',image)
                    ff('-i',image,'-frames:v','1','-pix_fmt','yuyv422','-threads','1','-f','rawvideo',plane)
                    data=plane.read_bytes()
                    if len(data)!=W*H*2:
                        raise ValueError('Invalid frame length')
                    for _ in range(HOLD):
                        stream.write(data)
                    refs.append(ocr(image,s['language']))
            count=len(pages)*HOLD
            ff('-f','rawvideo','-pixel_format','yuyv422','-video_size',f'{W}x{H}','-framerate',FPS,'-i',raw,'-pix_fmt','yuv420p','-threads','1','-f','rawvideo',ref)
            for preset,rate in PROFILES:
                key=f'{preset}-{rate}'
                d=out/key
                d.mkdir()
                try:
                    video=work/f'{key}.h264'
                    audio=work/f'{key}.aac'
                    with (d/'encoder.log').open('w') as err:
                        perf=json.loads(run([enc,raw,video,count,rate,preset,audio],stdout=subprocess.PIPE,stderr=err,text=True).stdout)
                    (d/'performance.json').write_text(json.dumps(perf,indent=2))
                    ps=d/'psnr.txt'
                    ss=d/'ssim.txt'
                    filt=f'[0:v]settb=AVTB,setpts=N/30/TB,split=2[a][b];[1:v]settb=AVTB,setpts=N/30/TB,split=2[c][d];[a][c]psnr=stats_file={ps}[p];[b][d]ssim=stats_file={ss}[s]'
                    ff('-xerror','-r',FPS,'-i',video,'-f','rawvideo','-pixel_format','yuv420p','-video_size',f'{W}x{H}','-framerate',FPS,'-i',ref,'-filter_complex',filt,'-map','[p]','-map','[s]','-f','null','-')
                    psn=metrics(ps,'psnr_avg')
                    ssim=metrics(ss,'All')
                    if len(psn)!=count or len(ssim)!=count:
                        raise ValueError('Decoded metric frame count mismatch')
                    idx=[i*HOLD+j for i in range(len(pages)) for j in (0,HOLD-1)]
                    expr='+'.join(f'eq(n\\,{n})' for n in idx)
                    ff('-r',FPS,'-i',video,'-vf',f'select={expr}','-fps_mode','vfr','-threads','1',work/f'{key}-%03d.png')
                    similarities=[]
                    page_rows=[]
                    for i,page in enumerate(pages):
                        for j,phase in enumerate(('transition','settled')):
                            n=idx[2*i+j]
                            sample=work/f'{key}-{2*i+j+1:03d}.png'
                            decoded=ocr(sample,s['language'])
                            similarity=max(0,1-distance(refs[i],decoded)/len(refs[i])) if refs[i] else None
                            if similarity is not None:
                                similarities.append(similarity)
                            page_rows.append(dict(page=page,phase=phase,frame=n,psnr_db=psn[n] if math.isfinite(psn[n]) else None,ssim=ssim[n],ocr_similarity=similarity,reference_ocr_chars=len(refs[i])))
                    (d/'page_metrics.json').write_text(json.dumps(page_rows,ensure_ascii=False,indent=2))
                    rows.append(dict(source=s['id'],language=s['language'],preset=preset,target_kbps=rate,frames=count,fps=perf['fps'],p95_ms=perf['p95_ms'],video_mbps=video.stat().st_size*8*FPS/count/1e6,median_frame_psnr_db=percentile(psn,.5),p05_frame_psnr_db=percentile(psn,.05),mean_ssim=sum(ssim)/count,p05_ssim=percentile(ssim,.05),ocr_similarity_mean=sum(similarities)/len(similarities) if similarities else None,ocr_similarity_min=min(similarities) if similarities else None,ocr_samples=len(similarities)))
                    print(json.dumps(rows[-1],ensure_ascii=False),flush=True)
                except Exception as e:
                    errors.append(dict(profile=key,error=str(e)))
    except Exception as e:
        errors.append(dict(source=s['id'],error=str(e)))
    (out/'results.json').write_text(json.dumps(rows,ensure_ascii=False,indent=2))
    (out/'errors.json').write_text(json.dumps(errors,ensure_ascii=False,indent=2))
    if rows:
        with (out/'results.csv').open('w',newline='') as f:
            writer=csv.DictWriter(f,fieldnames=rows[0].keys())
            writer.writeheader()
            writer.writerows(rows)
    print(json.dumps(errors,ensure_ascii=False),file=sys.stderr)
    return 1 if errors or len(rows)!=4 else 0

if __name__=='__main__':
    sys.exit(main())
