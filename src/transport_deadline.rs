//! A per-session watchdog: abort a stalled transport, never resume a partial RTMP message.
use std::{io, net::{Shutdown, TcpStream}, sync::{Arc,Mutex,Condvar}, thread::{self,JoinHandle}, time::{Duration,Instant}};

#[derive(Default)]
struct State { deadline: Option<Instant>, stopped: bool, expired: bool }

type Shared = (Mutex<State>, Condvar);

pub struct DeadlineWatchdog {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct DeadlineHandle(Arc<Shared>);

impl DeadlineHandle {
    /// Arm an absolute per-message deadline; the caller clears it only after successful I/O.
    pub fn arm(&self, budget: Duration) -> io::Result<()> {
        let mut state = self.0.0.lock().map_err(|_| io::Error::other("deadline lock poisoned"))?;
        if state.expired || state.stopped {
            return Err(io::Error::new(io::ErrorKind::TimedOut,"transport deadline expired"));
        }
        state.deadline = Some(Instant::now() + budget);
        self.0.1.notify_one();
        Ok(())
    }

    pub fn complete(&self) -> io::Result<()> {
        let mut state = self.0.0.lock().map_err(|_| io::Error::other("deadline lock poisoned"))?;
        if state.expired || state.deadline.is_some_and(|d| Instant::now() >= d) {
            // Leave an expired deadline armed; the worker will close the connection.
            self.0.1.notify_one();
            return Err(io::Error::new(io::ErrorKind::TimedOut,"transport deadline expired"));
        }
        state.deadline = None;
        self.0.1.notify_one();
        Ok(())
    }

    pub fn expired(&self) -> bool {
        self.0.0.lock().map_or(true, |s| s.expired)
    }
}

impl DeadlineWatchdog {
    pub fn start(socket: TcpStream) -> io::Result<Self> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let copy = Arc::clone(&shared);
        let worker = thread::Builder::new().name("t3box-deadline".into()).spawn(move || {
            let (lock, wake) = &*copy;
            let Ok(mut state) = lock.lock() else { return; };
            loop {
                if state.stopped { break; }
                if let Some(deadline) = state.deadline {
                    let now = Instant::now();
                    if now >= deadline {
                        state.expired = true;
                        let _ = socket.shutdown(Shutdown::Both);
                        break;
                    }
                    let Ok((next,_)) = wake.wait_timeout(state, deadline-now) else { break; };
                    state = next;
                } else {
                    let Ok(next) = wake.wait(state) else { break; };
                    state = next;
                }
            }
        })?;
        Ok(Self { shared, worker: Some(worker) })
    }

    pub fn handle(&self) -> DeadlineHandle { DeadlineHandle(Arc::clone(&self.shared)) }
}

impl Drop for DeadlineWatchdog {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.0.lock() {
            state.stopped = true;
            self.shared.1.notify_one();
        }
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, net::TcpListener};
    fn pair() -> (TcpStream,TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1",0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        (client,listener.accept().unwrap().0)
    }

    #[test]
    fn expiry_interrupts_blocked_io() {
        let (mut client,_peer) = pair();
        client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let guard = DeadlineWatchdog::start(client.try_clone().unwrap()).unwrap();
        let handle = guard.handle();
        handle.arm(Duration::from_millis(40)).unwrap();
        let start = Instant::now();
        assert!(matches!(client.read(&mut [0]), Ok(0)|Err(_)));
        assert!(handle.expired());
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(handle.arm(Duration::from_secs(1)).is_err());
    }

    #[test]
    fn completed_message_disarms_and_idle_does_not_expire() {
        let (client,_peer) = pair();
        let guard = DeadlineWatchdog::start(client).unwrap();
        let handle = guard.handle();
        handle.arm(Duration::from_secs(1)).unwrap();
        handle.complete().unwrap();
        thread::sleep(Duration::from_millis(60));
        assert!(!handle.expired());
        handle.arm(Duration::from_secs(1)).unwrap();
        handle.complete().unwrap();
    }
}
