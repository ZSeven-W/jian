//! Swipe 400ms throttle — spec §6.3.
//!
//! Independent from the per-action concurrency tracker (§6.1).
//! Spec: "swipe_* same direction within 400ms → second call returns
//! Busy(already_running). Prevents AI clients from spamming pan
//! events that the host can't physically process at frame cadence."
//!
//! Indexed by `(direction_kind, action_name)` — different actions in
//! different scopes don't share a throttle bucket. Direction is the
//! `SourceKind` itself (SwipeLeft/Right/Up/Down) so a swipe-left
//! immediately followed by swipe-right is allowed.

use jian_core::action_surface::SourceKind;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Per-direction last-fire timestamp. Two consecutive same-direction
/// calls within `THROTTLE_WINDOW` are rejected; a different
/// direction (or any other action kind) is allowed.
const THROTTLE_WINDOW: Duration = Duration::from_millis(400);

#[derive(Debug, Default)]
pub(crate) struct SwipeThrottle {
    last: HashMap<(SourceKind, String), Instant>,
}

impl SwipeThrottle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when the swipe action may proceed; `false`
    /// when the same direction fired within the throttle window.
    /// Test-friendly: clock injected via `now`.
    pub fn try_acquire_at(&mut self, kind: SourceKind, name: &str, now: Instant) -> bool {
        if !is_swipe(kind) {
            return true; // non-swipe kinds bypass.
        }
        let key = (kind, name.to_owned());
        if let Some(prev) = self.last.get(&key).copied() {
            if now.saturating_duration_since(prev) < THROTTLE_WINDOW {
                return false;
            }
        }
        self.last.insert(key, now);
        true
    }

    pub fn try_acquire(&mut self, kind: SourceKind, name: &str) -> bool {
        self.try_acquire_at(kind, name, Instant::now())
    }
}

fn is_swipe(kind: SourceKind) -> bool {
    matches!(
        kind,
        SourceKind::SwipeLeft | SourceKind::SwipeRight | SourceKind::SwipeUp | SourceKind::SwipeDown
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_swipe_fires() {
        let mut t = SwipeThrottle::new();
        let t0 = Instant::now();
        assert!(t.try_acquire_at(SourceKind::SwipeLeft, "carousel", t0));
    }

    #[test]
    fn same_direction_within_window_blocked() {
        let mut t = SwipeThrottle::new();
        let t0 = Instant::now();
        assert!(t.try_acquire_at(SourceKind::SwipeLeft, "carousel", t0));
        let t1 = t0 + Duration::from_millis(100);
        assert!(!t.try_acquire_at(SourceKind::SwipeLeft, "carousel", t1));
    }

    #[test]
    fn different_direction_allowed() {
        let mut t = SwipeThrottle::new();
        let t0 = Instant::now();
        assert!(t.try_acquire_at(SourceKind::SwipeLeft, "carousel", t0));
        let t1 = t0 + Duration::from_millis(50);
        assert!(t.try_acquire_at(SourceKind::SwipeRight, "carousel", t1));
    }

    #[test]
    fn different_action_allowed() {
        let mut t = SwipeThrottle::new();
        let t0 = Instant::now();
        assert!(t.try_acquire_at(SourceKind::SwipeLeft, "carousel_a", t0));
        let t1 = t0 + Duration::from_millis(50);
        assert!(t.try_acquire_at(SourceKind::SwipeLeft, "carousel_b", t1));
    }

    #[test]
    fn same_direction_after_window_allowed() {
        let mut t = SwipeThrottle::new();
        let t0 = Instant::now();
        assert!(t.try_acquire_at(SourceKind::SwipeLeft, "carousel", t0));
        let t1 = t0 + Duration::from_millis(450);
        assert!(t.try_acquire_at(SourceKind::SwipeLeft, "carousel", t1));
    }

    #[test]
    fn non_swipe_bypass() {
        let mut t = SwipeThrottle::new();
        let t0 = Instant::now();
        assert!(t.try_acquire_at(SourceKind::Tap, "btn", t0));
        let t1 = t0 + Duration::from_millis(1);
        // Tap is not throttled here — handled elsewhere by §6.1
        // same-action concurrency.
        assert!(t.try_acquire_at(SourceKind::Tap, "btn", t1));
    }
}
