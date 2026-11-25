//! Relay pre-encoded AVC/AAC FLV tags to a confirmed RTMP publish session.
//! Does not encode media. The input may be a file or live FFmpeg FLV pipe.
use crate::{flv_input::FlvReader, session::PublishSession};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

const MAX_GAP_MS: u32 = 30_000;

pub fn relay<R: Read, T: Read + Write>(
    input: &mut FlvReader<R>,
    session: &mut PublishSession<T>,
    realtime: bool,
) -> io::Result<u64> {
    let start = Instant::now();
    let mut first_ts = None;
    let mut last_ts = None;
    let mut audio_configured = false;
    let mut video_configured = false;
    let mut count = 0u64;
    while let Some(tag) = input.next_tag()? {
        if let Some(prev) = last_ts {
            if tag.timestamp_ms < prev {
                return Err(invalid("FLV timestamps regress"));
            }
            if tag.timestamp_ms - prev > MAX_GAP_MS {
                return Err(invalid("FLV timestamp gap exceeds 30 seconds"));
            }
        }
        let start_ts = *first_ts.get_or_insert(tag.timestamp_ms);
        let elapsed_ms = tag.timestamp_ms - start_ts;
        match tag.kind {
            8 => {
                if tag.payload.len() < 3 || tag.payload[0] != 0xaf {
                    return Err(invalid("expected stereo AAC FLV body"));
                }
                match tag.payload[1] {
                    0 => audio_configured = true,
                    1 if audio_configured => {}
                    _ => return Err(invalid("AAC sequence header missing")),
                }
            }
            9 => {
                if tag.payload.len() < 5 || tag.payload[0] & 0x0f != 7 {
                    return Err(invalid("expected AVC FLV body"));
                }
                match tag.payload[1] {
                    0 => video_configured = true,
                    1 | 2 if video_configured => {}
                    _ => return Err(invalid("AVC sequence header missing")),
                }
            }
            18 => {}
            _ => return Err(invalid("unsupported FLV payload")),
        }
        if realtime {
            let deadline = Duration::from_millis(u64::from(elapsed_ms));
            if let Some(remaining) = deadline.checked_sub(start.elapsed()) {
                std::thread::sleep(remaining);
            }
        }
        session.send_media(tag.kind, tag.timestamp_ms, &tag.payload)?;
        last_ts = Some(tag.timestamp_ms);
        count += 1;
    }
    Ok(count)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{flv::FlvMuxer, rtmp::RtmpChunkWriter, amf0, rtmp_read::ChunkReader};
    use std::io::Cursor;

    struct Mock {
        incoming: Cursor<Vec<u8>>,
        sent: Vec<u8>,
        fail: bool,
    }
    impl Read for Mock {
        fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> { self.incoming.read(dst) }
    }
    impl Write for Mock {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            if self.fail { return Err(io::Error::new(io::ErrorKind::BrokenPipe, "disconnect")); }
            self.sent.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    fn server_result(txn: f64, val: amf0::Value<'_>) -> Vec<u8> {
        let mut data=Vec::new();
        amf0::encode(&mut data,amf0::Value::String("_result")).unwrap();
        amf0::encode(&mut data,amf0::Value::Number(txn)).unwrap();
        amf0::encode(&mut data,amf0::Value::Null).unwrap();
        amf0::encode(&mut data,val).unwrap();
        data
    }
    fn server_status() -> Vec<u8> {
        let mut data=Vec::new();
        amf0::encode(&mut data,amf0::Value::String("onStatus")).unwrap();
        amf0::encode(&mut data,amf0::Value::Number(0.0)).unwrap();
        amf0::encode(&mut data,amf0::Value::Null).unwrap();
        amf0::encode_object(&mut data,&[("code",amf0::Value::String("NetStream.Publish.Start"))]).unwrap();
        data
    }
    fn session() -> PublishSession<Mock> {
        let mut writer=RtmpChunkWriter::new(Vec::new(),128).unwrap();
        writer.message(3,20,0,0,&server_result(1.0,amf0::Value::Null)).unwrap();
        writer.message(3,20,0,0,&server_result(2.0,amf0::Value::Number(1.0))).unwrap();
        writer.message(3,20,1,0,&server_status()).unwrap();
        let mock=Mock { incoming:Cursor::new(writer.into_inner()),sent:Vec::new(),fail:false };
        let mut s=PublishSession::new(mock);
        s.connect("live2","rtmps://example.test/live2").unwrap();
        s.create_stream().unwrap();
        s.publish("test-key").unwrap();
        s
    }

    #[test]
    fn relays_muxed_h264_aac_without_file_framing() {
        let mut mux=FlvMuxer::new(Vec::new()).unwrap();
        mux.avc_configuration(0,&[0x67,0x42,0,0x1f],&[0x68,1]).unwrap();
        mux.aac_configuration(0,&[0x11,0x90]).unwrap();
        mux.aac_frame(21,&[1,2,3]).unwrap();
        mux.avc_frame(33,0,true,&[&[0x65,0xaa]]).unwrap();
        let flv=mux.into_inner();
        let mut input=FlvReader::new(flv.as_slice()).unwrap();
        let mut session=session();
        assert_eq!(relay(&mut input,&mut session,false).unwrap(),4);
        let output=session.into_inner().sent;
        let mut r=ChunkReader::new();
        let mut source=output.as_slice();
        for _ in 0..3 { assert_eq!(r.read_message(&mut source).unwrap().type_id,20); }
        let video=r.read_message(&mut source).unwrap();
        assert_eq!((video.type_id,video.timestamp_ms),(9,0));
        assert_eq!(&video.payload[..5],&[0x17,0,0,0,0]);
        let audio=r.read_message(&mut source).unwrap();
        assert_eq!((audio.type_id,audio.timestamp_ms),(8,0));
        assert_eq!(audio.payload,&[0xaf,0,0x11,0x90]);
        let frame=r.read_message(&mut source).unwrap();
        assert_eq!((frame.type_id,frame.timestamp_ms),(8,21));
        let frame=r.read_message(&mut source).unwrap();
        assert_eq!((frame.type_id,frame.timestamp_ms),(9,33));
        assert!(source.is_empty());
    }

    #[test]
    fn surfaces_transport_disconnection() {
        let mut mux=FlvMuxer::new(Vec::new()).unwrap();
        mux.aac_configuration(0,&[0x11,0x90]).unwrap();
        let bytes=mux.into_inner();
        let mut source=FlvReader::new(bytes.as_slice()).unwrap();
        let mut s=session();
        s.transport_mut_for_test().fail=true;
        assert_eq!(relay(&mut source,&mut s,false).unwrap_err().kind(),io::ErrorKind::BrokenPipe);
    }
}
