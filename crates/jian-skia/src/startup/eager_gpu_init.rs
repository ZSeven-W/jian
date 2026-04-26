//! `spawn_gpu_init` — eager GPU `DirectContext` init (Plan 19 Task 2).
//!
//! `jian-skia` is backend-agnostic: it does not own a Metal / D3D12 / GL /
//! Vulkan context constructor. The host crate that *does* own one (Plan 8
//! desktop, Plan 11 OpenPencil canvas, Plan 12 web) passes its factory
//! closure into [`spawn_gpu_init`], which dispatches the blocking init on
//! a worker thread and returns a oneshot receiver the host can later
//! await from a `StartupPhase::InitGpuContext` phase impl.
//!
//! The function is a thin, named wrapper over [`super::spawn_blocking_init`].
//! Naming it `spawn_gpu_init` makes greppable intent in host code and
//! gives Plan 19 Task 8's `jian perf startup` table a stable phase note
//! source ("Metal", "D3D12", "GL").
//!
//! ### Why a parameterised factory rather than a hard-coded constructor
//!
//! - The actual `DirectContext` constructor depends on which `skia-safe`
//!   feature flag is active (`metal` / `d3d` / `gl` / `vulkan`). A single
//!   hard-coded path inside `jian-skia` would force every host into the
//!   same backend.
//! - The host already knows which backend it picked (it owns the window
//!   surface, the platform handle, the feature flag selection) — it is
//!   the right place to construct the context.
//! - Keeping the factory closure-shaped also lets hosts wrap the call in
//!   tracing, error handling, or fallback policy without `jian-skia`
//!   imposing a particular shape.
//!
//! ### Example: registering against the StartupDriver
//!
//! ```ignore
//! use jian_core::startup::{StartupDriver, StartupPhase, StartupConfig};
//! use jian_skia::startup::eager_gpu_init::spawn_gpu_init;
//!
//! // Started before Runtime::new — overlaps schema parse / state seed
//! // for the duration of the GPU context build (~50-120 ms).
//! let gpu_rx = spawn_gpu_init(|| {
//!     // Host-supplied factory. With the `metal` feature on macOS:
//!     //   skia_safe::gpu::direct_contexts::make_metal(&device, &queue, None)
//!     // With `d3d` on Windows:
//!     //   skia_safe::gpu::direct_contexts::make_d3d(&backend_context, None)
//!     // etc.
//!     host_specific::create_direct_context()
//! });
//!
//! let mut driver = StartupDriver::new();
//! driver.register(StartupPhase::InitGpuContext, move || async move {
//!     let _ctx = gpu_rx.await.map_err(|e| e.to_string())?;
//!     // Stash _ctx in shared state for RenderFirstFrame to pick up.
//!     Ok(())
//! });
//! ```

use futures::channel::oneshot;

/// Spawn the host's GPU `DirectContext` factory on a worker thread.
///
/// Returns a [`oneshot::Receiver`] the caller awaits from the
/// `InitGpuContext` phase impl. See module docs for usage.
///
/// `T` is whatever the factory produces — the host's wrapper around
/// `skia_safe::gpu::DirectContext`, a `(DirectContext, RenderTarget)` pair,
/// a fallible `Result<DirectContext, GpuInitError>`, etc. `jian-skia`
/// neither inspects nor unwraps the value; it just shuttles it across the
/// channel.
///
/// # Factory contract
///
/// **The factory must return a fully-usable resource bundle.** Any
/// platform-specific binding that has to run on the main thread, the
/// window thread, or any thread other than this worker — surface
/// attachment that requires a `Window` handle, GL context "make current",
/// AppKit `NSOpenGLContext::makeCurrentContext`, etc. — must happen
/// *before* `spawn_gpu_init` is called, must be encoded into `T` so the
/// caller knows another step is required, **or** must be a separate named
/// startup phase. A factory that returns a half-initialised context
/// silently defeats the abstraction: the awaiting phase will succeed but
/// `RenderFirstFrame` will fail with a confusing thread-affinity error.
///
/// ### Backends with per-draw-call thread affinity (e.g. `make_current`)
///
/// Some backends — classic OpenGL is the canonical one — require the
/// rendering thread to call `make_current` on every render frame, not
/// just once at construction. Those contexts are **never** "fully usable"
/// from this worker thread: even after `spawn_gpu_init` returns, the
/// caller still has to bind the context on the render thread before each
/// `RenderFirstFrame` and subsequent draw. Two ways to handle this
/// honestly:
///
/// 1. **Don't `spawn_gpu_init` for those backends.** Have the host
///    construct the context on the render thread the first time it's
///    needed; the wall-clock saving from off-thread init is forfeited
///    but the contract stays clean.
/// 2. **Encode the binding step into `T`** (e.g. return a
///    `(NeedsMakeCurrent, RawHandle)` pair) so the awaiting phase has to
///    perform the bind explicitly before forwarding the context to
///    `RenderFirstFrame`. The awaiter then becomes the canonical place
///    where main-thread / render-thread re-binding happens.
///
/// What you must **not** do is return a bare context handle that "works"
/// on the worker thread but silently breaks once a different thread tries
/// to draw. The thread-affinity error surfaces deep inside Skia and is
/// extremely hard to root-cause from a startup trace.
#[must_use = "dropping the receiver discards the only handle to observe init success or failure; \
              await it from a StartupPhase::InitGpuContext phase impl, or store it where the \
              awaiter can pick it up"]
pub fn spawn_gpu_init<T, F>(factory: F) -> oneshot::Receiver<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    super::spawn_blocking_init(factory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// Smoke test: a stub factory that pretends to build a "GPU context"
    /// runs on a worker thread and the result reaches the caller.
    #[test]
    fn smoke_runs_factory_and_returns_value() {
        let ran = Arc::new(AtomicBool::new(false));
        let ran_clone = Arc::clone(&ran);
        let rx = spawn_gpu_init(move || {
            ran_clone.store(true, Ordering::SeqCst);
            "fake-gpu-ctx"
        });
        let v = block_on(rx).expect("factory completed");
        assert_eq!(v, "fake-gpu-ctx");
        assert!(ran.load(Ordering::SeqCst));
    }

    /// The host can return a `Result` from its factory and `jian-skia`
    /// will pass it through verbatim. Phase impls that need to fail
    /// gracefully on a missing GPU surface use this shape.
    #[test]
    fn passes_through_result_factories() {
        let rx = spawn_gpu_init::<Result<u32, &'static str>, _>(|| Err("no metal device"));
        let v = block_on(rx).expect("worker completed");
        assert_eq!(v, Err("no metal device"));
    }
}
