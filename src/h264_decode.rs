//! H.264 decoding of phone camera tracks (docs/PHONE.md) via libavcodec
//! (`src/h264dec_bridge.c`, linked with the AAC encoder). Input is one
//! Annex-B access unit at a time; output is tightly packed I420 for the
//! compositor's source registry.
use crate::colorspace::I420Frame;
use std::ffi::c_void;
use std::io;
use std::ptr::NonNull;

extern "C" {
    fn t3dec_open(threads: i32, max_pixels: i64) -> *mut c_void;
    fn t3dec_close(s: *mut c_void);
    fn t3dec_send(s: *mut c_void, data: *const u8, len: i32, pts: i64) -> i32;
    fn t3dec_receive(s: *mut c_void, w: *mut i32, h: *mut i32, pts: *mut i64) -> i32;
    fn t3dec_copy_i420(s: *mut c_void, out: *mut u8, capacity: i32) -> i32;
}

/// Largest accepted access unit and picture (bounds memory per source).
pub const MAX_ACCESS_UNIT: usize = 2 * 1024 * 1024;
pub const MAX_WIDTH: usize = 1920;
pub const MAX_HEIGHT: usize = 1920;
/// Passed to libavcodec as `max_pixels`, so an oversized SPS is refused
/// before it allocates pictures (memory per decoder stays bounded).
pub const MAX_PIXELS: usize = MAX_WIDTH * MAX_HEIGHT;

pub struct H264Decoder(NonNull<c_void>);

// SAFETY: the decoder context is used from one thread at a time (&mut self).
unsafe impl Send for H264Decoder {}

impl Drop for H264Decoder {
    fn drop(&mut self) { unsafe { t3dec_close(self.0.as_ptr()) } }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Corrupt or undecodable input (e.g. missing reference after loss):
    /// the caller should request a keyframe.
    NeedKeyframe,
    /// Input outside the accepted bounds; dropped.
    Rejected(&'static str),
}

impl H264Decoder {
    pub fn new(threads: i32) -> io::Result<Self> {
        NonNull::new(unsafe { t3dec_open(threads.clamp(1, 4), MAX_PIXELS as i64) })
            .map(Self)
            .ok_or_else(|| io::Error::other("H.264 decoder initialization failed"))
    }

    /// Decodes one Annex-B access unit; returns every picture it completed.
    pub fn decode(&mut self, access_unit: &[u8], pts_us: u64) -> Result<Vec<I420Frame>, DecodeError> {
        if access_unit.is_empty() || access_unit.len() > MAX_ACCESS_UNIT {
            return Err(DecodeError::Rejected("access unit size"));
        }
        let pts = i64::try_from(pts_us).map_err(|_| DecodeError::Rejected("timestamp"))?;
        // libavcodec copies what it needs before returning (refcounted
        // packet created from caller memory is consumed synchronously).
        if unsafe { t3dec_send(self.0.as_ptr(), access_unit.as_ptr(), access_unit.len() as i32, pts) } < 0 {
            return Err(DecodeError::NeedKeyframe);
        }
        let mut frames = Vec::new();
        loop {
            let (mut w, mut h, mut out_pts) = (0i32, 0i32, 0i64);
            match unsafe { t3dec_receive(self.0.as_ptr(), &mut w, &mut h, &mut out_pts) } {
                0 => return Ok(frames),
                1 => {}
                -2 => return Err(DecodeError::Rejected("pixel format is not 4:2:0")),
                _ => return Err(DecodeError::NeedKeyframe),
            }
            let (w, h) = (w as usize, h as usize);
            if w > MAX_WIDTH || h > MAX_HEIGHT || w % 2 != 0 || h % 2 != 0 {
                return Err(DecodeError::Rejected("picture size"));
            }
            let mut bytes = vec![0u8; w * h * 3 / 2];
            if unsafe { t3dec_copy_i420(self.0.as_ptr(), bytes.as_mut_ptr(), bytes.len() as i32) } < 0 {
                return Err(DecodeError::Rejected("copy"));
            }
            frames.push(I420Frame { width: w, height: h, timestamp_us: out_pts.max(0) as u64, bytes });
        }
    }
}

/// Joins NAL units into one Annex-B access unit (4-byte start codes).
pub fn annex_b(nals: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nals.iter().map(|n| n.len() + 4).sum());
    for n in nals {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(n);
    }
    out
}

#[cfg(all(test, feature = "native-x264"))]
mod tests {
    use super::*;
    use crate::native_x264::X264Backend;

    fn solid(w: usize, h: usize, (y, u, v): (u8, u8, u8), ts: u64) -> I420Frame {
        let mut bytes = vec![y; w * h];
        bytes.extend(std::iter::repeat_n(u, w * h / 4));
        bytes.extend(std::iter::repeat_n(v, w * h / 4));
        I420Frame { width: w, height: h, timestamp_us: ts, bytes }
    }

    fn mean(p: &[u8]) -> f64 { p.iter().map(|&b| b as f64).sum::<f64>() / p.len() as f64 }

    #[test]
    fn decodes_what_the_native_encoder_produces() {
        let (w, h) = (640, 360);
        let mut enc = X264Backend::new(w, h, 30, 2000, true).unwrap();
        let (sps, pps) = enc.configuration().unwrap();
        let mut dec = H264Decoder::new(1).unwrap();
        let mut decoded = Vec::new();
        for i in 0..30u64 {
            let color = if i < 15 { (105, 212, 234) } else { (41, 240, 110) };
            if let Some(au) = enc.encode_i420(&solid(w, h, color, i * 33_333)).unwrap() {
                let mut nals: Vec<&[u8]> = Vec::new();
                if au.is_keyframe { nals.push(&sps); nals.push(&pps); }
                nals.extend(au.nals.iter().map(Vec::as_slice));
                decoded.extend(dec.decode(&annex_b(&nals), au.pts_us).unwrap());
            }
        }
        for au in enc.finish().unwrap() {
            let nals: Vec<&[u8]> = au.nals.iter().map(Vec::as_slice).collect();
            decoded.extend(dec.decode(&annex_b(&nals), au.pts_us).unwrap());
        }
        assert!(decoded.len() >= 25, "decoded {} frames", decoded.len());
        let first = &decoded[2];
        assert_eq!((first.width, first.height), (w, h));
        assert!((mean(first.y()) - 105.0).abs() < 3.0, "Y {}", mean(first.y()));
        assert!((mean(first.u()) - 212.0).abs() < 3.0);
        let last = decoded.last().unwrap();
        assert!((mean(last.y()) - 41.0).abs() < 3.0, "Y {}", mean(last.y()));
    }

    /// An SPS above MAX_PIXELS never yields a picture: libavcodec refuses it
    /// at activation instead of allocating frames for it.
    #[test]
    fn oversized_sps_is_refused_before_decoding() {
        let (w, h) = (2560, 1920);
        let mut enc = X264Backend::new(w, h, 30, 2000, true).unwrap();
        let (sps, pps) = enc.configuration().unwrap();
        let mut dec = H264Decoder::new(1).unwrap();
        let au = enc.encode_i420(&solid(w, h, (16, 128, 128), 0)).unwrap()
            .or_else(|| enc.finish().unwrap().into_iter().next()).unwrap();
        let mut nals: Vec<&[u8]> = vec![&sps, &pps];
        nals.extend(au.nals.iter().map(Vec::as_slice));
        let r = dec.decode(&annex_b(&nals), 0);
        assert!(matches!(r, Err(DecodeError::NeedKeyframe)) || matches!(r, Ok(ref f) if f.is_empty()), "{r:?}");
    }

    #[test]
    fn garbage_asks_for_a_keyframe_and_bounds_are_enforced() {
        let mut dec = H264Decoder::new(1).unwrap();
        assert_eq!(dec.decode(&[], 0), Err(DecodeError::Rejected("access unit size")));
        assert_eq!(dec.decode(&vec![0u8; MAX_ACCESS_UNIT + 1], 0), Err(DecodeError::Rejected("access unit size")));
        // A P-slice NAL with no prior SPS/PPS decodes to nothing.
        let r = dec.decode(&annex_b(&[&[0x41, 0x9a, 0x00, 0x11, 0x22]]), 0);
        assert!(matches!(r, Ok(ref f) if f.is_empty()) || r == Err(DecodeError::NeedKeyframe));
    }
}
