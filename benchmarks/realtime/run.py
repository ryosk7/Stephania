import os, sys, subprocess as sp, pathlib as P, json, re, time, tempfile, hashlib, statistics
def run(*a,**kw): return sp.run([str(x) for x in a],check=True,**kw)
def ff(*a): run('ffmpeg','-hide_banner','-loglevel','error','-nostdin','-y',*a)
def download(url,path):
    run('curl','-fL','--retry','2','--max-time','180','--max-filesize','1000000000','-o',path,url)
    return {'url':url,'sha256':hashlib.file_digest(path.open('rb'),'sha256').hexdigest()}
mode,rate,network=sys.argv[1:4]
out=P.Path('evidence').resolve();out.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory() as tmp:
    w=P.Path(tmp); sources=[]; clips=[]
    scale='scale=1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30'
    def clip(name,url,ext,frames,still=False):
        path=w/(name+ext);sources.append(download(url,path));raw=w/(name+'.raw')
        ff(*(['-loop','1'] if still else []),'-i',path,'-vf',scale,'-frames:v',frames,'-pix_fmt','yuyv422','-threads','1','-f','rawvideo',raw)
        assert raw.stat().st_size==frames*1920*1080*2
        clips.append(raw)
    if mode in ('photo','mixed'):
        for n in (5,13,23): clip('kodim%02d'%n,'https://r0k.us/graphics/kodak/kodak/kodim%02d.png'%n,'.png',60,True)
    if mode in ('people','mixed'):
        for n in ('FourPeople','Johnny'):
            clip(n,'https://media.xiph.org/video/derf/'+n+'_1280x720_60.y4m','.y4m',120)
    if mode in ('animation','mixed'):
        clip('bbb','https://raw.githubusercontent.com/bower-media-samples/big-buck-bunny-1080p-60fps-30s/master/video.mp4','.mp4',180)
    if mode=='mixed':
        s=json.loads(P.Path('benchmarks/slides/sources.json').read_text())[0]
        pdf=w/'slide.pdf';sources.append(download(s['url'],pdf))
        run('pdftoppm','-f','5','-l','5','-scale-to','1920','-singlefile','-png',pdf,w/'slide')
        raw=w/'slide.raw';ff('-loop','1','-i',w/'slide.png','-vf',scale,'-frames:v',90,'-pix_fmt','yuyv422','-threads','1','-f','rawvideo',raw);clips.insert(0,raw)
    (out/'sources.json').write_text(json.dumps(sources,indent=2))
    raw=w/'input.raw'
    with raw.open('wb') as f:
        import shutil
        for c in clips:
            with c.open('rb') as src: shutil.copyfileobj(src,f)
            c.unlink()
    count=raw.stat().st_size//(1920*1080*2)
    ref=w/'ref.raw'
    ff('-f','rawvideo','-pixel_format','yuyv422','-video_size','1920x1080','-framerate','30','-i',raw,'-pix_fmt','yuv420p','-threads','1','-f','rawvideo',ref)
    run('openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/CN=T3Box-Bench-CA','-keyout',w/'ca.key','-out',w/'ca.pem',stdout=sp.DEVNULL,stderr=sp.DEVNULL)
    run('openssl','req','-newkey','rsa:2048','-nodes','-subj','/CN=t3box-ci.test','-keyout',w/'server.key','-out',w/'server.csr',stdout=sp.DEVNULL,stderr=sp.DEVNULL)
    (w/'ext').write_text('subjectAltName=DNS:t3box-ci.test\nextendedKeyUsage=serverAuth\n')
    run('openssl','x509','-req','-in',w/'server.csr','-CA',w/'ca.pem','-CAkey',w/'ca.key','-CAcreateserial','-days','1','-extfile',w/'ext','-out',w/'server.pem')
    run('openssl','x509','-in',w/'ca.pem','-outform','der','-out',w/'ca.der')
    run('sudo','tee','-a','/etc/hosts',input='127.0.0.1 t3box-ci.test\n',text=True,stdout=sp.DEVNULL)
    env={**os.environ,'T3BOX_CI_SESSIONS':'1','T3BOX_CI_FIRST_LIMIT':str(count+1),'T3BOX_CI_CA_DER':str(w/'ca.der')}
    with (out/'server.log').open('w') as log:
        server=sp.Popen(['python3','ci/rtmps-test-server.py',str(w)],env=env,stdout=log,stderr=log)
        try:
            for _ in range(100):
                if (w/'ready').exists(): break
                time.sleep(.1)
            assert (w/'ready').exists()
            if network=='venue':
                run('sudo','tc','qdisc','add','dev','lo','root','netem','delay','40ms','10ms','loss','0.2%','rate','12mbit')
            with (out/'timing.jsonl').open('w') as f,(out/'encoder.log').open('w') as e:
                run('/usr/bin/time','-v','-o',out/'resources.txt','target/release/examples/realtime_media',raw,count,rate,'rtmps://t3box-ci.test:19443/live/test',env=env,stdout=f,stderr=e,timeout=180)
            assert server.wait(timeout=30)==0
        finally:
            if network=='venue': sp.run(['sudo','tc','qdisc','del','dev','lo','root'],check=False)
            if server.poll() is None: server.kill();server.wait()
    recv=json.loads((w/'session-1.json').read_text()); (out/'receiver.json').write_text(json.dumps(recv,indent=2))
    assert recv['video']==count
    ps=out/'psnr.txt'; ss=out/'ssim.txt'
    filt=f'[0:v]settb=AVTB,setpts=N/30/TB,split=2[a][b];[1:v]settb=AVTB,setpts=N/30/TB,split=2[c][d];[a][c]psnr=stats_file={ps}[p];[b][d]ssim=stats_file={ss}[s]'
    ff('-xerror','-i',w/'session-1.flv','-f','rawvideo','-pixel_format','yuv420p','-video_size','1920x1080','-framerate','30','-i',ref,'-filter_complex',filt,'-map','[p]','-map','[s]','-f','null','-')
    p=[float(v) for v in re.findall(r'psnr_avg:([\d.]+|inf)',ps.read_text())];s=[float(v) for v in re.findall(r'All:([\d.]+)',ss.read_text())]
    assert len(p)==len(s)==count
    timings=[json.loads(l) for l in (out/'timing.jsonl').read_text().splitlines()]
    def q(a,f):return sorted(a)[int((len(a)-1)*f)]
    import math
    result=dict(mode=mode,target_kbps=int(rate),network=network,frames=count,receiver_fps=(count-1)/(recv['last_video_arrival_s']-recv['first_video_arrival_s']),video_mbps=recv['video_payload_bytes']*8*30/count/1e6,psnr_median=q(p,.5),psnr_p05=q(p,.05),ssim_mean=statistics.mean(s),ssim_p05=q(s,.05),completion_p95_ms=q([t['completion_ms'] for t in timings[:-1]],.95),encode_send_p95_ms=q([t['encode_send_ms'] for t in timings[:-1]],.95),deadline_misses=timings[-1]['deadline_misses'],max_media_gap_ms=recv['video_max_gap_ms'],max_av_skew_ms=recv['max_packet_timestamp_skew_ms'])
    result={k:(None if isinstance(v,float) and not math.isfinite(v) else v) for k,v in result.items()}
    (out/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
