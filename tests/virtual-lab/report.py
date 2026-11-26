import json,pathlib,sys,os
p=pathlib.Path(sys.argv[1])
def read(n): return json.loads((p/n).read_text())
i,r,s=(read(n+'.json') for n in ('initial','running','stopped'))
checks=dict(initial=not i['streaming_requested'] and not i['native_worker_running'],
running=r['streaming_requested'] and r['native_worker_running'] and not r['ffmpeg_process_running'],
stopped=not s['streaming_requested'] and not s['native_worker_running'],
starts=all(read(n+'.json')['streaming_requested'] for n in ('start','start-repeat')),
stops=all(not read(n+'.json')['streaming_requested'] for n in ('stop','stop-repeat')))
rows=[]
previous=None
for f in sorted(p.glob('session-*.flv'),key=lambda x:int(x.stem.split('-')[1])):
    n=f.stem;c=read(n+'.json');ss=read(n+'.probe.json')['streams']
    v=next(x for x in ss if x['codec_name']=='h264');a=next(x for x in ss if x['codec_name']=='aac')
    arr=read(n+'.arrivals.json')
    growth=[(t-arr[0][1])*1000-(pts-arr[0][0]) for pts,t in arr]
    fps=(c['video']-1)/max(.001,c['last_video_arrival_s']-c['first_video_arrival_s'])
    expected=round(c['video_media_span_ms']*30/1000)+1
    recovery=None if previous is None else c['first_video_arrival_s']-previous
    previous=c['last_video_arrival_s']
    checks[n+'-decode']=int(v['nb_read_frames'])==c['video'] and int(a['nb_read_frames'])==c['audio'] and (v['width'],v['height'])==(1920,1080) and int(a['sample_rate'])==48000 and a['channels']==2
    tail=[x for x in arr if x[1]>=arr[-1][1]-3]
    rows.append(dict(session=n,video=c['video'],audio=c['audio'],fps=fps,missing=max(0,expected-c['video'])/expected,skew_ms=c['max_packet_timestamp_skew_ms'],relative_delay_growth_p95_ms=sorted(growth)[int(.95*(len(growth)-1))],recovery_s=recovery,tail_frames=len(tail),max_arrival_gap_s=max((b[1]-a[1] for a,b in zip(arr,arr[1:])),default=0)))
checks['sessions']=bool(rows)
if rows:
    last=rows[-1];fault=os.environ.get('LAB_FAULT','none')
    if fault=='none':
        checks.update(single_session=len(rows)==1,fps=last['fps']>=29.7,missing=last['missing']<=.03,skew=last['skew_ms']<=250,video=last['video']>=210)
    else:
        checks.update(reconnect=len(rows)>1 or fault!='disconnect',recovery=all(x['recovery_s'] is None or 0<=x['recovery_s']<=10 for x in rows),gap=max(x['max_arrival_gap_s'] for x in rows)<=10,tail=last['tail_frames']>=85,delay_growth=last['relative_delay_growth_p95_ms']<=1000)
resources=[json.loads(x) for x in (p/'resources.jsonl').read_text().splitlines()]
checks['resource_samples']=len(resources)>=3
peak=max((x['rss_kib'] for x in resources),default=0)
checks['rss_bound']=0<peak<=256*1024
if int(os.environ.get('LAB_SECONDS','15'))>=3600:
    duration=resources[-1]['monotonic_s']-resources[0]['monotonic_s']
    checks['soak_duration']=duration>=int(os.environ['LAB_SECONDS'])-5
    first=[x['rss_kib'] for x in resources[60:360]]
    last_rss=[x['rss_kib'] for x in resources[-300:]]
    import statistics
    checks['rss_growth']=bool(first and last_rss) and statistics.median(last_rss)-statistics.median(first)<=16*1024
    checks['thread_growth']=max(x['threads'] for x in resources[-300:])<=max(x['threads'] for x in resources[60:360])+2
    checks['soak_frame_count']=sum(x['video'] for x in rows)>=int(os.environ['LAB_SECONDS'])*29.7
cpu=[(b['cpu_s']-a['cpu_s'])/(b['monotonic_s']-a['monotonic_s'])*100 for a,b in zip(resources,resources[1:])]
result=dict(checks=checks,sessions=rows,latency_kind='relative growth only; markers.json has virtual source age',peak_rss_mib=peak/1024,cpu_one_core_percent_p95=sorted(cpu)[int(.95*(len(cpu)-1))] if cpu else None,resource_samples=len(resources))
(p/'report.json').write_text(json.dumps(result,indent=2));print(json.dumps(result));sys.exit(not all(checks.values()))
