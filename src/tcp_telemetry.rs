//! Linux TCP telemetry; no external dependencies or subprocesses.
//! Kernel ABI prefix through tcpi_total_retrans (linux/tcp.h).
use std::{io, net::TcpStream, os::fd::AsRawFd, ffi::{c_int,c_ulong,c_void}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpSample {
    pub rtt_us: u32,
    pub rtt_variance_us: u32,
    pub retransmitted_segments: u32,
    pub unacked_segments: u32,
    /// Sent but not acknowledged plus not-yet-sent bytes.
    pub send_queue_bytes: u32,
    /// Bytes queued in TCP but not yet transmitted; excludes TLS user-space data.
    pub not_sent_bytes: u32,
}

extern "C" {
    fn getsockopt(fd: c_int, level: c_int, name: c_int, value: *mut c_void, len: *mut u32) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}
const IPPROTO_TCP: c_int = 6;
const TCP_INFO: c_int = 11;
const SIOCOUTQ: c_ulong = 0x5411;
const SIOCOUTQNSD: c_ulong = 0x894b;

fn field(data: &[u8], offset: usize) -> io::Result<u32> {
    let bytes = data.get(offset..offset+4).ok_or_else(|| io::Error::new(
        io::ErrorKind::Unsupported, "short Linux TCP_INFO"))?;
    Ok(u32::from_ne_bytes(bytes.try_into().unwrap()))
}
fn queue(socket: &TcpStream, request: c_ulong) -> io::Result<u32> {
    let mut bytes: c_int = 0;
    // SAFETY: live borrowed fd and writable C int, Linux ioctl ABI.
    if unsafe { ioctl(socket.as_raw_fd(), request, &mut bytes as *mut c_int) } != 0 {
        return Err(io::Error::last_os_error());
    }
    u32::try_from(bytes).map_err(|_| io::Error::other("negative TCP queue size"))
}

impl TcpSample {
    pub fn read(socket: &TcpStream) -> io::Result<Self> {
        let mut data = [0u8; 104];
        let mut len = data.len() as u32;
        // SAFETY: kernel writes at most len bytes into initialized storage;
        // socklen_t is u32 on Linux. No Rust representation of kernel bitfields.
        if unsafe { getsockopt(socket.as_raw_fd(), IPPROTO_TCP, TCP_INFO,
            data.as_mut_ptr().cast(), &mut len) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let data = &data[..(len as usize).min(data.len())];
        Ok(Self {
            unacked_segments: field(data,24)?,
            rtt_us: field(data,68)?,
            rtt_variance_us: field(data,72)?,
            retransmitted_segments: field(data,100)?,
            send_queue_bytes: queue(socket,SIOCOUTQ)?,
            not_sent_bytes: queue(socket,SIOCOUTQNSD)?,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_short_kernel_prefix() {
        assert!(field(&[0;100],100).is_err());
        assert_eq!(field(&42u32.to_ne_bytes(),0).unwrap(),42);
    }
    #[test]
    fn reads_live_connected_socket() {
        let listener=std::net::TcpListener::bind(("127.0.0.1",0)).unwrap();
        let socket=TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_peer,_)=listener.accept().unwrap();
        let sample=TcpSample::read(&socket).unwrap();
        assert!(sample.not_sent_bytes <= sample.send_queue_bytes);
        assert_eq!(sample.send_queue_bytes,0);
    }
}
