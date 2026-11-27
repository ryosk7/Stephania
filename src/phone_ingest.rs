//! Phone media ingest (docs/PHONE.md). `stephania-phone` terminates WebRTC
//! (ICE, DTLS-SRTP, RTP) in its own process and forwards depacketized H.264
//! access units here over a Unix socket; the engine decodes them and
//! submits frames to the compositor's source registry. A WebRTC failure
//! therefore never takes the media engine down, and no network-facing
//! protocol stack runs inside it.
//!
//! Framing (both directions): `u32be length ‖ payload`, payload ≤ 2 MiB+10.
//! - client → engine: `0x01 ‖ source ‖ pts_us u64be ‖ Annex-B access unit`
//! - engine → client: `0x02 ‖ source` — request a keyframe (decoder lost
//!   sync, or the stream started without one). Rate-limited per source.
//!
//! Sources: `0x01` = `phone:rear`, `0x02` = `phone:front`.
//! One client at a time (one phone, §12); a new connection replaces the old.
use crate::colorspace::I420Frame;
use crate::h264_decode::{DecodeError, H264Decoder, MAX_ACCESS_UNIT};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

pub const KIND_ACCESS_UNIT: u8 = 1;
pub const KIND_KEYFRAME_REQUEST: u8 = 2;
pub const MAX_PAYLOAD: usize = MAX_ACCESS_UNIT + 10;
pub const KEYFRAME_REQUEST_INTERVAL: Duration = Duration::from_millis(500);
/// A silent peer is dropped (the phone service sends at least RTCP-driven
/// traffic; an idle camera still sends frames).
pub const READ_TIMEOUT: Duration = Duration::from_secs(10);

pub fn source_name(id: u8) -> Option<&'static str> {
    match id {
        1 => Some("phone:rear"),
        2 => Some("phone:front"),
        _ => None,
    }
}

pub fn encode_access_unit(source: u8, pts_us: u64, annex_b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + annex_b.len());
    out.extend_from_slice(&((10 + annex_b.len()) as u32).to_be_bytes());
    out.push(KIND_ACCESS_UNIT);
    out.push(source);
    out.extend_from_slice(&pts_us.to_be_bytes());
    out.extend_from_slice(annex_b);
    out
}

fn read_payload<R: Read>(r: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_be_bytes(len) as usize;
    if !(2..=MAX_PAYLOAD).contains(&n) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "payload length out of bounds"));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IngestStats {
    pub access_units: u64,
    pub frames: u64,
    pub keyframe_requests: u64,
    pub rejected: u64,
}

struct SourceState {
    decoder: H264Decoder,
    synced: bool,
    last_request: Option<Instant>,
}

/// Serves one client connection until EOF or a protocol error. `sink`
/// receives every decoded frame with its source name.
pub fn serve_client<S: Read + Write>(stream: &mut S, sink: &mut dyn FnMut(&'static str, I420Frame)) -> io::Result<IngestStats> {
    let mut stats = IngestStats::default();
    let mut sources: [Option<SourceState>; 2] = [None, None];
    while let Some(payload) = read_payload(stream)? {
        if payload[0] != KIND_ACCESS_UNIT || payload.len() < 11 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected message kind"));
        }
        let id = payload[1];
        let Some(name) = source_name(id) else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown source"));
        };
        let pts = u64::from_be_bytes(payload[2..10].try_into().expect("8 bytes"));
        let au = &payload[10..];
        stats.access_units += 1;
        let slot = &mut sources[(id - 1) as usize];
        if slot.is_none() {
            *slot = Some(SourceState { decoder: H264Decoder::new(2)?, synced: false, last_request: None });
        }
        let state = slot.as_mut().expect("initialised");
        let need_keyframe = match state.decoder.decode(au, pts) {
            Ok(frames) => {
                if !frames.is_empty() { state.synced = true; }
                for f in frames { stats.frames += 1; sink(name, f); }
                !state.synced
            }
            Err(DecodeError::NeedKeyframe) => { state.synced = false; true }
            Err(DecodeError::Rejected(_)) => { stats.rejected += 1; false }
        };
        if need_keyframe && state.last_request.is_none_or(|t| t.elapsed() >= KEYFRAME_REQUEST_INTERVAL) {
            state.last_request = Some(Instant::now());
            stats.keyframe_requests += 1;
            stream.write_all(&[0, 0, 0, 2, KIND_KEYFRAME_REQUEST, id])?;
            stream.flush()?;
        }
    }
    Ok(stats)
}

/// Accept loop on the ingest socket; frames go to the compositor.
#[cfg(unix)]
pub fn serve(listener: std::os::unix::net::UnixListener, policy: crate::ipc::PeerPolicy) {
    let own = unsafe { libc::getuid() };
    for conn in listener.incoming() {
        let Ok(mut stream) = conn else { continue };
        let permitted = match crate::ipc::peer_uid(&stream) { Some(uid) => policy.allows(uid, own), None => !cfg!(target_os = "linux") };
        if !permitted { continue; }
        let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        let registry = crate::composition::sources();
        let result = serve_client(&mut stream, &mut |name, frame| registry.submit(name, frame));
        match result {
            Ok(s) => eprintln!("Stephania phone ingest: client closed ({} AUs, {} frames, {} keyframe requests)", s.access_units, s.frames, s.keyframe_requests),
            Err(e) => eprintln!("Stephania phone ingest: client dropped: {e}"),
        }
    }
}

#[cfg(all(test, feature = "native-x264"))]
mod tests {
    use super::*;
    use crate::h264_decode::annex_b;
    use crate::native_x264::X264Backend;

    /// In-memory duplex: reads scripted input, records writes.
    struct Duplex { input: io::Cursor<Vec<u8>>, output: Vec<u8> }
    impl Read for Duplex { fn read(&mut self, b: &mut [u8]) -> io::Result<usize> { self.input.read(b) } }
    impl Write for Duplex {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> { self.output.extend_from_slice(b); Ok(b.len()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    fn encoded(n: u64, y: u8) -> Vec<(bool, u64, Vec<u8>)> {
        let (w, h) = (320, 240);
        let mut enc = X264Backend::new(w, h, 30, 1000, true).unwrap();
        let (sps, pps) = enc.configuration().unwrap();
        let mut out = Vec::new();
        for i in 0..n {
            let mut bytes = vec![y; w * h];
            bytes.extend(std::iter::repeat_n(128u8, w * h / 2));
            let f = I420Frame { width: w, height: h, timestamp_us: i * 33_333, bytes };
            if let Some(au) = enc.encode_i420(&f).unwrap() {
                let mut nals: Vec<&[u8]> = Vec::new();
                if au.is_keyframe { nals.push(&sps); nals.push(&pps); }
                nals.extend(au.nals.iter().map(Vec::as_slice));
                out.push((au.is_keyframe, au.pts_us, annex_b(&nals)));
            }
        }
        out
    }

    #[test]
    fn decodes_both_sources_and_requests_keyframes_when_joining_mid_stream() {
        let rear = encoded(20, 200);
        let front = encoded(20, 60);
        let mut input = Vec::new();
        // Front joins mid-GOP: its first AUs are P frames (no keyframe).
        for (_, pts, au) in &rear { input.extend(encode_access_unit(1, *pts, au)); }
        for (key, pts, au) in front.iter().skip(1) { assert!(!key); input.extend(encode_access_unit(2, *pts, au)); }
        let mut io = Duplex { input: io::Cursor::new(input), output: Vec::new() };
        let mut got: Vec<(&str, u8)> = Vec::new();
        let stats = serve_client(&mut io, &mut |name, f| got.push((name, f.y()[0]))).unwrap();
        assert!(got.iter().filter(|(n, _)| *n == "phone:rear").count() >= 15);
        assert!(got.iter().all(|(n, y)| *n == "phone:rear" && (*y as i32 - 200).abs() < 4), "front must not decode without a keyframe");
        assert!(stats.keyframe_requests >= 1);
        assert_eq!(&io.output[..6], &[0, 0, 0, 2, KIND_KEYFRAME_REQUEST, 2], "keyframe request for the front source");
    }

    #[test]
    fn protocol_violations_drop_the_client() {
        let mut bad_source = Duplex { input: io::Cursor::new(encode_access_unit(9, 0, &[0, 0, 0, 1, 0x65])), output: Vec::new() };
        assert!(serve_client(&mut bad_source, &mut |_, _| {}).is_err());
        let mut too_long = Duplex { input: io::Cursor::new(((MAX_PAYLOAD + 1) as u32).to_be_bytes().to_vec()), output: Vec::new() };
        assert!(serve_client(&mut too_long, &mut |_, _| {}).is_err());
        let mut empty = Duplex { input: io::Cursor::new(Vec::new()), output: Vec::new() };
        assert_eq!(serve_client(&mut empty, &mut |_, _| {}).unwrap(), IngestStats::default());
    }
}
