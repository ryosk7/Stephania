//! Deterministic primitives for media scheduling and failure recovery.
//! These are independent of the FFmpeg compatibility backend.
use std::collections::VecDeque;
use std::time::Duration;

/// Bounded retry avoids a tight crash loop when a venue cable is disconnected.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    failures: u32,
}
impl RetryPolicy {
    pub fn new() -> Self { Self { failures: 0 } }
    pub fn reset(&mut self) { self.failures = 0; }
    pub fn failed(&mut self) -> Duration {
        let power = self.failures.min(5);
        self.failures = self.failures.saturating_add(1);
        Duration::from_secs((1u64 << power).min(30))
    }
}

/// Maps timestamps from two hardware device clocks to a common timeline.
/// For production audio, resampling must additionally correct measured drift.
#[derive(Default, Debug)]
pub struct ClockMapper {
    first_input_us: Option<u64>,
    last_output_us: u64,
}
impl ClockMapper {
    pub fn map(&mut self, timestamp_us: u64) -> u64 {
        let first = *self.first_input_us.get_or_insert(timestamp_us);
        let pts = timestamp_us.saturating_sub(first).max(self.last_output_us);
        self.last_output_us = pts;
        pts
    }
    pub fn reset(&mut self) {
        self.first_input_us = None;
        self.last_output_us = 0;
    }
}

/// Keeps oldest frames from accumulating excessive latency. Prefer dropping
/// stale video frames over allowing unbounded RAM consumption.
pub struct VideoQueue<T> {
    frames: VecDeque<T>,
    capacity: usize,
    dropped: u64,
}
impl<T> VideoQueue<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self { frames: VecDeque::with_capacity(capacity), capacity, dropped: 0 }
    }
    pub fn push(&mut self, frame: T) {
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
            self.dropped += 1;
        }
        self.frames.push_back(frame);
    }
    pub fn pop(&mut self) -> Option<T> { self.frames.pop_front() }
    pub fn dropped(&self) -> u64 { self.dropped }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_is_bounded_and_resets() {
        let mut r = RetryPolicy::new();
        let periods: Vec<_> = (0..8).map(|_| r.failed().as_secs()).collect();
        assert_eq!(periods, vec![1, 2, 4, 8, 16, 30, 30, 30]);
        r.reset();
        assert_eq!(r.failed().as_secs(), 1);
    }
    #[test]
    fn pts_never_regresses() {
        let mut c = ClockMapper::default();
        assert_eq!(c.map(5000), 0);
        assert_eq!(c.map(9000), 4000);
        assert_eq!(c.map(6000), 4000);
        c.reset();
        assert_eq!(c.map(300), 0);
    }
    #[test]
    fn overflow_discards_oldest_video_only() {
        let mut q = VideoQueue::new(2);
        q.push(1); q.push(2); q.push(3);
        assert_eq!(q.dropped(), 1);
        assert_eq!(q.pop(), Some(2));
        assert_eq!(q.pop(), Some(3));
    }
}
