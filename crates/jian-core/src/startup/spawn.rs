//! `SpawnHandle` trait — pluggable scheduling for startup phases (Plan 19 Task 12 scaffolding).
//!
//! The current [`StartupDriver`] uses a calling-thread `FuturesUnordered`
//! pool — every phase is polled by whoever drives `block_on(driver.run(...))`.
//! That's correct and well-tested but it forecloses on hosts that want
//! true multi-thread parallelism for non-Send-friendly phases (e.g.
//! letting GPU init's worker-thread oneshot `recv()` happen concurrently
//! with schema parsing on a different OS thread).
//!
//! Plan 19 Task 12 introduces a [`SpawnHandle`] abstraction so host
//! adapters can plug in their preferred **local-executor**
//! scheduler — tokio's `LocalSet`, `async-std::task::spawn_local`, or
//! a custom single-thread pool. The trait deliberately scopes itself
//! to local executors: [`SpawnHandle::spawn`] takes a [`LocalBoxFuture`]
//! (no `Send` bound) so phase impls holding `Rc` / `RefCell` /
//! main-thread-bound GPU handles can still be scheduled. A separate
//! `SendSpawnHandle` trait that takes `BoxFuture<'static, T>` (with
//! `Send`) is the right shape for true multi-thread executors like
//! `tokio::spawn` (the cross-thread variant) — it lands when a host
//! crate actually needs it; **do not assume this trait can back
//! `tokio::spawn` directly** (Codex round 1 MEDIUM caught the docs
//! claiming otherwise).
//!
//! This module ships the local-executor **trait + default impl**
//! today; routing the driver's [`FuturesUnordered`] loop through
//! `SpawnHandle::spawn` lands in a focused follow-up commit because:
//!
//! 1. **Object-safety constraint.** The trait's `spawn` method returns
//!    the same future shape as its input. Making that polymorphic over
//!    `<T>` breaks `Box<dyn SpawnHandle>`; baking in the driver's
//!    internal `Result<PhaseTiming, StartupError>` type leaks an
//!    implementation detail into the public API. Resolving that needs
//!    care that doesn't fit in this commit's diff.
//! 2. **Real-world host adapters land elsewhere.** A `TokioSpawn` impl
//!    belongs in a host crate that already pulls `tokio` (Plan 19
//!    follow-up + Plan 11 OpenPencil canvas swap). Putting the trait
//!    in `jian-core` now lets those crates start writing against it.
//!
//! Tests below demonstrate the trait shape with a counting custom impl
//! so future driver integration can lift this directly.
//!
//! [`StartupDriver`]: crate::startup::StartupDriver
//! [`FuturesUnordered`]: futures::stream::FuturesUnordered

use futures::future::LocalBoxFuture;

/// Wraps a phase future before it is awaited.
///
/// Implementations may return the input future unchanged (the
/// [`CooperativeSpawn`] default), schedule it on a tokio `LocalSet`
/// or `async_std::task::spawn_local` and return a JoinHandle future,
/// or apply tracing / instrumentation around the await.
///
/// **Local-executor only.** `fut: LocalBoxFuture<'static, Output>`
/// has no `Send` bound, so this trait does **not** support
/// cross-thread executors like `tokio::spawn`. Hosts that need true
/// multi-thread parallelism for non-Send-friendly phases combine
/// `SpawnHandle` (for the main-thread cooperative path) with
/// `jian_skia::startup::spawn_blocking_init` (for the dedicated-
/// OS-thread path; lives in the `jian-skia` crate which `jian-core`
/// does not depend on, hence the plain-backtick reference rather than
/// an intra-doc link) — or wait for a `SendSpawnHandle` sibling trait
/// that lands when a host crate needs it.
///
/// `Output` is generic so the trait stays object-safe per concrete
/// `T`. `Box<dyn SpawnHandle<()>>` and `Box<dyn SpawnHandle<MyResult>>`
/// are both valid trait objects; a single trait object can only spawn
/// futures with one fixed `Output` type, but that's the natural shape
/// for the driver's internal use (every phase future yields the same
/// `Result<PhaseTiming, StartupError>`).
pub trait SpawnHandle<Output: 'static>: 'static {
    /// Schedule `fut` and return a future yielding the same `Output`.
    /// The returned future, when awaited, must produce exactly the
    /// value `fut` would have produced if polled directly. Side
    /// effects (where the future runs, what waker chain it uses) are
    /// implementation-defined.
    fn spawn(&self, fut: LocalBoxFuture<'static, Output>) -> LocalBoxFuture<'static, Output>;
}

/// Default cooperative single-threaded handle: returns the input
/// future unmodified, so whoever drives `driver.run(...).await`
/// also polls every phase. Identical to today's driver behaviour
/// (`FuturesUnordered` on the calling thread).
///
/// `CooperativeSpawn` carries no state — it's a zero-sized marker
/// (`Default + Copy + Clone + Debug`). Use it as the default
/// `SpawnHandle` whenever a host doesn't want to plug in a custom
/// executor.
#[derive(Debug, Default, Copy, Clone)]
pub struct CooperativeSpawn;

impl<Output: 'static> SpawnHandle<Output> for CooperativeSpawn {
    fn spawn(&self, fut: LocalBoxFuture<'static, Output>) -> LocalBoxFuture<'static, Output> {
        fut
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use futures::future::FutureExt;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn cooperative_spawn_returns_value_through_unmodified() {
        let s = CooperativeSpawn;
        let fut: LocalBoxFuture<'static, i32> = async { 42 }.boxed_local();
        let v = block_on(SpawnHandle::<i32>::spawn(&s, fut));
        assert_eq!(v, 42);
    }

    #[test]
    fn cooperative_spawn_passes_through_unit_futures() {
        let s = CooperativeSpawn;
        let flag = Rc::new(Cell::new(false));
        let f = Rc::clone(&flag);
        let fut: LocalBoxFuture<'static, ()> = async move { f.set(true) }.boxed_local();
        block_on(SpawnHandle::<()>::spawn(&s, fut));
        assert!(flag.get(), "wrapped future must still execute");
    }

    /// Demonstrates the canonical custom-impl shape that future driver
    /// integration will exercise: a `SpawnHandle` that records every
    /// `spawn` call. Real adapters (tokio / async-std) follow the same
    /// pattern but route through their executor's spawn primitive.
    #[derive(Default)]
    struct CountingSpawn {
        count: Rc<Cell<usize>>,
    }

    impl<T: 'static> SpawnHandle<T> for CountingSpawn {
        fn spawn(&self, fut: LocalBoxFuture<'static, T>) -> LocalBoxFuture<'static, T> {
            self.count.set(self.count.get() + 1);
            fut
        }
    }

    #[test]
    fn custom_spawn_handle_is_invoked_per_call() {
        let s = CountingSpawn::default();
        let counter = Rc::clone(&s.count);
        for i in 0..5 {
            let fut: LocalBoxFuture<'static, i32> = async move { i }.boxed_local();
            let v = block_on(SpawnHandle::<i32>::spawn(&s, fut));
            assert_eq!(v, i);
        }
        assert_eq!(counter.get(), 5, "spawn called once per future");
    }

    #[test]
    fn box_dyn_spawn_handle_is_object_safe_per_concrete_type() {
        // Compile-time proof: `Box<dyn SpawnHandle<i32>>` is a valid
        // trait object. If the trait ever becomes non-object-safe
        // (e.g. by adding a generic-on-method) this test fails to
        // compile.
        let boxed: Box<dyn SpawnHandle<i32>> = Box::new(CooperativeSpawn);
        let fut: LocalBoxFuture<'static, i32> = async { 7 }.boxed_local();
        let v = block_on(boxed.spawn(fut));
        assert_eq!(v, 7);
    }
}
