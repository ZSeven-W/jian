//! `ANativeWindow` acquire/release (Task 5 Step 2, spec §6.7).
//!
//! Ownership contract: the CALLER frame globalizes the `Surface`; the window
//! itself is acquired and released ONLY on the engine thread, with the engine
//! thread's `JNIEnv`, immediately around `jian_attach_surface` / `jian_resume`
//! and after `jian_suspend` / `jian_destroy` (or a failed attach/resume —
//! safe because Task 3 guarantees no EGL object survives the failure). Every
//! acquire and release emits a paired log line so the Task 7 script can
//! assert leak-free pairing; a bare "no leak warning" proves nothing.

#![cfg(target_os = "android")]

use std::cell::{Cell, RefCell};

use jni::objects::JObject;
use jni::JNIEnv;

use ndk_sys::{ANativeWindow, ANativeWindow_fromSurface, ANativeWindow_release};

thread_local! {
    /// Every `ANativeWindow` this engine currently owns a reference to. Each
    /// engine owns its thread exclusively, so an engine-thread-local is
    /// per-engine state. On a clean (`Ok`) surface transition the engine has
    /// dropped its old EGL surface, so all-but-the-new window are released;
    /// on a Poisoned transition NONE are released (the engine may still
    /// borrow one), so they accumulate until a confirmed-`Ok` teardown.
    static OWNED_WINDOWS: RefCell<Vec<*mut ANativeWindow>> = const { RefCell::new(Vec::new()) };

    /// Set once any lifecycle call returns `Poisoned`: the engine is dead and
    /// may still borrow an owned window, so further surface operations are
    /// REFUSED (they must not acquire or release anything) until destroy.
    static POISONED: Cell<bool> = const { Cell::new(false) };
}

/// Whether this engine has been poisoned (surface ops must be refused).
pub fn is_poisoned() -> bool {
    POISONED.with(|p| p.get())
}

/// Latches the poisoned state (any lifecycle call returned `Poisoned`).
pub fn mark_poisoned() {
    POISONED.with(|p| p.set(true));
}

/// Records `window` as owned WITHOUT releasing anything — used on a Poisoned
/// attach/resume where the engine may already borrow it.
///
/// # Safety
/// `window` must be a non-null window from [`acquire`] not yet released.
pub unsafe fn track_window(window: *mut ANativeWindow) {
    OWNED_WINDOWS.with(|v| v.borrow_mut().push(window));
}

/// Commits `window` as the sole owned window after a clean `Ok` transition:
/// every PREVIOUSLY-owned window is no longer borrowed (the engine dropped
/// its old surface) and is released — including one sharing `window`'s
/// address, since `window` is a fresh acquisition not yet in the list and
/// every prior entry is a distinct reference.
///
/// # Safety
/// `window` must be a non-null window from [`acquire`] not yet released and
/// NOT already present in `OWNED_WINDOWS` (attach/resume acquire it fresh and
/// commit without tracking first).
pub unsafe fn commit_window(window: *mut ANativeWindow) {
    OWNED_WINDOWS.with(|v| {
        let mut owned = v.borrow_mut();
        for previous in owned.drain(..) {
            // SAFETY: each came from `acquire`; released exactly once here.
            unsafe { release(previous) };
        }
        owned.push(window);
    });
}

/// Releases and forgets EVERY owned window (a confirmed-`Ok` suspend or
/// destroy). No-op when none are owned.
pub fn release_all_windows() {
    OWNED_WINDOWS.with(|v| {
        for window in v.borrow_mut().drain(..) {
            // SAFETY: each came from `acquire` and is released exactly once.
            unsafe { release(window) };
        }
    });
}

/// Acquires the `ANativeWindow` backing a `Surface`, incrementing its
/// reference count. Returns null when the Surface has no native window (e.g.
/// already destroyed) — the caller treats null as an attach/resume failure.
///
/// Runs on the engine thread with its `JNIEnv`. The returned window is owned
/// by the caller and MUST be paired with [`release`].
///
/// # Safety
/// `surface` must be a live `android.view.Surface`; `env` must be the current
/// thread's env.
pub unsafe fn acquire(env: &JNIEnv, surface: &JObject) -> *mut ANativeWindow {
    let raw_env = env.get_native_interface().cast();
    let raw_surface = surface.as_raw().cast();
    let window = unsafe { ANativeWindow_fromSurface(raw_env, raw_surface) };
    if window.is_null() {
        crate::alog::info(
            "JianJni",
            "window acquire failed (Surface has no native window)",
        );
    } else {
        crate::alog::info("JianJni", &format!("window acquired {window:p}"));
    }
    window
}

/// Releases a window acquired by [`acquire`], decrementing its reference
/// count. A null pointer is ignored (a failed acquire has nothing to release).
///
/// # Safety
/// `window` must be null or a window from [`acquire`] not yet released.
pub unsafe fn release(window: *mut ANativeWindow) {
    if window.is_null() {
        return;
    }
    crate::alog::info("JianJni", &format!("window released {window:p}"));
    unsafe { ANativeWindow_release(window) };
}
