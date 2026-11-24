//! Minimal AMF0 serialization for RTMP connect/createStream/publish commands.
//! Parsing incoming _result/onStatus and RTMP acknowledgments is a separate task.
use std::io::{self, Write};

pub fn string<W: Write>(out: &mut W, value: &str) -> io::Result<()> {
    let bytes = value.as_bytes();
    let len = u16::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "AMF0 string too long"))?;
    out.write_all(&[2])?;
    out.write_all(&len.to_be_bytes())?;
    out.write_all(bytes)
}

pub fn number<W: Write>(out: &mut W, value: f64) -> io::Result<()> {
    if !value.is_finite() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "nonfinite AMF0 number")); }
    out.write_all(&[0])?;
    out.write_all(&value.to_be_bytes())
}

pub fn null<W: Write>(out: &mut W) -> io::Result<()> { out.write_all(&[5]) }
pub fn boolean<W: Write>(out: &mut W, value: bool) -> io::Result<()> {
    out.write_all(&[1, u8::from(value)])
}

pub fn object<W: Write>(out: &mut W, fields: &[(&str, Value<'_>)]) -> io::Result<()> {
    out.write_all(&[3])?;
    for (key, value) in fields {
        let k=key.as_bytes();
        let len=u16::try_from(k.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "AMF0 key too long"))?;
        out.write_all(&len.to_be_bytes())?;
        out.write_all(k)?;
        value.write(out)?;
    }
    out.write_all(&[0,0,9])
}

pub enum Value<'a> {
    String(&'a str),
    Number(f64),
    Boolean(bool),
}
impl Value<'_> {
    fn write<W: Write>(&self, out: &mut W) -> io::Result<()> {
        match self {
            Self::String(s) => string(out,s),
            Self::Number(n) => number(out,*n),
            Self::Boolean(b) => boolean(out,*b),
        }
    }
}

/// RTMP AMF0 command message (message type 20).
pub fn connect_command(app: &str, tc_url: &str) -> io::Result<Vec<u8>> {
    let mut out=Vec::new();
    string(&mut out,"connect")?;
    number(&mut out,1.0)?;
    object(&mut out,&[
        ("app",Value::String(app)),
        ("type",Value::String("nonprivate")),
        ("flashVer",Value::String("T3Box/0.1")),
        ("tcUrl",Value::String(tc_url)),
        ("fpad",Value::Boolean(false)),
        ("capabilities",Value::Number(15.0)),
        ("audioCodecs",Value::Number(0x0400 as f64)),
        ("videoCodecs",Value::Number(0x0080 as f64)),
        ("videoFunction",Value::Number(1.0)),
        ("objectEncoding",Value::Number(0.0)),
    ])?;
    Ok(out)
}
pub fn create_stream_command() -> io::Result<Vec<u8>> {
    let mut out=Vec::new();
    string(&mut out,"createStream")?;
    number(&mut out,2.0)?;
    null(&mut out)?;
    Ok(out)
}
pub fn publish_command(stream_key: &str) -> io::Result<Vec<u8>> {
    if stream_key.is_empty() || stream_key.bytes().any(|b| b.is_ascii_control()) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid stream key"));
    }
    let mut out=Vec::new();
    string(&mut out,"publish")?;
    number(&mut out,0.0)?;
    null(&mut out)?;
    string(&mut out,stream_key)?;
    string(&mut out,"live")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn amf0_primitives_match_spec() {
        let mut x=Vec::new();string(&mut x,"abc").unwrap();
        assert_eq!(x,[2,0,3,b'a',b'b',b'c']);
        x.clear();number(&mut x,1.0).unwrap();
        assert_eq!(x,[0,0x3f,0xf0,0,0,0,0,0,0]);
        assert!(number(&mut x,f64::NAN).is_err());
    }
    #[test]
    fn commands_contain_required_values() {
        let c=connect_command("live2","rtmps://example.invalid/live2").unwrap();
        assert!(c.windows(7).any(|s|s==b"connect"));
        assert!(c.windows(5).any(|s|s==b"live2"));
        let p=publish_command("private-key").unwrap();
        assert!(p.windows(11).any(|s|s==b"private-key"));
        assert!(publish_command("").is_err());
        assert!(create_stream_command().unwrap().windows(12).any(|s|s==b"createStream"));
    }
}
