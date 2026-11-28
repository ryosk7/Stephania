//! Single-owner media writer with bounded FIFO and absolute media age.
//! No encoded frame is selectively dropped. Overflow invalidates the session.
use std::{collections::VecDeque, io, net::{Shutdown,TcpStream}, sync::{Arc,Mutex,Condvar}, thread::{self,JoinHandle}, time::{Duration,Instant}};
use crate::{native_video::NativeMediaSink,native_x264::EncodedAccessUnit,native_aac::AacPacket};

const MAX_PACKETS: usize = 32;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_millis(250);

enum Media { Video(EncodedAccessUnit), Audio(AacPacket) }
impl Media {
    fn bytes(&self) -> usize {
        match self {
            Self::Video(v) => v.nals.iter().map(Vec::len).sum(),
            Self::Audio(a) => a.raw.len(),
        }
    }
}
struct Entry { media: Media, expires: Instant, bytes: usize }
#[derive(Default)]
struct Queue { entries: VecDeque<Entry>, bytes: usize, closed: bool, aborted: bool }
impl Queue {
    fn push(&mut self, media: Media, captured: Instant, now: Instant) -> io::Result<()> {
        let bytes = media.bytes();
        let expires = captured + MAX_AGE;
        // One access unit larger than the byte budget (a big keyframe) is
        // accepted into an empty queue; it is bounded by the RTMP message size.
        if self.closed || self.aborted || expires <= now || self.entries.len() >= MAX_PACKETS ||
            (bytes > MAX_BYTES.saturating_sub(self.bytes) && !self.entries.is_empty()) {
            self.aborted = true;
            return Err(timeout());
        }
        self.bytes += bytes;
        self.entries.push_back(Entry { media, expires, bytes });
        Ok(())
    }
    fn pop(&mut self, now: Instant) -> io::Result<Option<Media>> {
        if self.aborted { return Err(timeout()); }
        if let Some(e) = self.entries.pop_front() {
            self.bytes -= e.bytes;
            if e.expires <= now { self.aborted = true; return Err(timeout()); }
            Ok(Some(e.media))
        } else { Ok(None) }
    }
}
fn timeout() -> io::Error { io::Error::new(io::ErrorKind::TimedOut,"media queue deadline or capacity exceeded") }
type Shared = (Mutex<Queue>,Condvar);

pub(crate) struct MediaWorker {
    shared: Arc<Shared>,
    socket: TcpStream,
    worker: Option<JoinHandle<io::Result<u64>>>,
}
impl MediaWorker {
    pub(crate) fn start<T: io::Read + io::Write + Send + 'static>(mut sink: NativeMediaSink<T>, socket: TcpStream) -> io::Result<Self> {
        let shared = Arc::new((Mutex::new(Queue::default()),Condvar::new()));
        let copy = Arc::clone(&shared);
        let abort_socket = socket.try_clone()?;
        let worker = thread::Builder::new().name("stephania-media-send".into()).spawn(move || {
            let result = (|| {
                loop {
                    let item = {
                        let (lock,wake) = &*copy;
                        let q = lock.lock().map_err(|_|io::Error::other("media queue poisoned"))?;
                        let mut q = wake.wait_while(q,|q|q.entries.is_empty() && !q.closed && !q.aborted)
                            .map_err(|_|io::Error::other("media queue poisoned"))?;
                        match q.pop(Instant::now())? {
                            Some(m) => m,
                            None => break,
                        }
                    };
                    match item {
                        Media::Video(v) => sink.publish_unit(&v)?,
                        Media::Audio(a) => {
                            let ms = u32::try_from(i128::from(a.pts_samples)*1000/48000)
                                .map_err(|_|io::Error::new(io::ErrorKind::InvalidInput,"AAC timestamp out of range"))?;
                            sink.push_aac(ms,&a.raw)?;
                        }
                    }
                }
                Ok(sink.finish().1)
            })();
            if result.is_err() {
                if let Ok(mut q) = copy.0.lock() { q.aborted = true; copy.1.notify_all(); }
                let _ = abort_socket.shutdown(Shutdown::Both);
            }
            result
        })?;
        Ok(Self { shared, socket, worker: Some(worker) })
    }

    fn submit(&self, media: Media, captured: Instant) -> io::Result<()> {
        let result = self.shared.0.lock().map_err(|_|io::Error::other("media queue poisoned"))?
            .push(media,captured,Instant::now());
        self.shared.1.notify_one();
        if result.is_err() { let _ = self.socket.shutdown(Shutdown::Both); }
        result
    }
    pub(crate) fn video(&self, unit: EncodedAccessUnit, captured: Instant) -> io::Result<()> {
        self.submit(Media::Video(unit),captured)
    }
    pub(crate) fn audio(&self, packets: Vec<AacPacket>, captured: Instant) -> io::Result<()> {
        for packet in packets { self.submit(Media::Audio(packet),captured)?; }
        Ok(())
    }
    pub(crate) fn check(&self) -> io::Result<()> {
        let q = self.shared.0.lock().map_err(|_|io::Error::other("media queue poisoned"))?;
        if q.aborted || q.entries.front().is_some_and(|e|e.expires <= Instant::now()) { Err(timeout()) } else { Ok(()) }
    }
    pub(crate) fn finish(mut self) -> io::Result<u64> {
        {
            let mut q = self.shared.0.lock().map_err(|_|io::Error::other("media queue poisoned"))?;
            q.closed = true;
            self.shared.1.notify_one();
        }
        self.worker.take().unwrap().join().map_err(|_|io::Error::other("media worker panicked"))?
    }
}
impl Drop for MediaWorker {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            if let Ok(mut q) = self.shared.0.lock() { q.aborted = true; self.shared.1.notify_all(); }
            let _ = self.socket.shutdown(Shutdown::Both);
            let _ = worker.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn audio(n:usize) -> Media { Media::Audio(AacPacket {pts_samples:0,raw:vec![0;n]}) }
    #[test]
    fn queue_is_fifo_and_overflow_invalidates_session() {
        let t=Instant::now();
        let mut q=Queue::default();
        q.push(audio(1),t,t).unwrap();
        q.push(audio(2),t,t).unwrap();
        assert_eq!(q.pop(t).unwrap().unwrap().bytes(),1);
        assert_eq!(q.bytes,2);
        assert!(q.push(audio(MAX_BYTES),t,t).is_err());
        assert!(q.pop(t).is_err());
    }
    #[test]
    fn one_large_access_unit_fits_an_empty_queue() {
        let t=Instant::now();
        let mut q=Queue::default();
        q.push(audio(3*MAX_BYTES),t,t).unwrap();
        assert!(q.push(audio(1),t,t).is_err());
    }
    #[test]
    fn age_and_count_are_bounded() {
        let t=Instant::now();
        let mut q=Queue::default();
        q.push(audio(1),t,t).unwrap();
        assert!(q.pop(t+MAX_AGE).is_err());
        let mut q=Queue::default();
        for _ in 0..MAX_PACKETS {q.push(audio(1),t,t).unwrap();}
        assert!(q.push(audio(1),t,t).is_err());
    }
}
