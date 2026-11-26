import pathlib,sys,time,json,os
pid=int(sys.argv[1]);p=pathlib.Path(sys.argv[2])/'resources.jsonl'
with p.open('w',buffering=1) as f:
    while True:
        try:
            stat=pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
            status=dict(x.split(':',1) for x in pathlib.Path(f'/proc/{pid}/status').read_text().splitlines() if ':' in x)
            temp=pathlib.Path('/sys/class/thermal/thermal_zone0/temp')
            f.write(json.dumps(dict(monotonic_s=time.monotonic(),cpu_s=(int(stat[11])+int(stat[12]))/os.sysconf('SC_CLK_TCK'),rss_kib=int(status['VmRSS'].split()[0]),threads=int(status['Threads']),temp_millic=int(temp.read_text()) if temp.exists() else None))+'\n')
        except (FileNotFoundError,ProcessLookupError): break
        time.sleep(1)
