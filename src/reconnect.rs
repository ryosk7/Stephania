//! Bounded exponential backoff for fresh publish sessions.
use std::{io, time::Duration};

pub fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs(1u64 << failures.saturating_sub(1).min(5))
}

/// Before the first accepted publish: configuration (InvalidInput) and
/// authentication (PermissionDenied) errors are final.
pub fn retryable(error: &io::Error) -> bool {
    !matches!(error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::InvalidInput)
}

/// Classification for a publisher run. Once the ingest has accepted a
/// publish, settings and stream key are proven good: any later failure
/// (a capture glitch, an encoder or packetizer rejecting one unit, a
/// "stream key busy" reply while the old session times out) is transient,
/// so the publisher reconnects with backoff instead of ending the broadcast.
pub fn retryable_after(error: &io::Error, published: bool) -> bool {
    published || retryable(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backoff_is_bounded_and_auth_is_fatal() {
        assert_eq!(retry_delay(1), Duration::from_secs(1));
        assert_eq!(retry_delay(4), Duration::from_secs(8));
        assert_eq!(retry_delay(u32::MAX), Duration::from_secs(32));
        assert!(retryable(&io::Error::from(io::ErrorKind::BrokenPipe)));
        assert!(!retryable(&io::Error::from(io::ErrorKind::PermissionDenied)));
    }
    #[test]
    fn errors_after_an_accepted_publish_are_transient() {
        for kind in [io::ErrorKind::InvalidInput, io::ErrorKind::PermissionDenied, io::ErrorKind::InvalidData] {
            assert!(retryable_after(&io::Error::from(kind), true));
        }
        assert!(!retryable_after(&io::Error::from(io::ErrorKind::InvalidInput), false));
        assert!(retryable_after(&io::Error::from(io::ErrorKind::TimedOut), false));
    }
}
