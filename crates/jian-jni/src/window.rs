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

use std::cell::Cell;
use std::ptr;

use jni::objects::JObject;
use jni::JNIEnv;

use ndk_sys::{ANativeWindow, ANativeWindow_fromSurface, ANativeWindow_release};

thread_local! {
    /// The window currently attached on THIS engine thread. Each engine owns
    /// its thread exclusively, so an engine-thread-local is per-engine state:
    /// `attach`/`resume` install it, `suspend`/`destroy` release it.
    static CURRENT_WINDOW: Cell<*mut ANativeWindow> = const { Cell::new(ptr::null_mut()) };
}

/// Installs `window` as the current one, releasing any previously attached
/// window first. Called on the engine thread right after a successful
/// acquire (attach/resume).
///
/// The previous window is released UNCONDITIONALLY, even if it shares
/// `window`'s address: every `ANativeWindow_fromSurface` adds a reference,
/// so a matching pointer still represents a distinct owned reference that
/// must be balanced. (In practice suspend clears the previous window first,
/// so there is usually none.)
///
/// # Safety
/// `window` must be a window from [`acquire`] not yet released, or null.
pub unsafe fn set_current_window(window: *mut ANativeWindow) {
    let previous = CURRENT_WINDOW.with(|w| w.replace(window));
    if !previous.is_null() {
        unsafe { release(previous) };
    }
}

/// Releases and clears the current window (suspend/destroy). No-op when none
/// is attached.
pub fn take_current_window() {
    let window = CURRENT_WINDOW.with(|w| w.replace(ptr::null_mut()));
    // SAFETY: any stored window came from `acquire` and is released once here.
    unsafe { release(window) };
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
