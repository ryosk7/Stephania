//! Send a pre-encoded FLV stream through the native Rust RTMPS publisher.
//!
//! Usage: T3BOX_OUTPUT=rtmps://.../app/KEY t3box-send-flv -
//!        T3BOX_OUTPUT=rtmps://.../app/KEY t3box-send-flv /path/video.flv
//! The stream key is read from the environment, never CLI arguments.
use std::env;
use std::fs::File;
use std::io::{self, Read};
use t3box::{flv_input::FlvReader, flv_relay, rtmps_tls};

fn main() {
    if let Err(error) = run() {
        eprintln!("t3box-send-flv: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let source = args.next().ok_or_else(|| io::Error::new(
        io::ErrorKind::InvalidInput,
        "provide a FLV filename or '-' for piped FFmpeg stdout",
    ))?;
    if args.next().is_some() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "expected one input argument"));
    }
    let url = env::var("T3BOX_OUTPUT").map_err(|_| io::Error::new(
        io::ErrorKind::InvalidInput,
        "T3BOX_OUTPUT RTMPS URL was not provided",
    ))?;
    let input: Box<dyn Read> = if source == "-" {
        Box::new(io::stdin())
    } else {
        Box::new(File::open(&source)?)
    };
    let mut reader = FlvReader::new(input)?;
    // A verified TLS transport and RTMP publish acknowledgement are required.
    // There is intentionally no insecure "rtmp://" fallback.
    let mut session = rtmps_tls::begin_publish(&url)?;
    let count = flv_relay::relay(&mut reader, &mut session, true)?;
    eprintln!("FLV input ended after {count} tags");
    Ok(())
}
