//! Benchmark-only paced file source through the production RTMPS pipeline.
use std::{env,fs::File,io::{self,Read},thread,time::{Instant,Duration}};
use t3box::{native_video::NativeVideoPipeline,native_aac::AacBackend,pcm_framer::PcmBlock,media_core::{VideoFrame,PixelFormat},rtmps_tls};

fn main()->io::Result<()> {
    let a:Vec<String>=env::args().collect();
    let count:u64=a[2].parse().map_err(io::Error::other)?;
    let rate:u32=a[3].parse().map_err(io::Error::other)?;
    let mut input=File::open(&a[1])?;
    let mut aac=AacBackend::new(128)?;
    let mut p=NativeVideoPipeline::new(rtmps_tls::begin_publish(&a[4])?,1920,1080,30,rate)?;
    p.configure_audio(&aac.configuration()?)?;
    let mut frame=VideoFrame{timestamp_us:0,sequence:0,width:1920,height:1080,row_stride_bytes:3840,format:PixelFormat::Yuyv422,bytes:vec![0;1920*1080*2]};
    let mut audio=0u64;
    let start=Instant::now();
    let mut missed=0;
    for i in 0..count {
        input.read_exact(&mut frame.bytes)?;
        let due=start+Duration::from_micros(i*1_000_000/30);
        thread::sleep(due.saturating_duration_since(Instant::now()));
        let tick=Instant::now();
        while audio+1024<=(i+1)*1600 {
            let block=PcmBlock{pts_samples:4800+audio,sample_rate:48000,channels:2,interleaved:(0..2048).map(|n| (((n/2+audio as usize)%109) as i16-54)*200).collect()};
            for pkt in aac.encode(&block)? { p.push_aac((pkt.pts_samples*1000/48000) as u32,&pkt.raw)?; }
            audio+=1024;
        }
        frame.timestamp_us=100000+i*1_000_000/30;
        frame.sequence=i;
        p.push_frame(&frame)?;
        let done=Instant::now();
        let late=done>due+Duration::from_micros(1_000_000/30);
        if late { missed+=1; }
        println!("{{\"frame\":{i},\"encode_send_ms\":{},\"completion_ms\":{},\"deadline_missed\":{late}}}",(done-tick).as_secs_f64()*1000.0,(done-due).as_secs_f64()*1000.0);
    }
    for pkt in aac.finish()? { p.push_aac((pkt.pts_samples*1000/48000) as u32,&pkt.raw)?; }
    let (_,emitted)=p.finish()?;
    println!("{{\"emitted\":{emitted},\"deadline_misses\":{missed},\"elapsed_s\":{}}}",start.elapsed().as_secs_f64());
    if emitted!=count {return Err(io::Error::other("missing encoded frames"));}
    Ok(())
}
