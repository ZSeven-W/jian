//! Process-global reentrant lock serializing all skia `FontMgr` /
//! DirectWrite access.
//!
//! skia's Windows font backend (DirectWrite) segfaults under concurrent
//! font enumeration / paragraph shaping from multiple threads. In production
//! this happens whenever one thread measures a layout while another paints or
//! measures on its own — e.g. the CLI standard-mode design route runs
//! `run_design_worker` → `editor_state_to_layout_scene` → [`SkiaMeasure`] off
//! the UI thread while the UI thread keeps painting. A single global lock
//! around every font / measure / draw entry keeps DirectWrite effectively
//! single-threaded.
//!
//! The lock is **reentrant per thread**: nested acquisitions (`measure` →
//! `build_collection` → `asset_provider`, or a `draw_text` that resolves a
//! fallback face) must not self-deadlock. Uncontended acquisition is a
//! thread-local flag check plus one `Mutex` lock — negligible on the paint
//! hot path (~30 `draw_text` calls per frame). Under real contention the two
//! threads serialize their DirectWrite work, which is exactly the goal;
//! critical sections are per-measure / per-draw, so neither thread holds the
//! lock long enough to stall the other visibly.
//!
//! Native-only: `jian-skia` is never compiled for wasm (the browser renders
//! through CanvasKit), so `Mutex` / `thread_local` are always available.
//!
//! [`SkiaMeasure`]: crate::measure::SkiaMeasure

use std::cell::Cell;
use std::sync::Mutex;

static FONT_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    /// Whether the current thread already holds [`FONT_LOCK`]. Guards the
    /// reentrant fast path so nested font calls don't deadlock on the
    /// non-reentrant `Mutex`.
    static HELD: Cell<bool> = const { Cell::new(false) };
}

/// Restores the thread-local `HELD` flag to `false` on drop, so a panic that
/// unwinds through the outermost [`with_font_lock`] frame can't wedge the
/// thread into a permanently "held" state.
struct HeldReset;

impl Drop for HeldReset {
    fn drop(&mut self) {
        HELD.with(|h| h.set(false));
    }
}

/// Run `f` with the global skia-font lock held. Reentrant: if the current
/// thread already holds the lock, `f` runs directly without re-locking.
///
/// Wrap every entry point that constructs or uses a skia `FontMgr` /
/// `FontCollection` / paragraph shaper, or resolves a system typeface, so all
/// DirectWrite access across the process is serialized.
pub fn with_font_lock<R>(f: impl FnOnce() -> R) -> R {
    // Reentrant fast path: this thread already owns the lock (an outer font
    // call is on the stack). Just run — the outer frame holds the guard.
    if HELD.with(Cell::get) {
        return f();
    }
    // Outermost acquisition on this thread. `into_inner` shrugs off a
    // poisoned lock: a prior panic mid-measure left no durable skia state to
    // corrupt, whereas propagating the poison would take down every
    // subsequent paint.
    let _guard = FONT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    HELD.with(|h| h.set(true));
    // Clears `HELD` even if `f` unwinds (drops before `_guard`).
    let _reset = HeldReset;
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reentrant_acquisition_does_not_deadlock() {
        let out = with_font_lock(|| {
            // Nested acquisition on the same thread must run inline.
            with_font_lock(|| 41) + 1
        });
        assert_eq!(out, 42);
        // Flag is cleared once the outermost call returns.
        assert!(!HELD.with(Cell::get));
    }

    #[test]
    fn flag_clears_after_a_panic_unwinds() {
        let caught = std::panic::catch_unwind(|| {
            with_font_lock(|| panic!("boom"));
        });
        assert!(caught.is_err());
        // The reset guard must have cleared the thread-local flag so later
        // font work on this thread can still acquire the lock.
        assert!(!HELD.with(Cell::get));
        // And the lock itself is reacquirable (poison shrugged off).
        assert_eq!(with_font_lock(|| 7), 7);
    }
}
