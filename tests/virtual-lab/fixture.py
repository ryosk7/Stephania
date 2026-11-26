import sys,pathlib,subprocess,json,hashlib,os,shutil
p=pathlib.Path(sys.argv[1]);path=p/'mixed.yuyv'
filters="drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='lt(t,2)',drawtext=fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf:text='fn main()  0123456789':fontsize=18:fontcolor=black:x=60:y=110:enable='lt(t,2)',drawtext=fontfile=/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc:text='技術発表':fontsize=24:fontcolor=black:x=60:y=180:enable='lt(t,2)',drawtext=fontfile=/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc:text='日本語の文字品質':fontsize=24:fontcolor=black:x=260:y=180:enable='lt(t,2)'"
subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i','testsrc2=size=1920x1080:rate=30','-vf',filters,'-frames:v','120','-threads','1','-pix_fmt','yuyv422','-f','rawvideo',str(path)],check=True)
sources=[]
if os.environ.get('LAB_MEDIA')=='real':
    sources=[
        ('slide','https://www.atdot.net/~ko1/activities/2025_astro_kernelvm2025.pdf','pdf',60),
        ('photo','https://r0k.us/graphics/kodak/kodak/kodim05.png','png',60),
        ('people','https://media.xiph.org/video/derf/webm/FourPeople_1280x720_60.webm','webm',120),
    ]
    records=[]
    scale='scale=1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30'
    for name,url,ext,frames in sources:
        src=p/(name+'.'+ext)
        subprocess.run(['curl','-fL','--retry','3','--max-time','180','-o',str(src),url],check=True)
        with src.open('rb') as inp: digest=hashlib.file_digest(inp,'sha256').hexdigest()
        records.append(dict(name=name,url=url,sha256=digest,frames=frames))
        if ext=='pdf':
            subprocess.run(['pdftoppm','-f','5','-l','5','-scale-to','1920','-singlefile','-png',str(src),str(p/'page')],check=True)
            src=p/'page.png'
        clip=p/(name+'.raw')
        subprocess.run(['ffmpeg','-v','error','-y',*(['-loop','1'] if ext in ('pdf','png') else []),'-i',str(src),'-vf',scale,'-frames:v',str(frames),'-threads','1','-pix_fmt','yuyv422','-f','rawvideo',str(clip)],check=True)
        assert clip.stat().st_size==frames*1920*1080*2
        with path.open('ab') as dest,clip.open('rb') as inp: shutil.copyfileobj(inp,dest)
        clip.unlink()
    sources=records
with path.open('rb') as f: sha=hashlib.file_digest(f,'sha256').hexdigest()
(p/'fixture.json').write_text(json.dumps(dict(frames=path.stat().st_size//(1920*1080*2),fps=30,sha256=sha,kind='generated plus external corpus' if sources else 'generated Japanese/code slide and motion',sources=sources)))
