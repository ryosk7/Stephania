//! RTMP publish negotiation on an already connected, optionally TLS-protected,
//! Read+Write stream. The caller owns TLS certificate verification and deadlines.
//!
//! This module does not send encoded media or open a network connection.
use crate::{amf0, amf0_decode::{self, Value}, rtmp::RtmpChunkWriter, rtmp_read::ChunkReader};
use std::io::{self, Read, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    New,
    Connected,
    StreamCreated(u32),
    Publishing(u32),
}

pub struct PublishSession<T> {
    transport: T,
    incoming: ChunkReader,
    state: State,
    media_buffer: Vec<u8>,
}

impl<T: Read + Write> PublishSession<T> {
    pub fn new(transport: T) -> Self {
        Self { transport, incoming: ChunkReader::new(), state: State::New, media_buffer: Vec::new() }
    }

    pub fn state(&self) -> State { self.state }
    pub fn into_inner(self) -> T { self.transport }

    /// Send already packetized AVC (9), AAC (8), or metadata (18) FLV tag bodies.
    /// Only permitted after the server confirms NetStream.Publish.Start.
    /// This does not include the FLV file header or PreviousTagSize.
    pub fn send_media(&mut self, type_id: u8, timestamp_ms: u32, payload: &[u8]) -> io::Result<()> {
        let State::Publishing(stream_id) = self.state else {
            return Err(invalid("media output requires confirmed RTMP publication"));
        };
        let chunk_stream_id = match type_id {
            8 => 4,
            9 => 6,
            18 => 5,
            _ => return Err(invalid("unsupported media message type")),
        };
        if payload.is_empty() || payload.len() > 1024 * 1024 {
            return Err(invalid("invalid media message length"));
        }
        // Preserve RTMP framing, but avoid thousands of tiny TLS writes.
        // Reuse bounded scratch; no cross-message buffering or retry on failure.
        self.media_buffer.clear();
        let mut writer = RtmpChunkWriter::new(&mut self.media_buffer, 128)?;
        writer.message(chunk_stream_id, type_id, stream_id, timestamp_ms, payload)?;
        self.transport.write_all(&self.media_buffer)?;
        self.transport.flush()
    }

    #[cfg(test)]
    pub(crate) fn transport_mut_for_test(&mut self) -> &mut T {
        &mut self.transport
    }

    fn send_command(&mut self, stream_id: u32, command: &[u8]) -> io::Result<()> {
        let mut writer = RtmpChunkWriter::new(&mut self.transport, 128)?;
        writer.message(3, 20, stream_id, 0, command)?;
        self.transport.flush()
    }

    fn next_command(&mut self) -> io::Result<Vec<u8>> {
        // Ignore protocol control messages and await the relevant AMF0 reply.
        // This is bounded even if a peer floods unrelated messages.
        for _ in 0..64 {
            let msg = self.incoming.read_message(&mut self.transport)?;
            if msg.type_id == 20 && msg.stream_id <= 1 {
                return Ok(msg.payload);
            }
            if msg.type_id == 17 {
                return Err(invalid("AMF3 RTMP commands are not supported"));
            }
        }
        Err(invalid("too many unrelated RTMP messages"))
    }

    fn transaction_result(&mut self, transaction: f64) -> io::Result<Vec<Value<'static>>> {
        // Values borrowed from response cannot outlive its buffer. Convert only
        // the exact data required for matching into an owned representation.
        let payload = self.next_command()?;
        let vals = amf0_decode::decode_all(&payload)?;
        let name = match vals.first() { Some(Value::String(name)) => *name, _ => return Err(invalid("missing RTMP result name")) };
        let txn = match vals.get(1) { Some(Value::Number(n)) => *n, _ => return Err(invalid("missing RTMP result transaction")) };
        if txn != transaction { return Err(invalid("unexpected RTMP transaction id")); }
        if name == "_error" { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "RTMP server rejected command")); }
        if name != "_result" { return Err(invalid("expected RTMP _result")); }
        // Re-parse into a minimal owned value projection: only stream number
        // is used by createStream; the original response can be dropped here.
        let last = match vals.last() {
            Some(Value::Number(n)) if n.is_finite() && *n >= 1.0 && *n <= u32::MAX as f64 && n.fract() == 0.0 => Value::Number(*n),
            _ => Value::Null,
        };
        Ok(vec![last])
    }

    pub fn connect(&mut self, app: &str, tc_url: &str) -> io::Result<()> {
        if self.state != State::New { return Err(invalid("connect requires new session")); }
        let payload = amf0::connect_command(app, tc_url)?;
        self.send_command(0, &payload)?;
        self.transaction_result(1.0)?;
        self.state = State::Connected;
        Ok(())
    }

    pub fn create_stream(&mut self) -> io::Result<u32> {
        if self.state != State::Connected { return Err(invalid("createStream requires connection")); }
        self.send_command(0, &amf0::create_stream_command(2.0)?)?;
        let result = self.transaction_result(2.0)?;
        let Some(Value::Number(n)) = result.first() else {
            return Err(invalid("missing createStream stream ID"));
        };
        let id = *n as u32;
        self.state = State::StreamCreated(id);
        Ok(id)
    }

    pub fn publish(&mut self, name: &str) -> io::Result<()> {
        let State::StreamCreated(id) = self.state else {
            return Err(invalid("publish requires a created stream"));
        };
        if name.is_empty() || name.len() > 256 { return Err(invalid("invalid stream name")); }
        self.send_command(id, &amf0::publish_command(name)?)?;
        for _ in 0..64 {
            let payload = self.next_command()?;
            let values = amf0_decode::decode_all(&payload)?;
            let event = match values.first() { Some(Value::String(s)) => *s, _ => continue };
            if event == "_error" {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "RTMP publish rejected"));
            }
            if event != "onStatus" { continue; }
            match amf0_decode::status_code(&values) {
                Some("NetStream.Publish.Start") => {
                    self.state = State::Publishing(id);
                    return Ok(());
                }
                Some(code) if code.starts_with("NetStream.Publish.") || code.ends_with(".Failed") || code.ends_with(".Rejected") =>
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, "RTMP server rejected publish")),
                _ => {}
            }
        }
        Err(invalid("RTMP server did not confirm publication"))
    }
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Duplex { read: Cursor<Vec<u8>>, written: Vec<u8> }
    impl Read for Duplex {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> { self.read.read(out) }
    }
    impl Write for Duplex {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(data); Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    fn server_result(transaction: f64, value: amf0::Value<'_>) -> Vec<u8> {
        let mut payload = Vec::new();
        amf0::encode(&mut payload, amf0::Value::String("_result")).unwrap();
        amf0::encode(&mut payload, amf0::Value::Number(transaction)).unwrap();
        amf0::encode(&mut payload, amf0::Value::Null).unwrap();
        amf0::encode(&mut payload, value).unwrap();
        payload
    }
    fn server_status(code: &str) -> Vec<u8> {
        let mut payload = Vec::new();
        amf0::encode(&mut payload, amf0::Value::String("onStatus")).unwrap();
        amf0::encode(&mut payload, amf0::Value::Number(0.0)).unwrap();
        amf0::encode(&mut payload, amf0::Value::Null).unwrap();
        amf0::encode_object(&mut payload, &[("code", amf0::Value::String(code))]).unwrap();
        payload
    }
    fn transcript(reject: bool) -> Duplex {
        let mut out = RtmpChunkWriter::new(Vec::new(), 128).unwrap();
        out.message(3,20,0,0,&server_result(1.0,amf0::Value::Null)).unwrap();
        out.message(3,20,0,0,&server_result(2.0,amf0::Value::Number(1.0))).unwrap();
        let status = server_status(if reject { "NetStream.Publish.BadName" } else { "NetStream.Publish.Start" });
        out.message(3,20,1,0,&status).unwrap();
        Duplex { read: Cursor::new(out.into_inner()), written: vec![] }
    }

    #[test]
    fn large_media_write_is_batched_and_wire_identical() {
        struct Counted { inner: Duplex, writes: usize, flushes: usize }
        impl Read for Counted {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> { self.inner.read(b) }
        }
        impl Write for Counted {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.inner.write(b)
            }
            fn flush(&mut self) -> io::Result<()> { self.flushes += 1; Ok(()) }
        }
        let mut session = PublishSession::new(Counted { inner: transcript(false), writes: 0, flushes: 0 });
        session.connect("live2", "rtmps://example.test/live2").unwrap();
        session.create_stream().unwrap();
        session.publish("test").unwrap();
        let payload = vec![0x55; 735_000];
        for ts in [100, 0x0100_0000] {
            let before = session.transport.inner.written.len();
            let writes = session.transport.writes;
            let flushes = session.transport.flushes;
            let mut expected = RtmpChunkWriter::new(Vec::new(), 128).unwrap();
            expected.message(6, 9, 1, ts, &payload).unwrap();
            session.send_media(9, ts, &payload).unwrap();
            assert_eq!(&session.transport.inner.written[before..], expected.into_inner().as_slice());
            assert_eq!(session.transport.writes - writes, 1);
            assert_eq!(session.transport.flushes - flushes, 1);
        }
    }
    #[test]
    fn mock_server_accepts_connect_create_and_publish() {
        let mut session = PublishSession::new(transcript(false));
        session.connect("live2","rtmps://example.test/live2").unwrap();
        assert_eq!(session.state(),State::Connected);
        assert_eq!(session.create_stream().unwrap(),1);
        assert_eq!(session.state(),State::StreamCreated(1));
        session.publish("stream-key-placeholder").unwrap();
        assert_eq!(session.state(),State::Publishing(1));
        assert!(!session.into_inner().written.is_empty());
    }
    #[test]
    fn mock_server_publish_rejection_does_not_advance_state() {
        let mut session = PublishSession::new(transcript(true));
        session.connect("live2","rtmps://example.test/live2").unwrap();
        session.create_stream().unwrap();
        assert_eq!(session.publish("test").unwrap_err().kind(),io::ErrorKind::PermissionDenied);
        assert_eq!(session.state(),State::StreamCreated(1));
    }
    #[test]
    fn unexpected_order_and_truncated_server_are_errors() {
        let mut session = PublishSession::new(transcript(false));
        assert!(session.create_stream().is_err());
        assert!(session.publish("test").is_err());
        let mut empty = PublishSession::new(Duplex { read: Cursor::new(vec![]), written: vec![] });
        assert_eq!(empty.connect("a","rtmps://example.test/a").unwrap_err().kind(),io::ErrorKind::UnexpectedEof);
    }
}
