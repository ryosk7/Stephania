//! Time-based one-time codes (RFC 6238 over RFC 4226, HMAC-SHA-1, 6 digits,
//! 30 s steps), as shown by Stephania Connect for maintenance windows
//! (docs/MAINTENANCE.md). SHA-1 is what authenticator apps and the RFC test
//! vectors use; HMAC-SHA-1 remains sound for this purpose.
use ring::hmac;

pub const STEP_SECONDS: u64 = 30;
pub const DIGITS: u32 = 6;

/// RFC 4226 HOTP value for `counter`, reduced to `digits` digits.
pub fn hotp(secret: &[u8], counter: u64, digits: u32) -> u32 {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let mac = hmac::sign(&key, &counter.to_be_bytes());
    let mac = mac.as_ref();
    let offset = (mac[19] & 0x0f) as usize;
    let value = u32::from_be_bytes([mac[offset] & 0x7f, mac[offset + 1], mac[offset + 2], mac[offset + 3]]);
    value % 10u32.pow(digits)
}

pub fn step(unix_seconds: u64) -> u64 { unix_seconds / STEP_SECONDS }

/// The 6-digit code for a Unix time.
pub fn code(secret: &[u8], unix_seconds: u64) -> u32 { hotp(secret, step(unix_seconds), DIGITS) }

/// How many used steps the caller keeps (`verify`'s `used`).
pub const USED_STEPS_KEPT: usize = 32;

/// Checks `input` against the current step and one step either side.
/// `used` lists the steps already used, oldest first (the caller keeps the
/// last `USED_STEPS_KEPT`). Each code works once: a step in `used` is
/// always refused, and while the clock is at or past the most recent use,
/// so is any step before it. If the most recent use lies ahead of the
/// clock, the clock went back (a cold boot without a real-time clock or
/// network time); codes for the current steps are then accepted unless
/// they are in `used`, instead of refusing every code until the clock
/// catches up. Returns the matching step, which the caller records.
pub fn verify(secret: &[u8], input: &str, unix_seconds: u64, used: &[u64]) -> Option<u64> {
    let input = input.trim();
    if input.len() != DIGITS as usize || !input.bytes().all(|b| b.is_ascii_digit()) { return None; }
    let wanted: u32 = input.parse().ok()?;
    let now = step(unix_seconds);
    let clock_behind = used.last().is_some_and(|&last| last > now + 1);
    let mut matched = None;
    // Compare every candidate so timing does not reveal which step matched.
    for s in [now.saturating_sub(1), now, now + 1] {
        let fresh = !used.contains(&s) && (clock_behind || used.last().is_none_or(|&last| s > last));
        if fresh && hotp(secret, s, DIGITS) == wanted && matched.is_none() { matched = Some(s); }
    }
    matched
}

/// `used` after `step` was accepted: appended, oldest dropped.
pub fn record_used(used: &mut Vec<u64>, step: u64) {
    used.retain(|&s| s != step);
    used.push(step);
    if used.len() > USED_STEPS_KEPT { used.drain(..used.len() - USED_STEPS_KEPT); }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn rfc_6238_sha1_vectors() {
        // Appendix B, truncated to 8 digits there; we check 8 and 6.
        for (t, eight) in [(59, 94287082), (1111111109, 7081804), (1111111111, 14050471), (1234567890, 89005924), (2000000000, 69279037), (20000000000, 65353130)] {
            assert_eq!(hotp(RFC_SECRET, step(t), 8), eight, "t={t}");
            assert_eq!(code(RFC_SECRET, t), eight % 1_000_000, "t={t}");
        }
    }

    #[test]
    fn rfc_4226_hotp_vectors() {
        let expected = [755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489];
        for (counter, value) in expected.iter().enumerate() {
            assert_eq!(hotp(RFC_SECRET, counter as u64, 6), *value);
        }
    }

    #[test]
    fn accepts_one_step_of_drift_and_rejects_more() {
        let t = 1_700_000_000;
        let now = format!("{:06}", code(RFC_SECRET, t));
        assert_eq!(verify(RFC_SECRET, &now, t, &[]), Some(step(t)));
        assert_eq!(verify(RFC_SECRET, &now, t + 30, &[]), Some(step(t)));
        assert_eq!(verify(RFC_SECRET, &now, t - 30, &[]), Some(step(t)));
        assert_eq!(verify(RFC_SECRET, &now, t + 61, &[]), None);
    }

    #[test]
    fn a_used_step_and_earlier_ones_are_refused() {
        let t = 1_700_000_000;
        let c = format!("{:06}", code(RFC_SECRET, t));
        let used = verify(RFC_SECRET, &c, t, &[]).unwrap();
        assert_eq!(verify(RFC_SECRET, &c, t, &[used]), None);
        let earlier = format!("{:06}", code(RFC_SECRET, t - 30));
        assert_eq!(verify(RFC_SECRET, &earlier, t, &[used]), None);
        let later = format!("{:06}", code(RFC_SECRET, t + 30));
        assert_eq!(verify(RFC_SECRET, &later, t, &[used]), Some(used + 1));
    }

    #[test]
    fn a_clock_that_went_back_still_accepts_fresh_codes_but_no_replays() {
        // Used yesterday at real time; today the device booted at an
        // earlier time (no RTC battery, no network time).
        let real = 1_800_000_000;
        let boot = 1_700_000_000;
        let mut used = vec![];
        record_used(&mut used, step(real));
        let fresh = format!("{:06}", code(RFC_SECRET, boot));
        let s = verify(RFC_SECRET, &fresh, boot, &used).expect("a current code works after the clock went back");
        record_used(&mut used, s);
        assert_eq!(verify(RFC_SECRET, &fresh, boot, &used), None, "still once only");
        let before = format!("{:06}", code(RFC_SECRET, boot - 30));
        assert_eq!(verify(RFC_SECRET, &before, boot, &used), None, "steps before the latest use stay refused");
        // The next cold boot lands on the same steps again: a code used
        // there in an earlier boot is still refused.
        let mut again = used.clone();
        record_used(&mut again, step(real) + 10);
        assert_eq!(verify(RFC_SECRET, &fresh, boot, &again), None);
        let next = format!("{:06}", code(RFC_SECRET, boot + 30));
        assert_eq!(verify(RFC_SECRET, &next, boot + 30, &again), Some(step(boot + 30)));
        // The used-step code at the real time is never accepted again.
        let old = format!("{:06}", code(RFC_SECRET, real));
        assert_eq!(verify(RFC_SECRET, &old, real, &used), None);
    }

    #[test]
    fn used_steps_are_bounded() {
        let mut used = vec![];
        for s in 0..(USED_STEPS_KEPT as u64 + 5) { record_used(&mut used, s); }
        assert_eq!(used.len(), USED_STEPS_KEPT);
        assert_eq!((used[0], *used.last().unwrap()), (5, USED_STEPS_KEPT as u64 + 4));
    }

    #[test]
    fn malformed_input_is_refused() {
        for bad in ["", "12345", "1234567", "12a456", "-12345", "１２３４５６"] {
            assert_eq!(verify(RFC_SECRET, bad, 1_700_000_000, &[]), None, "{bad:?}");
        }
    }
}
