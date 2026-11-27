//! Direct libx264 encoder through a narrow C bridge. No FFmpeg process.
//! This backend is intentionally optional: libx264 is GPL-licensed and must be
//! evaluated for distribution obligations and Pi 5 real-time performance.
//! Output is length-prefixed AVC NALs suitable for Stephania's RTMP packetizer.
use crate::colorspace::I420Frame;
use std::{ffi::c_void, io, ptr::NonNull};

#[repr(C)]
struct Packet {
    data: *const u8,
    pts: i64,
    dts: i64,
    keyframe: i32,
}

impl Packet {
    fn new() -> Self {
        Self { data: std::ptr::null(), pts: 0, dts: 0, keyframe: 0 }
    }

    // Only called immediately after a successful encoder call, before
    // another call can invalidate libx264-owned NAL storage.
    unsafe fn bytes(&self, size: i32) -> io::Result<&[u8]> {
        if size <= 0 || size as usize > MAX_ACCESS_UNIT || self.data.is_null() {
            return Err(invalid("invalid x264 output size"));
        }
        Ok(unsafe { std::slice::from_raw_parts(self.data, size as usize) })
    }
}

extern "C" {
    fn t3x264_open(width: i32, height: i32, fps: i32, bitrate: i32, fast: i32) -> *mut c_void;
    fn t3x264_reconfigure(handle: *mut c_void, bitrate: i32, vbv_ms: i32) -> i32;
    fn t3x264_close(handle: *mut c_void);
    fn t3x264_headers(handle: *mut c_void, out: *mut Packet) -> i32;
    fn t3x264_encode(handle: *mut c_void, data: *const u8, width: i32, height: i32,
        pts: i64, out: *mut Packet) -> i32;
    fn t3x264_delayed(handle: *mut c_void) -> i32;
    fn t3x264_rate_control(handle: *mut c_void, bitrate: i32) -> i32;
}

struct Encoder(NonNull<c_void>);
impl Drop for Encoder {
    fn drop(&mut self) { unsafe { t3x264_close(self.0.as_ptr()); } }
}

const MAX_DIMENSION: usize = 3840;
const MAX_ACCESS_UNIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedAccessUnit {
    pub pts_us: u64,
    pub dts_us: u64,
    pub is_keyframe: bool,
    pub nals: Vec<Vec<u8>>,
}

pub struct X264Backend {
    encoder: Option<Encoder>,
    width: usize,
    height: usize,
    last_input_us: Option<u64>,
}

impl X264Backend {
    pub fn new(width: usize, height: usize, fps: u32, bitrate_kbps: u32, fast: bool) -> io::Result<Self> {
        if width < 2 || height < 2 || width > MAX_DIMENSION || height > 2160 ||
            !width.is_multiple_of(2) || !height.is_multiple_of(2) ||
            !(1..=60).contains(&fps) || !(500..=30_000).contains(&bitrate_kbps) {
            return Err(invalid("invalid x264 resolution, fps or bitrate"));
        }
        let encoder = Encoder(NonNull::new(unsafe {
            t3x264_open(width as i32, height as i32, fps as i32, bitrate_kbps as i32, i32::from(fast))
        }).ok_or_else(|| io::Error::other("x264 encoder initialization failed"))?);
        if unsafe { t3x264_rate_control(encoder.0.as_ptr(), bitrate_kbps as i32) } != 1 {
            return Err(io::Error::other("x264 rate control was not applied"));
        }
        Ok(Self { encoder: Some(encoder), width, height, last_input_us: None })
    }

    /// Request a rate change at the next encoder frame; never changes geometry.
    pub fn reconfigure_rate(&mut self, bitrate_kbps: u32, vbv_ms: u32) -> io::Result<()> {
        if !(500..=30_000).contains(&bitrate_kbps) || !(250..=2000).contains(&vbv_ms) {
            return Err(invalid("invalid adaptive bitrate or VBV interval"));
        }
        let encoder = self.encoder.as_mut().ok_or_else(|| invalid("encoder was flushed"))?;
        if unsafe { t3x264_reconfigure(encoder.0.as_ptr(), bitrate_kbps as i32, vbv_ms as i32) } != 0 {
            return Err(io::Error::other("x264 rate reconfiguration failed"));
        }
        Ok(())
    }

    pub fn configuration(&mut self) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let encoder = self.encoder.as_mut().ok_or_else(||invalid("encoder was flushed"))?;
        let mut packet = Packet::new();
        let size = unsafe { t3x264_headers(encoder.0.as_ptr(), &mut packet) };
        let mut nals = avcc_nals(unsafe { packet.bytes(size)? })?;
        let sps_index = nals.iter().position(|n| n[0] & 0x1f == 7)
            .ok_or_else(||invalid("x264 did not provide SPS"))?;
        let sps = nals.swap_remove(sps_index);
        let pps_index = nals.iter().position(|n| n[0] & 0x1f == 8)
            .ok_or_else(||invalid("x264 did not provide PPS"))?;
        let pps = nals.swap_remove(pps_index);
        Ok((sps, pps))
    }

    pub fn encode_i420(&mut self, frame: &I420Frame) -> io::Result<Option<EncodedAccessUnit>> {
        if frame.width != self.width || frame.height != self.height ||
            frame.bytes.len() != self.width * self.height * 3 / 2 ||
            frame.timestamp_us > i64::MAX as u64 ||
            self.last_input_us.is_some_and(|ts| frame.timestamp_us <= ts) {
            return Err(invalid("I420 frame has invalid geometry, length or timestamp"));
        }
        let encoder = self.encoder.as_mut().ok_or_else(||invalid("encoder was flushed"))?;
        let mut packet = Packet::new();
        // x264 reads the validated I420 planes synchronously, copying input
        // into its own frame storage. It does not retain the caller's pointer.
        let size = unsafe { t3x264_encode(encoder.0.as_ptr(), frame.bytes.as_ptr(),
            self.width as i32, self.height as i32, frame.timestamp_us as i64, &mut packet) };
        let result = match size {
            0 => None,
            n if n < 0 => return Err(io::Error::other("x264 encode failed")),
            _ => Some(access_unit(unsafe { packet.bytes(size)? }, packet.pts, packet.dts,
                packet.keyframe != 0)?),
        };
        self.last_input_us = Some(frame.timestamp_us);
        Ok(result)
    }

    /// Fully drains delayed frames; the backend cannot be used after finish.
    pub fn finish(&mut self) -> io::Result<Vec<EncodedAccessUnit>> {
        let encoder = self.encoder.take().ok_or_else(||invalid("encoder already flushed"))?;
        let mut result = Vec::new();
        while unsafe { t3x264_delayed(encoder.0.as_ptr()) } > 0 {
            let mut packet = Packet::new();
            let size = unsafe { t3x264_encode(encoder.0.as_ptr(), std::ptr::null(),
                self.width as i32, self.height as i32, 0, &mut packet) };
            if size < 0 { return Err(io::Error::other("x264 flush failed")); }
            if size > 0 {
                result.push(access_unit(unsafe { packet.bytes(size)? }, packet.pts,
                    packet.dts, packet.keyframe != 0)?);
            }
        }
        Ok(result)
    }
}

fn access_unit(bytes: &[u8], pts: i64, dts: i64, keyframe: bool) -> io::Result<EncodedAccessUnit> {
    if pts < 0 || dts < 0 { return Err(invalid("x264 emitted a negative timestamp")); }
    Ok(EncodedAccessUnit {
        pts_us: pts as u64,
        dts_us: dts as u64,
        is_keyframe: keyframe,
        nals: avcc_nals(bytes)?,
    })
}

/// AVCC contains 4-byte big-endian lengths, not Annex-B start codes.
pub fn avcc_nals(data: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    if data.len() > MAX_ACCESS_UNIT { return Err(invalid("encoded AVC unit too large")); }
    let mut nals = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        if rest.len() < 5 { return Err(invalid("incomplete AVC NAL length")); }
        let size = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
        if size == 0 || size > rest.len() - 4 {
            return Err(invalid("invalid AVC NAL length"));
        }
        let nal = &rest[4..4+size];
        if nal[0] & 0x1f == 0 { return Err(invalid("invalid AVC NAL type")); }
        nals.push(nal.to_vec());
        rest = &rest[4+size..];
        if nals.len() > 128 { return Err(invalid("too many AVC NALs")); }
    }
    if nals.is_empty() { return Err(invalid("empty AVC access unit")); }
    Ok(nals)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoder_config_and_one_software_compressed_frame() {
        let mut enc = X264Backend::new(64, 64, 30, 800, true).unwrap();
        let (sps,pps)=enc.configuration().unwrap();
        assert_eq!(sps[0]&0x1f,7);
        assert_eq!(pps[0]&0x1f,8);
        let input=I420Frame {
            width:64,height:64,timestamp_us:0,
            bytes: {
                let mut v=vec![90u8;64*64];
                v.extend(vec![128u8;64*64/2]);
                v
            },
        };
        let mut encoded = Vec::new();
        if let Some(unit)=enc.encode_i420(&input).unwrap(){encoded.push(unit);}
        for i in 1..4 {
            let mut f=input.clone();
            f.timestamp_us=i*33_333;
            if let Some(unit)=enc.encode_i420(&f).unwrap(){encoded.push(unit);}
        }
        encoded.extend(enc.finish().unwrap());
        assert!(!encoded.is_empty());
        assert!(encoded.iter().any(|f| f.is_keyframe));
        assert!(encoded.iter().all(|f| !f.nals.is_empty()));
    }

    #[test]
    fn keyframe_interval_is_bounded_to_two_seconds() {
        for fast in [true, false] {
            let mut enc = X264Backend::new(64, 64, 30, 800, fast).unwrap();
            let mut frame = I420Frame {
                width: 64,
                height: 64,
                timestamp_us: 0,
                bytes: vec![128; 64 * 64 * 3 / 2],
            };
            let mut units = Vec::new();
            for i in 0..125 {
                frame.timestamp_us = i * 1_000_000 / 30;
                if let Some(unit) = enc.encode_i420(&frame).unwrap() {
                    units.push(unit);
                }
            }
            units.extend(enc.finish().unwrap());
            assert_eq!(units.len(), 125);
            let keys: Vec<_> = units.iter().enumerate()
                .filter_map(|(i, unit)| unit.is_keyframe.then_some(i))
                .collect();
            assert_eq!(keys.first(), Some(&0));
            assert!(keys.windows(2).all(|pair| pair[1] - pair[0] <= 60));
            assert!(units.len() - 1 - keys.last().unwrap() < 60);
        }
    }

    #[test]
    fn abr_and_vbv_match_requested_bitrate() {
        for fast in [true, false] {
            for bitrate in [6000, 8000] {
                let encoder = X264Backend::new(1920, 1080, 30, bitrate, fast).unwrap();
                let handle = encoder.encoder.as_ref().unwrap().0;
                assert_eq!(unsafe { t3x264_rate_control(handle.as_ptr(), bitrate as i32) }, 1);
            }
        }
    }

    #[test]
    fn avcc_parser_rejects_empty_and_truncated_packets() {
        assert_eq!(avcc_nals(&[0,0,0,2,0x65,0xcc]).unwrap(), vec![vec![0x65,0xcc]]);
        for input in [&[][..], &[0,0,0,0][..], &[0,0,0,4,0x65][..]] {
            assert!(avcc_nals(input).is_err());
        }
    }

    #[test]
    fn adaptive_rate_can_change_during_encoding() {
        let mut enc = X264Backend::new(64,64,30,8000,true).unwrap();
        let mut frame = I420Frame { width:64,height:64,timestamp_us:0,bytes:vec![128;64*64*3/2] };
        let mut units = Vec::new();
        for i in 0..12 {
            if i == 4 { enc.reconfigure_rate(6000, 500).unwrap(); }
            if i == 8 { enc.reconfigure_rate(4000, 500).unwrap(); }
            frame.timestamp_us = i*33_333;
            if let Some(unit)=enc.encode_i420(&frame).unwrap() { units.push(unit); }
        }
        units.extend(enc.finish().unwrap());
        assert_eq!(units.len(),12);
        assert!(units[0].is_keyframe);
        assert!(units.windows(2).all(|w| w[0].pts_us < w[1].pts_us));
    }

    #[test]
    fn invalid_encoder_config_is_rejected() {
        assert!(X264Backend::new(3,2,30,1000,true).is_err());
        assert!(X264Backend::new(1280,720,0,1000,true).is_err());
    }
}
