//! Benchmark-only paced file source through the production RTMPS pipeline.
use std::{env,fs::File,io::{self,Read},thread,time::{Instant,Duration,SystemTime,UNIX_EPOCH}};
use t3box::{native_video::NativeVideoPipeline,native_aac::AacBackend,pcm_framer::PcmBlock,media_core::{VideoFrame,PixelFormat},rtmps_tls};

fn main()->io::Result<()> {
    let a:Vec<String>=env::args().collect();
    let count:u64=a[2].parse().map_err(io::Error::other)?;
    let rate:u32=a[3].parse().map_err(io::Error::other)?;
    let mut input=File::open(&a[1])?;
    let mut aac=AacBackend::new(128)?;
    let profile=env::var("T3BOX_BENCH_PROFILE").unwrap_or_else(|_|"fixed".into());
    let adaptive=profile!="fixed";
    let mut session=rtmps_tls::begin_publish(&a[4])?;
    let monitor=session.tcp_monitor()?;
    let guard=if profile=="full" {
        let g=t3box::transport_deadline::DeadlineWatchdog::start(session.tcp_monitor()?)?;
        session.set_media_deadline(g.handle(),Duration::from_millis(500));
        Some(g)
    } else {None};
    let mut p=NativeVideoPipeline::new(session,1920,1080,30,rate)?;
    if adaptive {p.reconfigure_rate(rate,500)?;}
    if profile=="full" {p.enable_content_analysis();}
    let mut controller=t3box::rate_control::RateController::new(rate);
    let mut network_rate=rate;
    let mut applied=rate;
    let mut next_sample=Instant::now();
    p.configure_audio(&aac.configuration()?)?;
    let mut frame=VideoFrame{timestamp_us:0,sequence:0,width:1920,height:1080,row_stride_bytes:3840,format:PixelFormat::Yuyv422,bytes:vec![0;1920*1080*2]};
    let mut audio=0u64;
    let wall_start=SystemTime::now().duration_since(UNIX_EPOCH).map_err(io::Error::other)?.as_nanos();
    let start=Instant::now();
    let mut missed=0;
    let mut failed=false;
    for i in 0..count {
        input.read_exact(&mut frame.bytes)?;
        let due=start+Duration::from_micros(i*1_000_000/30);
        thread::sleep(due.saturating_duration_since(Instant::now()));
        let tick=Instant::now();
        if adaptive && tick>=next_sample {
            next_sample=tick+Duration::from_secs(1);
            if let Ok(s)=t3box::tcp_telemetry::TcpSample::read(&monitor) {
                if let Some(k)=controller.observe(t3box::rate_control::Observation{not_sent_bytes:s.not_sent_bytes,rtt_us:s.rtt_us,retrans_total:s.retransmitted_segments}) {network_rate=k;}
                let desired=if profile=="full" {t3box::content_analysis::content_rate(network_rate,p.content_class())} else {network_rate};
                if desired!=applied {p.reconfigure_rate(desired,500)?;applied=desired;}
            } else {controller.missing_sample();}
        }
        let sent=(||->io::Result<()> {
        while audio+1024<=(i+1)*1600 {
            let block=PcmBlock{pts_samples:4800+audio,sample_rate:48000,channels:2,interleaved:(0..2048).map(|n| (((n/2+audio as usize)%109) as i16-54)*200).collect()};
            for pkt in aac.encode(&block)? { p.push_aac((pkt.pts_samples*1000/48000) as u32,&pkt.raw)?; }
            audio+=1024;
        }
        frame.timestamp_us=100000+i*1_000_000/30;
        frame.sequence=i;
        p.push_frame(&frame)?;
        Ok(())})();
        if let Err(e)=sent {eprintln!("benchmark send failed: {:?}",e.kind());failed=true;break;}
        let done=Instant::now();
        let late=done>due+Duration::from_micros(1_000_000/30);
        if late { missed+=1; }
        let scheduled_wall_ns=wall_start+u128::from(i*1_000_000/30)*1000;
        println!("{{\"frame\":{i},\"scheduled_wall_ns\":{scheduled_wall_ns},\"encode_send_ms\":{},\"completion_ms\":{},\"deadline_missed\":{late}}}",(done-tick).as_secs_f64()*1000.0,(done-due).as_secs_f64()*1000.0);
    }
    if failed {
        drop(p);
        let expired=guard.as_ref().is_some_and(|g|g.handle().expired());
        println!("{{\"aborted\":true,\"watchdog_expired\":{expired},\"elapsed_s\":{}}}",start.elapsed().as_secs_f64());
        return Ok(());
    }
    for pkt in aac.finish()? { p.push_aac((pkt.pts_samples*1000/48000) as u32,&pkt.raw)?; }
    let (_,emitted)=p.finish()?;
    println!("{{\"emitted\":{emitted},\"deadline_misses\":{missed},\"elapsed_s\":{}}}",start.elapsed().as_secs_f64());
    if emitted!=count {return Err(io::Error::other("missing encoded frames"));}
    Ok(())
}
