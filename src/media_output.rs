//! Pre-encoded H.264/AVC + AAC frames sent over a confirmed RTMP publish session.
//!
//! The AVC/AAC "FLV tag bodies" are carried as RTMP video (type 9) and audio
//! (type 8) messages. No FLV file header or PreviousTagSize fields go on RTMP.
//! Inputs must already be encoded, with AVC NALs free of Annex-B start codes
//! and AAC frames free of ADTS headers.
use crate::session::{PublishSession, State};
use std::io::{self, Read, Write};

const MAX_MESSAGE: usize = 1024 * 1024;

pub struct EncodedPublisher<T: Read + Write> {
    session: PublishSession<T>,
    video_configured: bool,
    audio_configured: bool,
    last_video_dts: Option<u32>,
    last_audio_dts: Option<u32>,
}

impl<T: Read + Write> EncodedPublisher<T> {
    /// The session must already have received NetStream.Publish.Start.
    pub fn new(session: PublishSession<T>) -> io::Result<Self> {
        if !matches!(session.state(), State::Publishing(_)) {
            return Err(invalid("RTMP session has not confirmed publishing"));
        }
        Ok(Self {
            session,
            video_configured: false,
            audio_configured: false,
            last_video_dts: None,
            last_audio_dts: None,
        })
    }

    pub fn into_session(self) -> PublishSession<T> { self.session }

    /// Send AVCDecoderConfigurationRecord (one SPS, one PPS).
    pub fn avc_configuration(&mut self, dts_ms: u32, sps: &[u8], pps: &[u8]) -> io::Result<()> {
        if sps.len() < 4 || pps.is_empty() ||
            sps.len() > u16::MAX as usize || pps.len() > u16::MAX as usize ||
            sps[0] & 0x1f != 7 || pps[0] & 0x1f != 8 {
            return Err(invalid("expected valid SPS(type 7) and PPS(type 8)"));
        }
        self.check_video_timestamp(dts_ms)?;
        let mut payload = Vec::with_capacity(18 + sps.len() + pps.len());
        payload.extend_from_slice(&[0x17, 0, 0, 0, 0]); // AVC sequence header
        payload.extend_from_slice(&[1, sps[1], sps[2], sps[3], 0xff, 0xe1]);
        payload.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        payload.extend_from_slice(sps);
        payload.push(1); // PPS count
        payload.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        payload.extend_from_slice(pps);
        self.session.send_media(9, dts_ms, &payload)?;
        self.video_configured = true;
        self.last_video_dts = Some(dts_ms);
        Ok(())
    }

    /// Encode AVC access unit, using length-prefixed NAL units.
    /// cts_ms is PTS - DTS and must fit in signed 24 bits.
    pub fn avc_frame(&mut self, dts_ms: u32, cts_ms: i32, keyframe: bool, nals: &[&[u8]]) -> io::Result<()> {
        if !self.video_configured { return Err(invalid("AVC configuration required before video frames")); }
        self.check_video_timestamp(dts_ms)?;
        if !(-8_388_608..=8_388_607).contains(&cts_ms) || nals.is_empty() {
            return Err(invalid("invalid AVC access unit or composition offset"));
        }
        let mut payload = Vec::with_capacity(64);
        payload.push(if keyframe { 0x17 } else { 0x27 });
        payload.push(1); // AVC NALU packet
        payload.extend_from_slice(&cts_ms.to_be_bytes()[1..]);
        for nal in nals {
            if nal.is_empty() || nal.starts_with(&[0, 0, 1]) || nal.starts_with(&[0, 0, 0, 1]) ||
                nal.len() > u32::MAX as usize ||
                payload.len().saturating_add(nal.len()).saturating_add(4) > MAX_MESSAGE {
                return Err(invalid("invalid or oversized AVC NAL unit"));
            }
            payload.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            payload.extend_from_slice(nal);
        }
        self.session.send_media(9, dts_ms, &payload)?;
        self.last_video_dts = Some(dts_ms);
        Ok(())
    }

    /// AAC sequence header. ASC is usually 2 bytes at 48kHz stereo.
    pub fn aac_configuration(&mut self, timestamp_ms: u32, asc: &[u8]) -> io::Result<()> {
        if !(2..=16).contains(&asc.len()) { return Err(invalid("invalid AAC AudioSpecificConfig")); }
        self.check_audio_timestamp(timestamp_ms)?;
        let mut payload = Vec::with_capacity(2 + asc.len());
        payload.extend_from_slice(&[0xaf, 0]); // AAC / 44kHz container signaling / stereo, sequence header
        payload.extend_from_slice(asc);
        self.session.send_media(8, timestamp_ms, &payload)?;
        self.audio_configured = true;
        self.last_audio_dts = Some(timestamp_ms);
        Ok(())
    }

    /// Send a raw AAC access unit (no ADTS header).
    pub fn aac_frame(&mut self, timestamp_ms: u32, raw: &[u8]) -> io::Result<()> {
        if !self.audio_configured { return Err(invalid("AAC configuration required before audio frames")); }
        self.check_audio_timestamp(timestamp_ms)?;
        if raw.is_empty() || raw.len() > MAX_MESSAGE - 2 ||
            (raw.len() >= 2 && raw[0] == 0xff && (raw[1] & 0xf0) == 0xf0) {
            return Err(invalid("empty, oversized or ADTS-wrapped AAC frame"));
        }
        let mut payload = Vec::with_capacity(raw.len() + 2);
        payload.extend_from_slice(&[0xaf, 1]);
        payload.extend_from_slice(raw);
        self.session.send_media(8, timestamp_ms, &payload)?;
        self.last_audio_dts = Some(timestamp_ms);
        Ok(())
    }

    fn check_video_timestamp(&self, timestamp: u32) -> io::Result<()> {
        if self.last_video_dts.is_some_and(|prev| timestamp < prev) {
            return Err(invalid("video decode timestamps regressed"));
        }
        Ok(())
    }

    fn check_audio_timestamp(&self, timestamp: u32) -> io::Result<()> {
        if self.last_audio_dts.is_some_and(|prev| timestamp < prev) {
            return Err(invalid("audio timestamps regressed"));
        }
        Ok(())
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

    struct MockTransport {
        incoming: Cursor<Vec<u8>>,
        written: Vec<u8>,
        fail_writes: bool,
    }
    impl Read for MockTransport {
        fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> { self.incoming.read(dst) }
    }
    impl Write for MockTransport {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail_writes { return Err(io::Error::new(io::ErrorKind::BrokenPipe, "simulated disconnect")); }
            self.written.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    fn status_message(code: &str) -> Vec<u8> {
        let mut payload = Vec::new();
        amf0::encode(&mut payload, amf0::Value::String("onStatus")).unwrap();
        amf0::encode(&mut payload, amf0::Value::Number(0.0)).unwrap();
        amf0::encode(&mut payload, amf0::Value::Null).unwrap();
        amf0::encode_object(&mut payload, &[("code", amf0::Value::String(code))]).unwrap();
        payload
    }
    fn result_message(txn: f64, value: amf0::Value<'_>) -> Vec<u8> {
        let mut payload=Vec::new();
        amf0::encode(&mut payload, amf0::Value::String("_result")).unwrap();
        amf0::encode(&mut payload, amf0::Value::Number(txn)).unwrap();
        amf0::encode(&mut payload, amf0::Value::Null).unwrap();
        amf0::encode(&mut payload, value).unwrap();
        payload
    }
    fn mock_session() -> PublishSession<MockTransport> {
        let mut writer=RtmpChunkWriter::new(Vec::new(), 128).unwrap();
        writer.message(3,20,0,0,&result_message(1.0,amf0::Value::Null)).unwrap();
        writer.message(3,20,0,0,&result_message(2.0,amf0::Value::Number(1.0))).unwrap();
        writer.message(3,20,1,0,&status_message("NetStream.Publish.Start")).unwrap();
        let io=MockTransport { incoming: Cursor::new(writer.into_inner()), written: Vec::new(), fail_writes: false };
        let mut session=PublishSession::new(io);
        session.connect("live2", "rtmps://example.test/live2").unwrap();
        session.create_stream().unwrap();
        session.publish("private-test-key").unwrap();
        session
    }

    #[test]
    fn sends_valid_avc_aac_messages_and_preserves_timestamps() {
        let mut pubr=EncodedPublisher::new(mock_session()).unwrap();
        pubr.avc_configuration(0,&[0x67,0x42,0,0x1f,0xe5], &[0x68,0xce,0x06,0xe2]).unwrap();
        pubr.aac_configuration(0,&[0x11,0x90]).unwrap();
        pubr.aac_frame(21,&[0x12,0x34,0x56]).unwrap();
        pubr.avc_frame(33, -1, true, &[&[0x65,0xaa,0xbb]]).unwrap();
        pubr.avc_frame(66, 0, false, &[&[0x41,0xcc]]).unwrap();

        let io=pubr.into_session().into_inner();
        // Skip the three command messages before checking media.
        let mut stream=io.written.as_slice();
        let mut reader=ChunkReader::new();
        for _ in 0..3 { assert_eq!(reader.read_message(&mut stream).unwrap().type_id,20); }
        let avc_config=reader.read_message(&mut stream).unwrap();
        assert_eq!((avc_config.type_id, avc_config.stream_id, avc_config.timestamp_ms),(9,1,0));
        assert_eq!(&avc_config.payload[..5],&[0x17,0,0,0,0]);
        let aac_config=reader.read_message(&mut stream).unwrap();
        assert_eq!((aac_config.type_id,aac_config.timestamp_ms),(8,0));
        assert_eq!(aac_config.payload,&[0xaf,0,0x11,0x90]);
        let aac_frame=reader.read_message(&mut stream).unwrap();
        assert_eq!((aac_frame.type_id,aac_frame.timestamp_ms),(8,21));
        assert_eq!(aac_frame.payload,&[0xaf,1,0x12,0x34,0x56]);
        let key=reader.read_message(&mut stream).unwrap();
        assert_eq!((key.type_id,key.timestamp_ms),(9,33));
        assert_eq!(&key.payload[..9], &[0x17,1,0xff,0xff,0xff,0,0,0,3]);
        let next=reader.read_message(&mut stream).unwrap();
        assert_eq!((next.type_id,next.timestamp_ms),(9,66));
        assert_eq!(next.payload[0],0x27);
        assert!(stream.is_empty());
    }

    #[test]
    fn refuses_invalid_encoding_and_reordered_timestamps() {
        let mut p=EncodedPublisher::new(mock_session()).unwrap();
        assert!(p.avc_frame(0,0,true,&[&[0x65,0]]).is_err());
        assert!(p.aac_frame(0,&[0x01]).is_err());
        assert!(p.avc_configuration(0,&[0x65,0,0,0],&[0x68,1]).is_err());
        p.avc_configuration(0,&[0x67,0x42,0,0x1f],&[0x68,1]).unwrap();
        p.aac_configuration(0,&[0x11,0x90]).unwrap();
        p.avc_frame(33,0,true,&[&[0x65,0]]).unwrap();
        assert!(p.avc_frame(32,0,false,&[&[0x41,0]]).is_err());
        assert!(p.avc_frame(33,0,false,&[&[0,0,0,1,0x41]]).is_err());
        assert!(p.avc_frame(34,8_388_608,false,&[&[0x41]]).is_err());
        p.aac_frame(21,&[0x12,0x34]).unwrap();
        assert!(p.aac_frame(20,&[1]).is_err());
        assert!(p.aac_frame(22,&[0xff,0xf1,0x00]).is_err());
    }

    #[test]
    fn write_failure_is_returned() {
        let mut session=mock_session();
        session.transport_mut_for_test().fail_writes=true;
        let mut p=EncodedPublisher::new(session).unwrap();
        let err=p.aac_configuration(0,&[0x11,0x90]).unwrap_err();
        assert_eq!(err.kind(),io::ErrorKind::BrokenPipe);
    }
}
