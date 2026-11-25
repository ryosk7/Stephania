//! Experimental video-only Pi 5 streaming path, all media processing in Rust.
//!
//! A/V combined streaming is NOT ready: only H.264 video is sent, with no AAC.
//! RTMPS must be a private test stream until a full event acceptance test.
#![cfg(all(feature = "native-capture", feature = "native-x264"))]
use std::{env, io};
use t3box::{
    media_core::{VideoSource, PixelFormat},
    native_capture::{CaptureSettings, V4lVideoSource},
    native_video::NativeVideoPipeline,
    rtmps_tls,
};

fn main() {
    if let Err(e)=run() {
        eprintln!("native-video: {e}");
        std::process::exit(1);
    }
}
fn number(key: &str, default: u32) -> io::Result<u32> {
    env::var(key).ok().map(|s|s.parse()
        .map_err(|_|io::Error::new(io::ErrorKind::InvalidInput,"invalid numeric setting")))
        .transpose().map(|value|value.unwrap_or(default))
}
fn run() -> io::Result<()> {
    let url=env::var("T3BOX_OUTPUT")
        .map_err(|_|io::Error::new(io::ErrorKind::InvalidInput,"T3BOX_OUTPUT must be an RTMPS URL"))?;
    let mut capture=CaptureSettings::default();
    if let Ok(path)=env::var("T3BOX_VIDEO"){capture.video_path=path;}
    capture.width=number("T3BOX_WIDTH",1280)?;
    capture.height=number("T3BOX_HEIGHT",720)?;
    let fps=number("T3BOX_FPS",30)?;
    let bitrate=number("T3BOX_BITRATE_KBPS",3000)?;
    capture.preferred_format=match env::var("T3BOX_PIXEL_FORMAT").as_deref(){
        Ok("nv12")=>PixelFormat::Nv12,
        Ok("yuyv")|Err(_)=>PixelFormat::Yuyv422,
        _=>return Err(io::Error::new(io::ErrorKind::InvalidInput,"use T3BOX_PIXEL_FORMAT=yuyv or nv12")),
    };

    let mut source=V4lVideoSource::open(capture.clone())?;
    // Device is checked when next_frame retrieves the first buffer. Connecting
    // here also validates the RTMP publish server response before media transfer.
    let first=source.next_frame()?;
    if first.width!=capture.width || first.height!=capture.height{
        return Err(io::Error::new(io::ErrorKind::InvalidData,"capture frame geometry mismatch"));
    }
    let session=rtmps_tls::begin_publish(&url)?;
    let mut pipeline=NativeVideoPipeline::new(
        session,capture.width as usize,capture.height as usize,fps,bitrate
    )?;
    pipeline.push_frame(&first)?;
    let max_frames=number("T3BOX_MAX_FRAMES",900)?;
    for _ in 1..max_frames {
        let frame=source.next_frame()?;
        pipeline.push_frame(&frame)?;
    }
    let (_,count)=pipeline.finish()?;
    eprintln!("Encoded and sent {count} video access units; audio not implemented");
    Ok(())
}
