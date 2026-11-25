//! Direct libx264 encoder through Rust bindings. No FFmpeg process.
//! This backend is intentionally optional: libx264 is GPL-licensed and must be
//! evaluated for distribution obligations and Pi 5 real-time performance.
//! Output is length-prefixed AVC NALs suitable for T3Box's RTMP packetizer.
use crate::colorspace::I420Frame;
use std::io;
use x264::{Colorspace, Encoder, Image, Plane, Preset, Setup, Tune};

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
        let preset = if fast { Preset::Ultrafast } else { Preset::Veryfast };
        let encoder = Setup::preset(preset, Tune::None, false, true)
            .fps(fps, 1)
            .timebase(1, 1_000_000)
            .bitrate(bitrate_kbps as i32)
            .annexb(false)
            .build(Colorspace::I420, width as i32, height as i32)
            .map_err(codec_error)?;
        Ok(Self { encoder: Some(encoder), width, height, last_input_us: None })
    }

    pub fn configuration(&mut self) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let encoder = self.encoder.as_mut().ok_or_else(||invalid("encoder was flushed"))?;
        let headers = encoder.headers().map_err(codec_error)?;
        let nals = avcc_nals(headers.entirety())?;
        let sps = nals.iter().find(|n| n[0] & 0x1f == 7)
            .cloned().ok_or_else(||invalid("x264 did not provide SPS"))?;
        let pps = nals.iter().find(|n| n[0] & 0x1f == 8)
            .cloned().ok_or_else(||invalid("x264 did not provide PPS"))?;
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
        let y_stride = self.width as i32;
        let uv_stride = (self.width / 2) as i32;
        let planes = [
            Plane { stride: y_stride, data: frame.y() },
            Plane { stride: uv_stride, data: frame.u() },
            Plane { stride: uv_stride, data: frame.v() },
        ];
        let image = Image::new(Colorspace::I420, self.width as i32, self.height as i32, &planes);
        let (data, pic) = encoder.encode(frame.timestamp_us as i64, image).map_err(codec_error)?;
        let bytes = data.entirety();
        let result = if bytes.is_empty() { None } else {
            Some(access_unit(bytes, pic.pts(), pic.dts(), pic.keyframe())?)
        };
        self.last_input_us = Some(frame.timestamp_us);
        Ok(result)
    }

    /// Fully drains delayed frames; the backend cannot be used after finish.
    pub fn finish(&mut self) -> io::Result<Vec<EncodedAccessUnit>> {
        let encoder = self.encoder.take().ok_or_else(||invalid("encoder already flushed"))?;
        let mut result = Vec::new();
        let mut flush = encoder.flush();
        while let Some(frame) = flush.next() {
            let (data, pic) = frame.map_err(codec_error)?;
            if !data.entirety().is_empty() {
                result.push(access_unit(data.entirety(), pic.pts(), pic.dts(), pic.keyframe())?);
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

fn codec_error(error: x264::Error) -> io::Error {
    io::Error::other(format!("libx264 error: {error:?}"))
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
    fn avcc_parser_rejects_empty_and_truncated_packets() {
        assert_eq!(avcc_nals(&[0,0,0,2,0x65,0xcc]).unwrap(), vec![vec![0x65,0xcc]]);
        for input in [&[][..], &[0,0,0,0][..], &[0,0,0,4,0x65][..]] {
            assert!(avcc_nals(input).is_err());
        }
    }

    #[test]
    fn invalid_encoder_config_is_rejected() {
        assert!(X264Backend::new(3,2,30,1000,true).is_err());
        assert!(X264Backend::new(1280,720,0,1000,true).is_err());
    }
}
