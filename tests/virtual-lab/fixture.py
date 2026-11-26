import sys,pathlib,subprocess,json,hashlib,os,shutil
from PIL import Image,ImageDraw,ImageFont
p=pathlib.Path(sys.argv[1]);path=p/'mixed.yuyv'
# Render known Unicode glyphs explicitly rather than FFmpeg drawtext's TTC layout.
slide=Image.new('RGB',(1920,1080),'white')
draw=ImageDraw.Draw(slide)
mono=ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf',18)
jp=ImageFont.truetype('/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc',24,index=0)
draw.text((60,110),'fn main()  0123456789',font=mono,fill='black',anchor='lt')
draw.text((60,180),'技術発表',font=jp,fill='black',anchor='lt')
draw.text((260,180),'日本語の文字品質',font=jp,fill='black',anchor='lt')
slide.save(p/'generated-slide.png')
for box,expected in [((40,160,220,230),'技術発表'),((240,160,500,230),'日本語の文字品質')]:
    crop=slide.crop(box).convert('L').resize(((box[2]-box[0])*4,(box[3]-box[1])*4),Image.Resampling.NEAREST)
    import io
    buf=io.BytesIO();crop.save(buf,format='PPM')
    actual=subprocess.check_output(['tesseract','stdin','stdout','-l','jpn','--psm','7'],input=buf.getvalue(),stderr=subprocess.DEVNULL).decode()
    assert ''.join(c for c in actual if c.isalnum())==expected, f'generated reference OCR failed: {actual!r} vs {expected!r}'
# Generate the exact existing 2s slide + 2s moving pattern, 120 YUYV frames.
subprocess.run(['ffmpeg','-v','error','-y','-loop','1','-framerate','30','-i',str(p/'generated-slide.png'),'-frames:v','60','-threads','1','-pix_fmt','yuyv422','-f','rawvideo',str(path)],check=True)
motion=p/'generated-motion.raw'
subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i','testsrc2=size=1920x1080:rate=30','-frames:v','60','-threads','1','-pix_fmt','yuyv422','-f','rawvideo',str(motion)],check=True)
with path.open('ab') as dest,motion.open('rb') as inp: shutil.copyfileobj(inp,dest)
motion.unlink()

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
