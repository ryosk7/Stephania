//! Native video path: V4L2 frame -> Rust I420 conversion -> libx264 -> RTMPS.
//! This is a video-only engineering integration. The venue audio path still
//! requires native AAC encoding and sample-clock synchronization.
use crate::{
    colorspace::{self, I420Frame},
    media_core::{PixelFormat, VideoFrame},
    media_output::EncodedPublisher,
    native_x264::{EncodedAccessUnit, X264Backend},
    session::PublishSession,
};
use std::io::{self, Read, Write};

pub struct NativeVideoPipeline<T: Read + Write> {
    encoder: X264Backend,
    publisher: EncodedPublisher<T>,
    emitted: u64,
    i420: I420Frame,
}

impl<T: Read + Write> NativeVideoPipeline<T> {
    pub fn new(
        session: PublishSession<T>, width: usize, height: usize, fps: u32, bitrate_kbps: u32,
    ) -> io::Result<Self> {
        let mut encoder = X264Backend::new(width, height, fps, bitrate_kbps, true)?;
        let mut publisher = EncodedPublisher::new(session)?;
        let (sps, pps) = encoder.configuration()?;
        publisher.avc_configuration(0, &sps, &pps)?;
        Ok(Self { encoder, publisher, emitted: 0, i420: I420Frame {
            width, height, timestamp_us: 0, bytes: Vec::new(),
        } })
    }

    pub fn push_frame(&mut self, frame: &VideoFrame) -> io::Result<()> {
        let width = frame.width as usize;
        let height = frame.height as usize;
        match frame.format {
            PixelFormat::Yuyv422 => colorspace::yuyv422_to_i420_into(
                &frame.bytes, width, height, frame.row_stride_bytes, frame.timestamp_us, &mut self.i420
            )?,
            PixelFormat::Nv12 => colorspace::nv12_to_i420_into(
                &frame.bytes, width, height, (frame.row_stride_bytes, frame.row_stride_bytes),
                frame.timestamp_us, &mut self.i420
            )?,
            PixelFormat::Mjpeg => return Err(invalid("MJPEG capture decoding is not yet implemented")),
        };
        if let Some(unit) = self.encoder.encode_i420(&self.i420)? {
            self.publish_unit(&unit)?;
        }
        Ok(())
    }

    fn publish_unit(&mut self, unit: &EncodedAccessUnit) -> io::Result<()> {
        if unit.pts_us > u64::from(u32::MAX) * 1000 ||
            unit.dts_us > u64::from(u32::MAX) * 1000 {
            return Err(invalid("RTMP timestamp overflow"));
        }
        let composition_us = i128::from(unit.pts_us) - i128::from(unit.dts_us);
        let cts_ms = i32::try_from(composition_us / 1000)
            .map_err(|_| invalid("composition offset overflow"))?;
        let nal_refs: Vec<&[u8]> = unit.nals.iter().map(Vec::as_slice).collect();
        self.publisher.avc_frame(
            (unit.dts_us / 1000) as u32, cts_ms, unit.is_keyframe, &nal_refs,
        )?;
        self.emitted += 1;
        Ok(())
    }

    /// Send AAC on the same confirmed RTMP session as video.
    pub fn configure_audio(&mut self, asc: &[u8]) -> io::Result<()> {
        self.publisher.aac_configuration(0, asc)
    }

    pub fn push_aac(&mut self, timestamp_ms: u32, raw: &[u8]) -> io::Result<()> {
        self.publisher.aac_frame(timestamp_ms, raw)
    }

    /// Push delayed frames before disconnecting.
    pub fn finish(mut self) -> io::Result<(PublishSession<T>, u64)> {
        for unit in self.encoder.finish()? {
            self.publish_unit(&unit)?;
        }
        Ok((self.publisher.into_session(), self.emitted))
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{amf0, rtmp::RtmpChunkWriter, rtmp_read::ChunkReader};
    use std::io::Cursor;
    struct Duplex { incoming: Cursor<Vec<u8>>, outgoing: Vec<u8> }
    impl Read for Duplex {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> { self.incoming.read(out) }
    }
    impl Write for Duplex {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.outgoing.extend_from_slice(data); Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    fn command(txn: f64, result: amf0::Value<'_>) -> Vec<u8> {
        let mut p=Vec::new();
        amf0::encode(&mut p, amf0::Value::String("_result")).unwrap();
        amf0::encode(&mut p, amf0::Value::Number(txn)).unwrap();
        amf0::encode(&mut p, amf0::Value::Null).unwrap();
        amf0::encode(&mut p, result).unwrap();
        p
    }
    fn status() -> Vec<u8> {
        let mut p=Vec::new();
        amf0::encode(&mut p, amf0::Value::String("onStatus")).unwrap();
        amf0::encode(&mut p, amf0::Value::Number(0.0)).unwrap();
        amf0::encode(&mut p, amf0::Value::Null).unwrap();
        amf0::encode_object(&mut p, &[("code", amf0::Value::String("NetStream.Publish.Start"))]).unwrap();
        p
    }
    fn mock_session() -> PublishSession<Duplex> {
        let mut w=RtmpChunkWriter::new(Vec::new(), 128).unwrap();
        w.message(3,20,0,0,&command(1.0,amf0::Value::Null)).unwrap();
        w.message(3,20,0,0,&command(2.0,amf0::Value::Number(1.0))).unwrap();
        w.message(3,20,1,0,&status()).unwrap();
        let io=Duplex { incoming: Cursor::new(w.into_inner()), outgoing: vec![] };
        let mut session=PublishSession::new(io);
        session.connect("live2","rtmps://example.test/live2").unwrap();
        session.create_stream().unwrap();
        session.publish("test-key").unwrap();
        session
    }
    #[test]
    fn raw_yuyv_to_h264_to_native_rtmp_video_packets() {
        let mut p=NativeVideoPipeline::new(mock_session(),64,64,30,800).unwrap();
        let mut data=vec![0u8;64*64*2];
        for x in data.as_chunks_mut::<4>().0 { x.copy_from_slice(&[82,128,82,128]); }
        for i in 0..4 {
            p.push_frame(&VideoFrame {
                timestamp_us: i*33_333,
                sequence: i,
                width:64,height:64,row_stride_bytes:128,
                format:PixelFormat::Yuyv422, bytes:data.clone(),
            }).unwrap();
        }
        let (session,count)=p.finish().unwrap();
        assert!(count>=1);
        let bytes=session.into_inner().outgoing;
        let mut reader=ChunkReader::new();
        let mut input=bytes.as_slice();
        for _ in 0..3 { assert_eq!(reader.read_message(&mut input).unwrap().type_id,20); }
        let config=reader.read_message(&mut input).unwrap();
        assert_eq!(config.type_id,9);
        assert_eq!(&config.payload[..2],&[0x17,0]);
        let frame=reader.read_message(&mut input).unwrap();
        assert_eq!(frame.type_id,9);
        assert_eq!(frame.payload[1],1);
    }
    #[test]
    fn unsupported_jpeg_fails_without_encoding() {
        let mut p=NativeVideoPipeline::new(mock_session(),64,64,30,800).unwrap();
        assert!(p.push_frame(&VideoFrame {
            timestamp_us:0,sequence:0,width:64,height:64,row_stride_bytes:0,
            format:PixelFormat::Mjpeg,bytes:vec![0xff,0xd8],
        }).is_err());
    }
}
