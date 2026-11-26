//! Conservative AIMD controller. Pure policy, independent of sockets/codecs.
//! A rate ceiling is not an estimate of available bandwidth.
#[derive(Clone, Copy, Debug)]
pub struct Observation {
    pub not_sent_bytes: u32,
    pub rtt_us: u32,
    pub retrans_total: u32,
}

pub struct RateController {
    ceiling: u32,
    floor: u32,
    current: u32,
    previous_retrans: Option<u32>,
    healthy: u8,
    cooldown: u8,
}

impl RateController {
    pub fn new(ceiling: u32) -> Self {
        Self { ceiling, floor: (ceiling / 4).max(500).min(ceiling),
            current: ceiling, previous_retrans: None, healthy: 0, cooldown: 0 }
    }

    pub fn current(&self) -> u32 { self.current }

    /// Preserve the configured ceiling across sessions but restart conservatively.
    pub fn reconnect(&mut self) {
        self.current = (self.current * 3 / 4).max(self.floor);
        self.previous_retrans = None;
        self.healthy = 0;
        self.cooldown = 3;
    }

    /// Called once per second, only on a successful TCP sample.
    /// Estimated serialization time is a heuristic, not measured network delay.
    pub fn observe(&mut self, sample: Observation) -> Option<u32> {
        let retrans = self.previous_retrans.replace(sample.retrans_total)
            .is_some_and(|old| sample.retrans_total != old);
        let queue_ms = u64::from(sample.not_sent_bytes) * 8 / u64::from(self.current.max(1));
        let congested = queue_ms >= 150 || (retrans && queue_ms >= 50);
        let old = self.current;
        if congested {
            self.current = (self.current * 3 / 4).max(self.floor);
            self.healthy = 0;
            self.cooldown = 3;
        } else if self.cooldown > 0 {
            self.cooldown -= 1;
            self.healthy = 0;
        } else if queue_ms <= 25 && !retrans && sample.rtt_us <= 250_000 {
            self.healthy += 1;
            if self.healthy >= 5 {
                self.current = self.current.saturating_add(200).min(self.ceiling);
                self.healthy = 0;
            }
        } else {
            self.healthy = 0;
        }
        (old != self.current).then_some(self.current)
    }

    /// Missing samples must not count as healthy recovery intervals.
    pub fn missing_sample(&mut self) {
        self.healthy = 0;
        self.previous_retrans = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(bytes: u32, retrans: u32) -> Observation {
        Observation { not_sent_bytes: bytes, rtt_us: 80_000, retrans_total: retrans }
    }
    #[test]
    fn cuts_quickly_and_recovers_slowly() {
        let mut c = RateController::new(8000);
        assert_eq!(c.observe(sample(200_000,0)),Some(6000));
        assert_eq!(c.observe(sample(200_000,1)),Some(4500));
        for _ in 0..7 { assert_eq!(c.observe(sample(0,1)),None); }
        assert_eq!(c.observe(sample(0,1)),Some(4700));
        for _ in 0..30 { c.observe(sample(0,1)); }
        assert!(c.current <= 8000);
    }
    #[test]
    fn reconnect_preserves_reduced_rate_and_ceiling() {
        let mut c = RateController::new(8000);
        c.observe(sample(200_000,0));
        c.reconnect();
        assert_eq!(c.current(),4500);
        for _ in 0..7 { assert_eq!(c.observe(sample(0,0)),None); }
        assert_eq!(c.observe(sample(0,0)),Some(4700));
        for _ in 0..100 { c.observe(sample(0,0)); }
        assert_eq!(c.current(),8000);
    }
    #[test]
    fn floor_and_missing_samples() {
        let mut c = RateController::new(6000);
        for _ in 0..20 { c.observe(sample(1_000_000,0)); }
        assert_eq!(c.current,1500);
        for _ in 0..7 { assert_eq!(c.observe(sample(0,0)),None); }
        c.missing_sample();
        assert_eq!(c.observe(sample(0,0)),None);
        assert_eq!(c.current,1500);
    }
}
