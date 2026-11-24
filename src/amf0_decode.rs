//! Bounded AMF0 server-response decoder.
//!
//! Supports the primitive and object types needed to understand RTMP
//! connect/createStream/onStatus responses. Explicit depth, string and field
//! limits prevent untrusted RTMP peers from exhausting memory.
use std::io::{self, ErrorKind};

const MAX_DEPTH: usize = 12;
const MAX_FIELDS: usize = 256;
const MAX_STRING: usize = 65_536;

#[derive(Clone, Debug, PartialEq)]
pub enum Value<'a> {
    Number(f64),
    Boolean(bool),
    String(&'a str),
    Null,
    Undefined,
    Object(Vec<(&'a str, Value<'a>)>),
    StrictArray(Vec<Value<'a>>),
}

pub struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Decoder<'a> {
    pub fn new(data: &'a [u8]) -> Self { Self { data, pos: 0 } }

    fn take(&mut self, len: usize) -> io::Result<&'a [u8]> {
        let end = self.pos.checked_add(len).ok_or_else(bad_data)?;
        let bytes = self.data.get(self.pos..end).ok_or_else(bad_data)?;
        self.pos = end;
        Ok(bytes)
    }
    fn u16(&mut self) -> io::Result<usize> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]) as usize)
    }
    fn u32(&mut self) -> io::Result<usize> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
    }
    fn text(&mut self, len: usize) -> io::Result<&'a str> {
        if len > MAX_STRING { return Err(bad_data()); }
        std::str::from_utf8(self.take(len)?).map_err(|_| bad_data())
    }
    fn short_string(&mut self) -> io::Result<&'a str> {
        let len = self.u16()?;
        self.text(len)
    }
    fn object(&mut self, depth: usize) -> io::Result<Value<'a>> {
        let mut fields = Vec::new();
        loop {
            if fields.len() >= MAX_FIELDS { return Err(bad_data()); }
            let len = self.u16()?;
            // The AMF0 object end marker is 0x00 0x00 0x09.
            if len == 0 && self.data.get(self.pos) == Some(&9) {
                self.pos += 1;
                return Ok(Value::Object(fields));
            }
            let name = self.text(len)?;
            let value = self.value_with_depth(depth + 1)?;
            fields.push((name, value));
        }
    }
    fn value_with_depth(&mut self, depth: usize) -> io::Result<Value<'a>> {
        if depth >= MAX_DEPTH { return Err(bad_data()); }
        let kind = self.take(1)?[0];
        match kind {
            0x00 => {
                let bits: [u8;8] = self.take(8)?.try_into().map_err(|_|bad_data())?;
                let n = f64::from_be_bytes(bits);
                if !n.is_finite() { return Err(bad_data()); }
                Ok(Value::Number(n))
            }
            0x01 => {
                let b = self.take(1)?[0];
                if b>1 { return Err(bad_data()); }
                Ok(Value::Boolean(b==1))
            }
            0x02 => Ok(Value::String(self.short_string()?)),
            0x03 => self.object(depth),
            0x05 => Ok(Value::Null),
            0x06 => Ok(Value::Undefined),
            0x08 => {
                let count = self.u32()?;
                if count > MAX_FIELDS { return Err(bad_data()); }
                let out = self.object(depth)?;
                if let Value::Object(ref fields) = out {
                    // ECMA array counts are advisory in real RTMP servers.
                    if fields.len() > MAX_FIELDS { return Err(bad_data()); }
                }
                Ok(out)
            }
            0x0a => {
                let count = self.u32()?;
                if count > MAX_FIELDS { return Err(bad_data()); }
                let mut values = Vec::with_capacity(count);
                for _ in 0..count { values.push(self.value_with_depth(depth + 1)?); }
                Ok(Value::StrictArray(values))
            }
            0x0c => {
                let len = self.u32()?;
                Ok(Value::String(self.text(len)?))
            }
            _ => Err(bad_data()),
        }
    }
    pub fn value(&mut self) -> io::Result<Value<'a>> {
        self.value_with_depth(0)
    }
    pub fn at_end(&self) -> bool { self.pos == self.data.len() }
}

fn bad_data() -> io::Error {
    io::Error::new(ErrorKind::InvalidData, "malformed or unsupported AMF0 response")
}

pub fn decode_all(payload: &[u8]) -> io::Result<Vec<Value<'_>>> {
    if payload.len() > 1024 * 1024 { return Err(bad_data()); }
    let mut d = Decoder::new(payload);
    let mut vals = Vec::new();
    while !d.at_end() {
        if vals.len() >= 32 { return Err(bad_data()); }
        vals.push(d.value()?);
    }
    Ok(vals)
}

/// Server event code, e.g. NetStream.Publish.Start, if found at top-level object.
/// Never interpret a missing event as confirmation of successful publication.
pub fn status_code<'a>(values: &'a [Value<'a>]) -> Option<&'a str> {
    for value in values {
        if let Value::Object(fields) = value {
            for (key, val) in fields {
                if *key == "code" {
                    if let Value::String(code) = val { return Some(code); }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strval(s: &str) -> Vec<u8> {
        let mut v = vec![2];
        v.extend_from_slice(&(s.len() as u16).to_be_bytes());
        v.extend_from_slice(s.as_bytes());
        v
    }
    #[test]
    fn decodes_connect_response_and_status_code() {
        let mut p = strval("onStatus");
        p.extend_from_slice(&[0, 0,0,0,0,0,0,0,0, 5]);
        p.push(3);
        p.extend_from_slice(&[0,4]);
        p.extend_from_slice(b"code");
        p.extend(strval("NetStream.Publish.Start"));
        p.extend_from_slice(&[0,0,9]);
        let values = decode_all(&p).unwrap();
        assert_eq!(values[0], Value::String("onStatus"));
        assert_eq!(status_code(&values), Some("NetStream.Publish.Start"));
    }
    #[test]
    fn rejects_partial_and_invalid_utf8() {
        assert!(decode_all(&[0,1,2]).is_err());
        assert!(decode_all(&[2,0,1,0xff]).is_err());
        assert!(decode_all(&[3,0,0,9]).is_ok());
        assert!(decode_all(&[1,4]).is_err());
    }
    #[test]
    fn decodes_number_from_create_stream() {
        let mut d = Decoder::new(&[0,0x40,0x08,0,0,0,0,0,0]);
        assert_eq!(d.value().unwrap(),Value::Number(3.0));
    }
    #[test]
    fn detects_truncated_object_and_oversized_array() {
        assert!(decode_all(&[3,0,1,b'a',5]).is_err());
        assert!(decode_all(&[10,0,0,1,1]).is_err());
    }
}
