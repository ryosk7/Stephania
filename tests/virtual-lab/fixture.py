import sys,pathlib,subprocess,json,hashlib
p=pathlib.Path(sys.argv[1]);path=p/'mixed.yuyv'
filters="drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='lt(t,2)',drawtext=fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf:text='fn main()  0123456789':fontsize=18:fontcolor=black:x=60:y=110:enable='lt(t,2)',drawtext=fontfile=/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc:text='技術発表 日本語の文字品質':fontsize=24:fontcolor=black:x=60:y=180:enable='lt(t,2)'"
subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i','testsrc2=size=1920x1080:rate=30','-vf',filters,'-frames:v','120','-threads','1','-pix_fmt','yuyv422','-f','rawvideo',str(path)],check=True)
with path.open('rb') as f: sha=hashlib.file_digest(f,'sha256').hexdigest()
(p/'fixture.json').write_text(json.dumps(dict(frames=120,fps=30,sha256=sha,kind='generated Japanese/code slide and motion')))
