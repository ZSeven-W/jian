//! Token bucket rate limiter — spec §7.
//!
//! Capacity 10, refill 1 token per 100 ms (≈ 10 calls/sec). Per
//! session. `list_available_actions` does **not** consume a token;
//! only `execute_action` does.

use std::time::Instant;

const CAPACITY: f64 = 10.0;
const REFILL_PER_SEC: f64 = 10.0;

#[derive(Debug, Clone)]
pub(crate) struct TokenBucket {
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new() -> Self {
        Self {
            tokens: CAPACITY,
            last: Instant::now(),
        }
    }

    /// Try to take one token. Returns `true` on success, `false`
    /// when the bucket is empty (caller surfaces `rate_limited`).
    pub fn take(&mut self) -> bool {
        self.refill(Instant::now())
    }

    /// Test-friendly variant that uses an injected clock so we don't
    /// need to sleep in unit tests.
    pub fn take_at(&mut self, now: Instant) -> bool {
        self.refill(now)
    }

    fn refill(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        if elapsed > 0.0 {
            let add = elapsed * REFILL_PER_SEC;
            self.tokens = (self.tokens + add).min(CAPACITY);
            self.last = now;
        }
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl Default for TokenBucket {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_drains_after_ten_calls() {
        let mut b = TokenBucket::new();
        let now = Instant::now();
        for _ in 0..10 {
            assert!(b.take_at(now), "first 10 should succeed");
        }
        assert!(!b.take_at(now), "11th immediately blocked");
    }

    #[test]
    fn refills_at_rate() {
        use std::time::Duration;
        let mut b = TokenBucket::new();
        let t0 = Instant::now();
        for _ in 0..10 {
            assert!(b.take_at(t0));
        }
        assert!(!b.take_at(t0));
        // 100ms later → +1 token.
        let t1 = t0 + Duration::from_millis(100);
        assert!(b.take_at(t1));
        // Immediately empty again.
        assert!(!b.take_at(t1));
    }

    #[test]
    fn refill_caps_at_capacity() {
        use std::time::Duration;
        let mut b = TokenBucket::new();
        let t0 = Instant::now();
        // Drain.
        for _ in 0..10 {
            b.take_at(t0);
        }
        // Long gap — bucket should cap at 10, not accumulate.
        let t1 = t0 + Duration::from_secs(60);
        for _ in 0..10 {
            assert!(b.take_at(t1));
        }
        assert!(!b.take_at(t1));
    }
}
