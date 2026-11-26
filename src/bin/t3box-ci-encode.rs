//! Offline benchmark of production color conversion and native encoders.
//! Raw YUYV input, H.264 Annex B and AAC ADTS output. No device or network.
use std::{env, fs::File, io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write}, time::Instant};
use t3box::{colorspace, native_x264::X264Backend, native_aac::AacBackend,
    av_clock::AudioClock, media_core::AudioFrame, pcm_framer::PcmFramer};

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 7 { return Err(io::Error::other("usage: input output frames bitrate preset audio-output")); }
    let frames: u64 = args[3].parse().map_err(io::Error::other)?;
    let bitrate: u32 = args[4].parse().map_err(io::Error::other)?;
    let fast = match args[5].as_str() { "ultrafast" => true, "veryfast" => false,
        _ => return Err(io::Error::other("unknown preset")) };
    let mut input = BufReader::new(File::open(&args[1])?);
    let mut out = BufWriter::new(File::create(&args[2])?);
    let mut audio_out = BufWriter::new(File::create(&args[6])?);
    let mut encoder = X264Backend::new(1920,1080,30,bitrate,fast)?;
    let mut aac = AacBackend::new(128)?;
    let mut clock = AudioClock::default();
    let mut framer = PcmFramer::new(48000,2)?;
    let (sps,pps) = encoder.configuration()?;
    for nal in [&sps,&pps] { out.write_all(&[0,0,0,1])?; out.write_all(nal)?; }
    let mut raw = vec![0;1920*1080*2];
    let soak = env::var_os("T3BOX_CI_SOAK").is_some();
    let mut durations = Vec::new();
    let mut emitted = 0u64;
    let mut audio_packets = 0u64;
    let start = Instant::now();
    for i in 0..frames {
        if soak && i % 120 == 0 { input.seek(SeekFrom::Start(0))?; }
        if soak {
            let deadline = start + std::time::Duration::from_micros(i * 1_000_000 / 30);
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
        let tick = Instant::now();
        input.read_exact(&mut raw)?;
        let frame = colorspace::yuyv422_to_i420(&raw,1920,1080,3840,1000000+i*1000000/30)?;
        if let Some(unit) = encoder.encode_i420(&frame)? {
            for nal in unit.nals { out.write_all(&[0,0,0,1])?; out.write_all(&nal)?; }
            emitted += 1;
        }
        let samples: Vec<i16> = (0..1600).flat_map(|j| {
            let x = (((i*1600+j)%109) as i16 -54)*200; [x,x]
        }).collect();
        let pcm = AudioFrame { timestamp_us:100000+i*1000000/30,
            sample_rate:48000,channels:2,samples };
        for block in framer.push(&clock.correct(&pcm)?)? {
            for p in aac.encode(&block)? {
                write_adts(&mut audio_out,&p.raw)?;
                audio_packets += 1;
            }
        }
        durations.push(tick.elapsed().as_secs_f64()*1000.0);
        if soak && i % 1800 == 1799 {
            eprintln!("soak frames={} elapsed_s={:.2}", i+1, start.elapsed().as_secs_f64());
            if start.elapsed().as_secs_f64() > (i+1) as f64 / 30.0 + 10.0 {
                return Err(io::Error::other("soak missed real-time deadline by >10s"));
            }
        }
    }
    for unit in encoder.finish()? {
        for nal in unit.nals { out.write_all(&[0,0,0,1])?; out.write_all(&nal)?; }
        emitted += 1;
    }
    for p in aac.finish()? { write_adts(&mut audio_out,&p.raw)?; audio_packets += 1; }
    out.flush()?; audio_out.flush()?;
    let elapsed = start.elapsed().as_secs_f64();
    durations.sort_by(f64::total_cmp);
    if emitted != frames || frames == 0 { return Err(io::Error::other("video frame count mismatch")); }
    println!("{{\"frames\":{emitted},\"audio_packets\":{audio_packets},\"seconds\":{elapsed},\"fps\":{},\"p95_ms\":{}}}",
        emitted as f64/elapsed,durations[(durations.len()-1)*95/100]);
    Ok(())
}
fn write_adts(out: &mut impl Write, raw: &[u8]) -> io::Result<()> {
    let n = raw.len()+7;
    if n > 8191 { return Err(io::Error::other("AAC frame too large")); }
    out.write_all(&[0xff,0xf1,0x4c,0x80|((n>>11) as u8),
        (n>>3) as u8, ((n&7)<<5) as u8|0x1f,0xfc])?;
    out.write_all(raw)
}
