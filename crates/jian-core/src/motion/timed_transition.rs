//! The clock of a short timed transition: when it started, how long it
//! runs, how far along it is, and when the host should wake next.
//!
//! OpenPencil carried two copies of this arithmetic, one for Preview's
//! screen transitions and one for the canvas layout transition. Both moved
//! here unchanged; the screen transition and any host-side tween share it.

/// Frame step a running transition asks the host to wake at.
const FRAME_INTERVAL_MS: u64 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedTransition {
    started_at_ms: u64,
    duration_ms: u64,
}

impl TimedTransition {
    pub fn new(started_at_ms: u64, duration_ms: u64) -> Self {
        Self {
            started_at_ms,
            duration_ms,
        }
    }

    pub fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// The host time the transition settles at.
    pub fn end_ms(&self) -> u64 {
        self.started_at_ms.saturating_add(self.duration_ms)
    }

    pub fn is_active(&self, now_ms: u64) -> bool {
        now_ms < self.end_ms()
    }

    /// Next wake time while running: one frame ahead, never past the end.
    pub fn next_deadline_ms(&self, now_ms: u64) -> Option<u64> {
        if !self.is_active(now_ms) {
            return None;
        }
        Some((now_ms.saturating_add(FRAME_INTERVAL_MS)).min(self.started_at_ms + self.duration_ms))
    }

    /// Elapsed share of the duration, clamped to [0, 1].
    pub fn linear_t(&self, now_ms: u64) -> f32 {
        let elapsed = now_ms.saturating_sub(self.started_at_ms);
        (elapsed as f32 / self.duration_ms.max(1) as f32).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_runs_from_start_to_end_and_clamps() {
        let clock = TimedTransition::new(1_000, 240);
        assert_eq!(clock.linear_t(900), 0.0);
        assert_eq!(clock.linear_t(1_000), 0.0);
        assert!((clock.linear_t(1_120) - 0.5).abs() < 1e-6);
        assert_eq!(clock.linear_t(1_240), 1.0);
        assert_eq!(clock.linear_t(9_000), 1.0);
    }

    #[test]
    fn deadlines_step_a_frame_and_stop_at_the_end() {
        let clock = TimedTransition::new(1_000, 40);
        assert_eq!(clock.next_deadline_ms(1_000), Some(1_016));
        assert_eq!(clock.next_deadline_ms(1_030), Some(1_040));
        assert_eq!(clock.next_deadline_ms(1_040), None);
        assert!(!clock.is_active(1_040));
    }

    #[test]
    fn a_zero_duration_is_already_over() {
        let clock = TimedTransition::new(500, 0);
        assert!(!clock.is_active(500));
        assert_eq!(clock.next_deadline_ms(500), None);
        assert_eq!(clock.linear_t(500), 0.0);
    }
}
