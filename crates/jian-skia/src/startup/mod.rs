//! Cold-start helpers for `jian-skia` (Plan 19 Task 2).
//!
//! Plan 19 §C19 wants expensive blocking init — GPU `DirectContext`
//! creation, font db scan, image decode warm-up — kicked off **before**
//! the schema parse begins so it overlaps the critical path. This module
//! ships [`spawn_blocking_init`], the generic worker-thread primitive that
//! every backend hook re-uses; the named GPU shim lives in
//! [`eager_gpu_init::spawn_gpu_init`].
//!
//! Because the actual GPU surface factories are still deferred (Plan 8
//! Tasks 7-10 / Plan 11 / Plan 12), `spawn_gpu_init` takes the factory
//! closure as a parameter rather than hard-coding the constructor — that
//! keeps `jian-skia` backend-agnostic and lets the host crate pick the
//! right `DirectContext` builder per platform when those tasks land.
//!
//! ### How it composes with the StartupDriver
//!
//! ```ignore
//! use jian_core::startup::{StartupDriver, StartupPhase};
//! use jian_skia::startup::eager_gpu_init::spawn_gpu_init;
//!
//! // Started before Runtime::new — overlaps schema parse / state seed.
//! let gpu_rx = spawn_gpu_init(|| host_specific::create_direct_context());
//!
//! let mut driver = StartupDriver::new();
//! driver.register(StartupPhase::InitGpuContext, move || async move {
//!     let _ctx = gpu_rx.await.map_err(|e| e.to_string())?;
//!     Ok(())
//! });
//! ```

pub mod eager_gpu_init;

use futures::channel::oneshot;

/// Run `f` on a fresh OS thread; return a oneshot receiver that resolves
/// to its result.
///
/// Use this for **blocking, CPU-bound** init that would otherwise stall
/// the main thread (GPU context creation, font db scan, image decode
/// warm-up). The thread is detached — there is no `JoinHandle` to wait on
/// — and the return type is async-await compatible (`futures::channel::oneshot::Receiver<T>`
/// implements `Future`).
///
/// # Failure modes
///
/// - If `f` panics, the worker thread aborts and `tx` is dropped; the
///   awaiting receiver yields `Err(oneshot::Canceled)`. The caller can
///   surface this through a phase impl as a `PhaseFailed` error or fall
///   back to a slower synchronous init.
/// - The OS thread spawn itself can fail in extreme cases (resource
///   limits); `std::thread::spawn` panics on failure rather than
///   returning `Result`, so this function will panic too. In practice
///   that condition means the process is doomed regardless.
///
/// # Send bound
///
/// Both `T` and `F` need `Send + 'static` because the closure runs on a
/// new thread and the result crosses the channel. This is intentionally
/// stricter than the `StartupDriver::register` bounds (which accept
/// `!Send` futures) because OS-thread spawning is the whole point: the
/// init we want to overlap is exactly the kind that has no business
/// running on the main thread.
#[must_use = "dropping the receiver discards the only handle to observe the init outcome; \
              await it from a StartupPhase impl, store it for a later phase, or use a fire-\
              and-forget detached `std::thread::spawn` directly if you really don't care"]
pub fn spawn_blocking_init<T, F>(f: F) -> oneshot::Receiver<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let v = f();
        // Receiver may already have been dropped if startup aborted before
        // this phase was awaited. That's fine — discard the value silently.
        let _ = tx.send(v);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    // (Duration / Instant no longer needed — the parallelism test now
    // proves overlap via Barrier::wait deadlock-detection rather than
    // wall-clock assertions; CI flake risk on loaded runners is gone.)

    #[test]
    fn delivers_value_through_receiver() {
        let rx = spawn_blocking_init(|| 42_u32);
        let v = block_on(rx).expect("init thread completed");
        assert_eq!(v, 42);
    }

    #[test]
    fn closure_runs_on_a_different_thread() {
        let main_id = thread::current().id();
        let rx = spawn_blocking_init(move || thread::current().id());
        let worker_id = block_on(rx).expect("init thread completed");
        assert_ne!(
            main_id, worker_id,
            "spawn_blocking_init must execute the closure off the main thread"
        );
    }

    /// The whole point of `spawn_blocking_init`: blocking work overlaps
    /// the caller's parallel work.
    ///
    /// We prove overlap **structurally** via a `Barrier::new(2)` —
    /// both threads must reach the barrier before either can
    /// proceed, so the barrier returning is itself proof both threads
    /// are on-CPU concurrently. A serialised implementation (worker
    /// runs only after caller blocks on `block_on(rx)`) would deadlock
    /// at the barrier and the test would time out. No wall-clock
    /// assertion needed (and no wall-clock assertion *should* be made:
    /// CI hardware varies enough that a "total < 90 ms" check on the
    /// post-barrier window flakes on loaded macOS runners — Codex
    /// round 1 of Task 2 flagged the flake risk as "not zero" and
    /// macOS aarch64 CI surfaced it on the first real run).
    #[test]
    fn worker_runs_in_parallel_with_caller() {
        use std::sync::Barrier;

        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = Arc::clone(&barrier);
        let rx = spawn_blocking_init(move || {
            // Sync point: blocks until the caller also reaches the
            // barrier. Reaching here proves the worker is on-CPU
            // *and* the main thread is too (both must arrive for the
            // barrier to release).
            worker_barrier.wait();
            "done"
        });
        // Main thread meets the barrier. If the worker hadn't been
        // dispatched concurrently with this call, the barrier would
        // deadlock and the test would hang. The barrier returning
        // is the parallelism proof.
        barrier.wait();
        let v = block_on(rx).expect("init thread completed");
        assert_eq!(v, "done");
    }

    #[test]
    fn panic_in_closure_yields_canceled() {
        // The worker thread will print the panic backtrace to stderr; we
        // only care that the receiver yields `Err(Canceled)` because the
        // sender was dropped during the unwind.
        let rx = spawn_blocking_init::<u32, _>(|| panic!("init failed"));
        let outcome = block_on(rx);
        assert!(outcome.is_err(), "expected Canceled, got {outcome:?}");
    }

    #[test]
    fn each_call_spawns_an_independent_thread() {
        // Multiple receivers must not collide. The shared counter proves
        // each closure ran exactly once.
        let counter = Arc::new(AtomicUsize::new(0));
        let mut receivers = Vec::new();
        for _ in 0..8 {
            let counter = Arc::clone(&counter);
            receivers.push(spawn_blocking_init(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                42
            }));
        }
        for rx in receivers {
            assert_eq!(block_on(rx).expect("worker done"), 42);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 8);
    }
}
