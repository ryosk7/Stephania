//! Safe owner of the direct libavcodec AAC-LC bridge (48 kHz stereo).
use crate::pcm_framer::PcmBlock;
use std::{ffi::c_void, io, ptr::NonNull};

extern "C" {
    fn t3aac_open(bitrate: i32) -> *mut c_void;
    fn t3aac_free(handle: *mut c_void);
    fn t3aac_config(handle: *mut c_void, out: *mut u8, capacity: i32) -> i32;
    fn t3aac_send(handle: *mut c_void, pcm: *const i16, pts: i64) -> i32;
    fn t3aac_receive(handle: *mut c_void, out: *mut u8, capacity: i32, pts: *mut i64) -> i32;
}

#[derive(Debug)]
pub struct AacPacket {
    pub pts_samples: i64,
    pub raw: Vec<u8>,
}

pub struct AacBackend {
    handle: NonNull<c_void>,
    last_pts: Option<i64>,
    finished: bool,
}

impl AacBackend {
    pub fn new(bitrate_kbps: u32) -> io::Result<Self> {
        if !(64..=320).contains(&bitrate_kbps) { return Err(invalid("AAC bitrate out of range")); }
        let handle = NonNull::new(unsafe { t3aac_open((bitrate_kbps * 1000) as i32) })
            .ok_or_else(|| io::Error::other("AAC encoder initialization failed"))?;
        Ok(Self { handle, last_pts: None, finished: false })
    }

    pub fn configuration(&self) -> io::Result<Vec<u8>> {
        let mut data = vec![0; 16];
        let n = unsafe { t3aac_config(self.handle.as_ptr(), data.as_mut_ptr(), 16) };
        if !(2..=16).contains(&n) { return Err(io::Error::other("invalid AAC extradata")); }
        data.truncate(n as usize);
        Ok(data)
    }

    pub fn encode(&mut self, block: &PcmBlock) -> io::Result<Vec<AacPacket>> {
        let pts = i64::try_from(block.pts_samples).map_err(|_| invalid("AAC PTS overflow"))?;
        if self.finished || block.sample_rate != 48000 || block.channels != 2 ||
            block.interleaved.len() != 2048 ||
            self.last_pts.is_some_and(|last| last.checked_add(1024) != Some(pts)) {
            return Err(invalid("invalid or discontinuous AAC PCM block"));
        }
        // The bridge copies exactly 2048 validated samples before returning.
        let ret = unsafe { t3aac_send(self.handle.as_ptr(), block.interleaved.as_ptr(), pts) };
        if ret < 0 { return Err(io::Error::other("AAC frame rejected")); }
        self.last_pts = Some(pts);
        self.receive()
    }

    pub fn finish(&mut self) -> io::Result<Vec<AacPacket>> {
        if self.finished { return Err(invalid("AAC encoder already finished")); }
        self.finished = true;
        if unsafe { t3aac_send(self.handle.as_ptr(), std::ptr::null(), 0) } < 0 {
            return Err(io::Error::other("AAC flush failed"));
        }
        self.receive()
    }

    fn receive(&mut self) -> io::Result<Vec<AacPacket>> {
        let mut packets = Vec::new();
        for _ in 0..64 {
            let mut raw = vec![0; 8192];
            let mut pts = 0;
            let n = unsafe { t3aac_receive(self.handle.as_ptr(), raw.as_mut_ptr(), 8192, &mut pts) };
            if n == 0 { return Ok(packets); }
            if !(1..=8192).contains(&n) { return Err(io::Error::other("AAC packet failed")); }
            raw.truncate(n as usize);
            packets.push(AacPacket { pts_samples: pts, raw });
        }
        Err(io::Error::other("AAC output limit exceeded"))
    }
}

impl Drop for AacBackend {
    fn drop(&mut self) { unsafe { t3aac_free(self.handle.as_ptr()); } }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encodes_and_flushes_real_aac_lc() {
        let mut encoder = AacBackend::new(128).unwrap();
        assert_eq!(&encoder.configuration().unwrap()[..2], &[0x11, 0x90]);
        let mut packets = Vec::new();
        for i in 0..8 {
            let pcm = PcmBlock { pts_samples: i * 1024, sample_rate: 48000,
                channels: 2, interleaved: (0..2048).map(|n| ((n % 100) * 100 - 5000) as i16).collect() };
            packets.extend(encoder.encode(&pcm).unwrap());
        }
        packets.extend(encoder.finish().unwrap());
        assert!(packets.len() >= 8);
        assert!(packets.windows(2).all(|p| p[1].pts_samples - p[0].pts_samples == 1024));
        assert!(packets.iter().all(|p| !p.raw.is_empty()));
    }
}
