import os, sys, subprocess as sp, pathlib as P, json, re, time, tempfile, hashlib, statistics, threading, math
def run(*a,**kw): return sp.run([str(x) for x in a],check=True,**kw)
def ff(*a): run('ffmpeg','-hide_banner','-loglevel','error','-nostdin','-y',*a)
def download(url,path):
    run('curl','-fL','--retry','2','--max-time','180','--max-filesize','1000000000','-o',path,url)
    return {'url':url,'sha256':hashlib.file_digest(path.open('rb'),'sha256').hexdigest()}
mode,rate,network=sys.argv[1:4]
profile=os.environ.get('T3BOX_BENCH_PROFILE','fixed')
binary=os.environ.get('T3BOX_BENCH_BINARY','target/release/examples/realtime_media')
out=P.Path(sys.argv[4] if len(sys.argv)>4 else 'evidence').resolve();out.mkdir(parents=True,exist_ok=True)
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
            clip(n,'https://media.xiph.org/video/derf/webm/'+n+'_1280x720_60.webm','.webm',120)
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
    run('sudo','sed','-i','/t3box-ci.test/d','/etc/hosts')
    run('sudo','tee','-a','/etc/hosts',input='10.203.0.2 t3box-ci.test\n',text=True,stdout=sp.DEVNULL)
    env={**os.environ,'T3BOX_CI_SESSIONS':'1','T3BOX_CI_FIRST_LIMIT':str(count+1),'T3BOX_CI_CA_DER':str(w/'ca.der')}
    # Sender on host, receiver in its own namespace. Shape sender-bound
    # ACKs on receiver egress, data at receiver ingress via IFB.
    ns='t3bench'
    stop=threading.Event()
    def sample():
        with (out/'network.jsonl').open('w') as f:
            while not stop.is_set():
                ss=sp.run(['ss','-tinm','dst','10.203.0.2:19443'],capture_output=True,text=True)
                tc=sp.run(['sudo','ip','netns','exec',ns,'tc','-s','qdisc','show'],capture_output=True,text=True)
                f.write(json.dumps({'monotonic_ns':time.monotonic_ns(),'ss':ss.stdout,'qdisc':tc.stdout})+'\n');f.flush()
                stop.wait(.1)
    run('sudo','ip','netns','add',ns)
    try:
        run('sudo','ip','link','add','t3tx','type','veth','peer','name','t3rx')
        run('sudo','ip','link','set','t3rx','netns',ns)
        run('sudo','ip','addr','add','10.203.0.1/24','dev','t3tx')
        run('sudo','ip','link','set','t3tx','up')
        run('sudo','ip','netns','exec',ns,'ip','addr','add','10.203.0.2/24','dev','t3rx')
        run('sudo','ip','netns','exec',ns,'ip','link','set','t3rx','up')
        run('sudo','ip','netns','exec',ns,'ip','link','set','lo','up')
        if network=='venue':
            run('sudo','modprobe','ifb')
            run('sudo','ip','netns','exec',ns,'ip','link','add','t3ifb','type','ifb')
            run('sudo','ip','netns','exec',ns,'ip','link','set','t3ifb','up')
            run('sudo','ip','netns','exec',ns,'tc','qdisc','add','dev','t3rx','ingress')
            run('sudo','ip','netns','exec',ns,'tc','filter','add','dev','t3rx','parent','ffff:','protocol','all','u32','match','u32','0','0','action','mirred','egress','redirect','dev','t3ifb')
            run('sudo','ip','netns','exec',ns,'tc','qdisc','add','dev','t3ifb','root','netem','delay','40ms','10ms','loss','0.2%','rate','12mbit')
            run('sudo','ip','netns','exec',ns,'tc','qdisc','add','dev','t3rx','root','netem','delay','40ms','10ms','loss','0.2%','rate','12mbit')
        with (out/'server.log').open('w') as log:
            server=sp.Popen(['sudo','ip','netns','exec',ns,'sudo','-u',os.environ['USER'],'env','T3BOX_CI_SESSIONS=1','T3BOX_CI_FIRST_LIMIT='+str(count+1),'python3','benchmarks/realtime/receiver.py',str(w)],stdout=log,stderr=log)
            monitor=threading.Thread(target=sample,daemon=True)
            try:
                for _ in range(100):
                    if (w/'ready').exists(): break
                    time.sleep(.1)
                assert (w/'ready').exists()
                monitor.start()
                with (out/'timing.jsonl').open('w') as f,(out/'encoder.log').open('w') as e:
                    run('/usr/bin/time','-v','-o',out/'resources.txt',binary,raw,count,rate,'rtmps://t3box-ci.test:19443/live/test',env=env,stdout=f,stderr=e,timeout=180)
                assert server.wait(timeout=30)==0
            finally:
                stop.set()
                if monitor.is_alive(): monitor.join(timeout=5)
                if server.poll() is None:
                    for pid in sp.run(['sudo','ip','netns','pids',ns],capture_output=True,text=True).stdout.split():
                        sp.run(['sudo','kill',pid],check=False)
                    server.wait(timeout=10)
    finally:
        sp.run(['sudo','ip','netns','del',ns],check=False)
        sp.run(['sudo','ip','link','del','t3tx'],check=False)
    recv=json.loads((w/'session-1.json').read_text()); (out/'receiver.json').write_text(json.dumps(recv,indent=2))
    timings=[json.loads(l) for l in (out/'timing.jsonl').read_text().splitlines()]
    if timings and timings[-1].get('aborted'):
        arrivals=(w/'arrivals-1.jsonl').read_text()
        (out/'arrivals.jsonl').write_text(arrivals)
        result=dict(mode=mode,target_kbps=int(rate),network=network,profile=profile,frames=count,received_frames=recv['video'],missing_frames=count-recv['video'],aborted=True,watchdog_expired=timings[-1]['watchdog_expired'],elapsed_s=timings[-1]['elapsed_s'])
        (out/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result));sys.exit(0)
    assert recv['video']==count
    arrivals=[json.loads(l) for l in (w/'arrivals-1.jsonl').read_text().splitlines()]
    (out/'arrivals.jsonl').write_text((w/'arrivals-1.jsonl').read_text())
    ps=out/'psnr.txt'; ss=out/'ssim.txt'
    filt=f'[0:v]settb=AVTB,setpts=N/30/TB,split=2[a][b];[1:v]settb=AVTB,setpts=N/30/TB,split=2[c][d];[a][c]psnr=stats_file={ps}[p];[b][d]ssim=stats_file={ss}[s]'
    ff('-xerror','-i',w/'session-1.flv','-f','rawvideo','-pixel_format','yuv420p','-video_size','1920x1080','-framerate','30','-i',ref,'-filter_complex',filt,'-map','[p]','-map','[s]','-f','null','-')
    p=[float(v) for v in re.findall(r'psnr_avg:([\d.]+|inf)',ps.read_text())];s=[float(v) for v in re.findall(r'All:([\d.]+)',ss.read_text())]
    assert len(p)==len(s)==count
    timings=[json.loads(l) for l in (out/'timing.jsonl').read_text().splitlines()]
    def q(a,f):return sorted(a)[int((len(a)-1)*f)]
    assert len(arrivals)==count==len(timings)-1
    assert all(a['pts_ms']==100+i*1000//30 for i,a in enumerate(arrivals))
    delays=[(a['wall_ns']-t['scheduled_wall_ns'])/1e6 for a,t in zip(arrivals,timings)]
    gaps=[(b['monotonic_ns']-a['monotonic_ns'])/1e6 for a,b in zip(arrivals,arrivals[1:])]
    def peak(window,field):
        left=0;total=0;maximum=0
        for right,a in enumerate(arrivals):
            total+=a['bytes']
            while a[field]-arrivals[left][field]>=window:
                total-=arrivals[left]['bytes'];left+=1
            maximum=max(maximum,total)
        return maximum*8/window*1000 if field=='pts_ms' else maximum*8/window*1e9
    result=dict(mode=mode,target_kbps=int(rate),network=network,profile=profile,aborted=False,missing_frames=0,frames=count,receiver_fps=(count-1)/(recv['last_video_arrival_s']-recv['first_video_arrival_s']),video_mbps=recv['video_payload_bytes']*8*30/count/1e6,psnr_median=q(p,.5),psnr_p05=q(p,.05),ssim_mean=statistics.mean(s),ssim_p05=q(s,.05),completion_p95_ms=q([t['completion_ms'] for t in timings[:-1]],.95),encode_send_p95_ms=q([t['encode_send_ms'] for t in timings[:-1]],.95),deadline_misses=timings[-1]['deadline_misses'],max_media_gap_ms=recv['video_max_gap_ms'],max_av_skew_ms=recv['max_packet_timestamp_skew_ms'],arrival_delay_p95_ms=q(delays,.95),arrival_delay_max_ms=max(delays),arrival_gap_p95_ms=q(gaps,.95),arrival_gap_max_ms=max(gaps),media_peak_100ms_mbps=peak(100,'pts_ms')/1e6,media_peak_1s_mbps=peak(1000,'pts_ms')/1e6,arrival_peak_100ms_mbps=peak(100_000_000,'monotonic_ns')/1e6,arrival_peak_1s_mbps=peak(1_000_000_000,'monotonic_ns')/1e6,max_frame_bytes=max(a['bytes'] for a in arrivals))
    result={k:(None if isinstance(v,float) and not math.isfinite(v) else v) for k,v in result.items()}
    (out/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
