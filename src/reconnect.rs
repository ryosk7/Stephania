//! Bounded exponential backoff for fresh publish sessions.
use std::{io, time::Duration};

pub fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs(1u64 << failures.saturating_sub(1).min(5))
}

pub fn retryable(error: &io::Error) -> bool {
    !matches!(error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::InvalidInput)
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
}
