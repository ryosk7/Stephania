//! AMF0 command payload serializer for RTMP publish negotiation.
//! This module generates commands but does not parse server responses.
use std::io::{self, Write};

#[derive(Clone, Copy)]
pub enum Value<'a> {
    Number(f64),
    Boolean(bool),
    String(&'a str),
    Null,
}

fn short_string(out: &mut impl Write, s: &str) -> io::Result<()> {
    let bytes = s.as_bytes();
    if bytes.len() > u16::MAX as usize {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "AMF0 short string overflow"));
    }
    out.write_all(&(bytes.len() as u16).to_be_bytes())?;
    out.write_all(bytes)
}

pub fn encode(out: &mut impl Write, value: Value<'_>) -> io::Result<()> {
    match value {
        Value::Number(n) if n.is_finite() => {
            out.write_all(&[0x00])?;
            out.write_all(&n.to_be_bytes())
        }
        Value::Number(_) => Err(io::Error::new(io::ErrorKind::InvalidInput, "non-finite AMF0 number")),
        Value::Boolean(b) => out.write_all(&[0x01, u8::from(b)]),
        Value::String(s) => {
            out.write_all(&[0x02])?;
            short_string(out, s)
        }
        Value::Null => out.write_all(&[0x05]),
    }
}

/// AMF0 key/value object limited to primitive values.
pub fn encode_object(out: &mut impl Write, fields: &[(&str, Value<'_>)]) -> io::Result<()> {
    out.write_all(&[0x03])?;
    for &(name, value) in fields {
        short_string(out, name)?;
        encode(out, value)?;
    }
    out.write_all(&[0, 0, 9])
}

pub fn connect_command(app: &str, tc_url: &str) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    encode(&mut data, Value::String("connect"))?;
    encode(&mut data, Value::Number(1.0))?;
    encode_object(&mut data, &[
        ("app", Value::String(app)),
        ("type", Value::String("nonprivate")),
        ("tcUrl", Value::String(tc_url)),
        ("fpad", Value::Boolean(false)),
        ("capabilities", Value::Number(15.0)),
        ("audioCodecs", Value::Number(4071.0)),
        ("videoCodecs", Value::Number(252.0)),
        ("videoFunction", Value::Number(1.0)),
        ("objectEncoding", Value::Number(0.0)),
    ])?;
    Ok(data)
}

pub fn create_stream_command(transaction: f64) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    encode(&mut data, Value::String("createStream"))?;
    encode(&mut data, Value::Number(transaction))?;
    encode(&mut data, Value::Null)?;
    Ok(data)
}

pub fn publish_command(stream_name: &str) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    encode(&mut data, Value::String("publish"))?;
    encode(&mut data, Value::Number(0.0))?;
    encode(&mut data, Value::Null)?;
    encode(&mut data, Value::String(stream_name))?;
    encode(&mut data, Value::String("live"))?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn primitive_types_are_network_ordered() {
        let mut data=Vec::new();
        encode(&mut data, Value::String("go")).unwrap();
        encode(&mut data, Value::Number(1.0)).unwrap();
        encode(&mut data, Value::Boolean(true)).unwrap();
        encode(&mut data, Value::Null).unwrap();
        assert_eq!(&data[..5], &[2,0,2,b'g',b'o']);
        assert_eq!(&data[5..14], &[0,0x3f,0xf0,0,0,0,0,0,0]);
        assert_eq!(&data[14..], &[1,1,5]);
    }
    #[test]
    fn object_terminator_is_emitted() {
        let mut data=Vec::new();
        encode_object(&mut data, &[("app",Value::String("live2"))]).unwrap();
        assert!(data.starts_with(&[3,0,3,b'a',b'p',b'p']));
        assert!(data.ends_with(&[0,0,9]));
    }
    #[test]
    fn commands_have_expected_amf_names() {
        let connect=connect_command("live2","rtmps://example.test/live2").unwrap();
        assert!(connect.windows(7).any(|x|x==b"connect"));
        assert!(connect.windows(5).any(|x|x==b"live2"));
        let create=create_stream_command(2.0).unwrap();
        assert!(create.windows(12).any(|x|x==b"createStream"));
        let publish=publish_command("PRIVATE_TEST_ONLY").unwrap();
        assert!(publish.windows(7).any(|x|x==b"publish"));
        assert!(publish.ends_with(&[2,0,4,b'l',b'i',b'v',b'e']));
    }
    #[test]
    fn refuses_nan_and_oversized_string() {
        assert!(encode(&mut Vec::new(),Value::Number(f64::NAN)).is_err());
        assert!(encode(&mut Vec::new(),Value::String(&"x".repeat(65536))).is_err());
    }
}
