//! Animation primitives — pure math for square-wave caret blink,
//! breathe-in / breathe-out cycles, and similar periodic UI cues.
//!
//! No platform deps. Hosts provide a millisecond clock (e.g.
//! `std::time::Instant::elapsed().as_millis()` on desktop or
//! `performance.now()` on the browser); this module gives the
//! widgets a way to translate "current ms + last-event ms" into
//! "is the caret visible right now?" + "when does the host need
//! to wake again?".
//!
//! The split keeps jian's UI primitives uniform across products:
//! both OpenPencil's chrome and Zode's TUI / IDE bridge consume
//! [`blink_visible`] / [`next_blink_flip_ms`], rather than each
//! reinventing the same square-wave logic.

/// Compute whether a blinking element should be visible at
/// `now_ms`, given the most-recent activity anchor `anchor_ms`
/// and a half-period `period_ms`.
///
/// `anchor_ms` resets the cycle — set it to `now_ms` whenever the
/// element receives focus or accepts a keystroke so the caret
/// appears immediately rather than mid-blink.
///
/// Special cases:
/// - `period_ms == 0` → always visible (animation disabled).
/// - `now_ms < anchor_ms` → treated as 0 delta (visible).
pub fn blink_visible(now_ms: u64, anchor_ms: u64, period_ms: u64) -> bool {
    if period_ms == 0 {
        return true;
    }
    let delta = now_ms.saturating_sub(anchor_ms);
    (delta / period_ms) % 2 == 0
}

/// Return the next `now_ms` value at which [`blink_visible`] will
/// flip state. Hosts use this to schedule their event-loop
/// wake-up (e.g. `winit::ControlFlow::WaitUntil`) so the next
/// redraw lands exactly on the boundary instead of polling.
///
/// `period_ms == 0` → returns `now_ms + 1` so the host doesn't
/// burn a tight loop; callers that disable blinking should also
/// stop scheduling.
pub fn next_blink_flip_ms(now_ms: u64, anchor_ms: u64, period_ms: u64) -> u64 {
    if period_ms == 0 {
        return now_ms.saturating_add(1);
    }
    let delta = now_ms.saturating_sub(anchor_ms);
    anchor_ms.saturating_add(((delta / period_ms) + 1) * period_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_visible_at_anchor() {
        assert!(blink_visible(0, 0, 500));
        assert!(blink_visible(100, 0, 500));
        assert!(blink_visible(499, 0, 500));
    }

    #[test]
    fn caret_hidden_in_second_half() {
        assert!(!blink_visible(500, 0, 500));
        assert!(!blink_visible(750, 0, 500));
        assert!(!blink_visible(999, 0, 500));
    }

    #[test]
    fn caret_visible_again_after_full_period() {
        assert!(blink_visible(1000, 0, 500));
        assert!(blink_visible(1499, 0, 500));
    }

    #[test]
    fn anchor_resets_phase() {
        // 250 ms after anchor, caret should still be visible
        // even if `now_ms` would be in a "hidden" half-period
        // relative to the previous anchor.
        assert!(blink_visible(500, 250, 500));
        assert!(blink_visible(749, 250, 500));
        assert!(!blink_visible(750, 250, 500));
    }

    #[test]
    fn period_zero_is_always_visible() {
        assert!(blink_visible(0, 0, 0));
        assert!(blink_visible(99_999, 0, 0));
    }

    #[test]
    fn now_before_anchor_is_visible() {
        // Saturating-sub means a stale anchor never produces a
        // panic; treat as the current cycle's start.
        assert!(blink_visible(100, 1_000_000, 500));
    }

    #[test]
    fn next_flip_lands_on_period_boundary() {
        assert_eq!(next_blink_flip_ms(0, 0, 500), 500);
        assert_eq!(next_blink_flip_ms(100, 0, 500), 500);
        assert_eq!(next_blink_flip_ms(499, 0, 500), 500);
        assert_eq!(next_blink_flip_ms(500, 0, 500), 1000);
        assert_eq!(next_blink_flip_ms(750, 0, 500), 1000);
    }

    #[test]
    fn next_flip_respects_anchor() {
        assert_eq!(next_blink_flip_ms(300, 200, 500), 700);
        assert_eq!(next_blink_flip_ms(699, 200, 500), 700);
        assert_eq!(next_blink_flip_ms(700, 200, 500), 1200);
    }

    #[test]
    fn next_flip_handles_zero_period() {
        // `period_ms == 0` → caller has disabled blinking; we
        // hand back `now+1` so the host doesn't busy-loop on a
        // huge next_flip_ms.
        assert_eq!(next_blink_flip_ms(42, 0, 0), 43);
    }
}
