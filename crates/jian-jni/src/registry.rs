//! Engine-handle registry (Task 5 Step 1).
//!
//! Every JNI native receives an opaque `jlong` handle and MUST validate it
//! before touching engine state: a closed or unknown handle returns
//! [`STATUS_CLOSING`](crate::STATUS_CLOSING) instead of dereferencing freed
//! memory. Handles are monotonic ids (never reused — no ABA), so a
//! destroyed engine's slot stays as a TOMBSTONE that still answers
//! `nativeLastError`, and a never-allocated id is simply unknown; both are
//! rejected identically.
//!
//! The generic [`Registry`] core carries no JNI or engine types, so it is
//! compiled and unit-tested on the host. The Android-only engine payload and
//! the process-global instance live behind `cfg(target_os = "android")` in
//! the modules that own the raw engine pointer and JNI global refs.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// Reserved handle: `nativeCreate` returns `0` for failure, and the
/// create-failure error text is read back through this id.
pub const HANDLE_FAILURE: i64 = 0;

/// One registry slot. `payload` is `None` once the engine is torn down
/// (tombstone); `error` outlives the payload so `nativeLastError` works after
/// teardown.
struct Slot<T> {
    payload: Option<T>,
    error: String,
}

struct RegistryState<T> {
    /// Monotonic id source; the first live handle is `1` (`0` is reserved).
    next_id: i64,
    slots: HashMap<i64, Slot<T>>,
    /// Error text for the last `nativeCreate` that produced no handle.
    create_error: String,
}

/// A tombstoning handle table. `T` is the per-engine payload (host tests use
/// a plain value; Android uses the engine record).
pub struct Registry<T> {
    state: Mutex<RegistryState<T>>,
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Registry<T> {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(RegistryState {
                next_id: 1,
                slots: HashMap::new(),
                create_error: String::new(),
            }),
        }
    }

    /// Locks the state, RECOVERING from poison. `with`'s closure and the
    /// `Into<String>` conversions in `set_error`/`set_create_error` DO run
    /// under this guard, so a panic there could poison it; recovering
    /// guarantees a later registry access invoked from a JNI native never
    /// itself panics and crosses the non-unwinding boundary (the recovered
    /// state is structurally intact — the panicking op simply had no effect).
    fn locked(&self) -> MutexGuard<'_, RegistryState<T>> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Registers a live engine, returning its handle (always `>= 1`).
    pub fn insert(&self, payload: T) -> i64 {
        let mut state = self.locked();
        let id = state.next_id;
        state.next_id += 1;
        state.slots.insert(
            id,
            Slot {
                payload: Some(payload),
                error: String::new(),
            },
        );
        id
    }

    /// Runs `f` against the live payload, or returns `None` when the handle
    /// is unknown or tombstoned — the caller maps `None` to `STATUS_CLOSING`.
    pub fn with<R>(&self, handle: i64, f: impl FnOnce(&T) -> R) -> Option<R> {
        let state = self.locked();
        let payload = state.slots.get(&handle)?.payload.as_ref()?;
        Some(f(payload))
    }

    /// Records the last error text for a live handle (no-op for an
    /// unknown/tombstoned handle — a tombstone keeps the error it died with).
    pub fn set_error(&self, handle: i64, message: impl Into<String>) {
        let mut state = self.locked();
        if let Some(slot) = state.slots.get_mut(&handle) {
            slot.error = message.into();
        }
    }

    /// Records the create-failure error text (read back via `HANDLE_FAILURE`).
    pub fn set_create_error(&self, message: impl Into<String>) {
        self.locked().create_error = message.into();
    }

    /// The last error text for a handle. `HANDLE_FAILURE` returns the
    /// create-failure text; a live or tombstoned handle returns its slot's
    /// text; an unknown handle returns the empty string.
    pub fn last_error(&self, handle: i64) -> String {
        let state = self.locked();
        if handle == HANDLE_FAILURE {
            return state.create_error.clone();
        }
        state
            .slots
            .get(&handle)
            .map(|slot| slot.error.clone())
            .unwrap_or_default()
    }

    /// Tombstones a handle, returning the owned payload EXACTLY ONCE so the
    /// caller can run teardown (engine close → window release → global-ref
    /// deletion). A second close, or an unknown handle, returns `None`. The
    /// slot and its error text are retained for `nativeLastError`.
    pub fn take_for_close(&self, handle: i64) -> Option<T> {
        let mut state = self.locked();
        state.slots.get_mut(&handle)?.payload.take()
    }

    /// Whether the handle currently names a live engine.
    pub fn is_live(&self, handle: i64) -> bool {
        let state = self.locked();
        state
            .slots
            .get(&handle)
            .map(|slot| slot.payload.is_some())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_assigns_monotonic_nonzero_handles() {
        let reg: Registry<u32> = Registry::new();
        let a = reg.insert(10);
        let b = reg.insert(20);
        assert!(a >= 1 && b > a, "handles are monotonic and nonzero");
        assert_ne!(a, HANDLE_FAILURE);
    }

    #[test]
    fn with_reaches_live_payload_and_rejects_unknown() {
        let reg: Registry<u32> = Registry::new();
        let h = reg.insert(42);
        assert_eq!(reg.with(h, |v| *v), Some(42));
        assert_eq!(reg.with(9999, |v| *v), None, "unknown handle rejected");
    }

    #[test]
    fn close_tombstones_exactly_once() {
        let reg: Registry<u32> = Registry::new();
        let h = reg.insert(7);
        assert!(reg.is_live(h));
        assert_eq!(reg.take_for_close(h), Some(7), "first close owns payload");
        assert!(!reg.is_live(h), "closed handle is tombstoned");
        assert_eq!(reg.take_for_close(h), None, "second close gets nothing");
        assert_eq!(reg.with(h, |v| *v), None, "tombstoned handle rejects work");
    }

    #[test]
    fn error_slot_survives_teardown() {
        let reg: Registry<u32> = Registry::new();
        let h = reg.insert(1);
        reg.set_error(h, "boom");
        assert_eq!(reg.last_error(h), "boom");
        let _ = reg.take_for_close(h);
        assert_eq!(
            reg.last_error(h),
            "boom",
            "nativeLastError must work after teardown"
        );
    }

    #[test]
    fn create_failure_error_read_through_reserved_handle() {
        let reg: Registry<u32> = Registry::new();
        reg.set_create_error("bad doc");
        assert_eq!(reg.last_error(HANDLE_FAILURE), "bad doc");
        assert_eq!(reg.last_error(4242), "", "unknown handle has no error");
    }

    #[test]
    fn set_error_on_unknown_handle_is_a_noop() {
        let reg: Registry<u32> = Registry::new();
        reg.set_error(1234, "nope");
        assert_eq!(reg.last_error(1234), "");
    }

    #[test]
    fn access_recovers_after_a_closure_panics_under_the_lock() {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        use std::sync::Arc;

        let reg: Arc<Registry<u32>> = Arc::new(Registry::new());
        let h = reg.insert(5);
        // A `with` closure panics WHILE holding the guard, poisoning it.
        let poisoned = {
            let reg = reg.clone();
            catch_unwind(AssertUnwindSafe(|| {
                reg.with(h, |_| panic!("boom under the lock"));
            }))
        };
        assert!(poisoned.is_err(), "the closure panic propagated");
        // A subsequent access RECOVERS (no panic on the poisoned lock) and
        // the state is intact — the panicking op simply had no effect.
        assert_eq!(reg.with(h, |v| *v), Some(5), "state intact after recovery");
        reg.set_error(h, "ok");
        assert_eq!(reg.last_error(h), "ok");
    }
}
