//! Engine → Java upcall trampolines (Task 5 Step 3).
//!
//! The C ABI invokes these callbacks synchronously ON the engine thread
//! (inside `jian_frame`, `jian_pointer`, …). Each trampoline copies its
//! borrowed C payload into owned Java values, calls the one `JianCallbacks`
//! receiver, then clears any pending exception. Every body is bracketed by a
//! JNI local frame so per-upcall Strings/arrays never accumulate in the
//! engine thread's local-reference table (the thread stays attached for its
//! whole life).
//!
//! `user_data` for the C callback table is a `*const EngineCtx` owned by the
//! engine record; it outlives every callback and is freed only in the
//! teardown final job, after `jian_destroy` returns.

#![cfg(target_os = "android")]

use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;

use jni::objects::{GlobalRef, JObject, JValue};
use jni::{JNIEnv, JavaVM};

use jian_engine_ffi::{JianCallbacks, JianCapabilityRequest, JianFieldInfo, JianRuntimeError};

use crate::engine_thread::{enter_callback_frame, exit_callback_frame};
use crate::marshal;

thread_local! {
    /// Set to `true` while a `nativeFrame` call drives `jian_frame`; the
    /// `needs_redraw` trampoline reads it to supply `fromFrame` (the C
    /// callback itself carries no origin bit).
    static FROM_FRAME: Cell<bool> = const { Cell::new(false) };
}

/// Marks the current engine-thread scope as originating from a frame pump.
pub struct FrameOriginGuard(bool);

impl FrameOriginGuard {
    pub fn enter() -> Self {
        let previous = FROM_FRAME.with(|f| f.replace(true));
        FrameOriginGuard(previous)
    }
}

impl Drop for FrameOriginGuard {
    fn drop(&mut self) {
        FROM_FRAME.with(|f| f.set(self.0));
    }
}

fn from_frame() -> bool {
    FROM_FRAME.with(|f| f.get())
}

/// Per-engine upcall context; the `user_data` behind the C callback table.
pub struct EngineCtx {
    vm: JavaVM,
    /// The `JianCallbacks` Java receiver (a global ref — valid across
    /// threads and callbacks).
    receiver: GlobalRef,
}

impl EngineCtx {
    pub fn new(vm: JavaVM, receiver: GlobalRef) -> Self {
        Self { vm, receiver }
    }

    /// The engine thread's `JNIEnv`. The engine thread is attached to the VM
    /// permanently at spawn, so `get_env` always succeeds here; a failure
    /// means we are off the engine thread (a bug) and the upcall is skipped.
    fn env(&self) -> Option<JNIEnv<'_>> {
        self.vm.get_env().ok()
    }
}

/// Builds the C callback table pointing at a freshly boxed [`EngineCtx`].
/// The returned raw pointer is the table's `user_data`; the engine record
/// owns it and frees it (`drop_ctx`) in the teardown final job.
pub fn build_callbacks(ctx: Box<EngineCtx>) -> (JianCallbacks, *mut EngineCtx) {
    let raw = Box::into_raw(ctx);
    let table = JianCallbacks {
        size: std::mem::size_of::<JianCallbacks>(),
        user_data: raw as *mut c_void,
        needs_redraw: Some(needs_redraw),
        runtime_error: Some(runtime_error),
        ime_control: Some(ime_control),
        input_focus_changed: Some(input_focus_changed),
        text_state_changed: Some(text_state_changed),
        capability_request: Some(capability_request),
        capability_cancelled: Some(capability_cancelled),
    };
    (table, raw)
}

/// Frees the boxed context. Called ONCE, on the engine thread, in the
/// teardown final job after `jian_destroy` has returned (no further callback
/// can fire).
///
/// # Safety
/// `raw` must be the pointer returned by [`build_callbacks`] and not yet
/// freed.
pub unsafe fn drop_ctx(raw: *mut EngineCtx) {
    if !raw.is_null() {
        drop(unsafe { Box::from_raw(raw) });
    }
}

/// Casts `user_data` back to the borrowed context. Returns `None` for a null
/// pointer (never expected — the table always carries a live context).
///
/// # Safety
/// `user_data` must be a `*const EngineCtx` from [`build_callbacks`] that is
/// still live (guaranteed while any callback can fire).
unsafe fn ctx<'a>(user_data: *mut c_void) -> Option<&'a EngineCtx> {
    (user_data as *const EngineCtx).as_ref()
}

/// Runs `body` inside a JNI local frame with the receiver in hand, then
/// checks-and-clears any pending exception. Missing env / frame errors are
/// swallowed: an upcall must never unwind across the C ABI.
fn upcall(ctx: &EngineCtx, capacity: i32, body: impl FnOnce(&mut JNIEnv, &JObject)) {
    let Some(mut env) = ctx.env() else {
        return;
    };
    let receiver = ctx.receiver.clone();
    // Bracket the callback so a native re-entered from the Java callback (e.g.
    // nativeDestroy) sees `in_callback_frame()` and defers per the no-re-entry
    // rule. The guard restores the depth even if the body panics.
    let _frame = CallbackFrame::enter();
    let _framed = env.with_local_frame(capacity, |env| -> Result<(), jni::errors::Error> {
        // Catch INSIDE the frame so a panic in marshalling or a JNI wrapper
        // can never unwind across the C callback ABI (which would abort) and
        // so `with_local_frame` still runs `PopLocalFrame`. The caught
        // payload is disposed through the guarded dropper: a panic_any
        // payload whose own Drop panics would otherwise re-panic before the
        // frame closure returns and skip PopLocalFrame.
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| body(env, receiver.as_obj()))) {
            crate::engine_thread::drop_guarded(payload);
        }
        Ok(())
    });
    // Clear any exception the Java callback left pending; describe it first
    // for the log. Both are best-effort. (A PushLocalFrame OOM above leaves
    // nothing to clean up beyond this.)
    if let Ok(true) = env.exception_check() {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

/// RAII bracket for a C callback frame (drives `close_deferred` routing on
/// callback-origin destroy). Restores the depth on drop, panic or not.
struct CallbackFrame;

impl CallbackFrame {
    fn enter() -> Self {
        enter_callback_frame();
        CallbackFrame
    }
}

impl Drop for CallbackFrame {
    fn drop(&mut self) {
        exit_callback_frame();
    }
}

/// Runs a callback trampoline body under an unwind guard covering the WHOLE
/// trampoline — the context lookup, the C-pointer marshalling, the JNI local
/// frame, and exception cleanup — so a panic anywhere (e.g. an allocation
/// failure while copying a string) can never cross the non-unwinding C
/// callback ABI. The caught payload is disposed through the guarded dropper
/// (a panicking-Drop payload cannot re-panic). A null/absent context is a
/// no-op.
fn run_trampoline(user_data: *mut c_void, body: impl FnOnce(&EngineCtx)) {
    let guarded = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` is a live `*const EngineCtx` while any callback
        // can fire (freed only after jian_destroy on the engine thread).
        if let Some(ctx) = unsafe { ctx(user_data) } {
            body(ctx);
        }
    }));
    if let Err(payload) = guarded {
        crate::engine_thread::drop_guarded(payload);
    }
}

extern "C" fn needs_redraw(user_data: *mut c_void, has_next_wake: bool, next_wake_ms: u64) {
    run_trampoline(user_data, |ctx| {
        let from_frame = from_frame();
        upcall(ctx, 2, |env, receiver| {
            let _ = env.call_method(
                receiver,
                "onNeedsRedraw",
                "(ZZJ)V",
                &[
                    JValue::Bool(from_frame as u8),
                    JValue::Bool(has_next_wake as u8),
                    JValue::Long(next_wake_ms as i64),
                ],
            );
        });
    });
}

extern "C" fn runtime_error(user_data: *mut c_void, error: *const JianRuntimeError) {
    run_trampoline(user_data, |ctx| {
        let Some(error) = (unsafe { error.as_ref() }) else {
            return;
        };
        let message = unsafe { borrowed_str(error.message_ptr, error.message_len) };
        let source = if error.source_ptr.is_null() {
            None
        } else {
            Some(unsafe { borrowed_str(error.source_ptr, error.source_len) })
        };
        let kind = error.kind as i32;
        upcall(ctx, 4, |env, receiver| {
            let Ok(jmessage) = env.new_string(&message) else {
                return;
            };
            let jsource = match &source {
                Some(s) => match env.new_string(s) {
                    Ok(js) => js.into(),
                    Err(_) => return,
                },
                None => JObject::null(),
            };
            let _ = env.call_method(
                receiver,
                "onRuntimeError",
                "(ILjava/lang/String;Ljava/lang/String;)V",
                &[
                    JValue::Int(kind),
                    JValue::Object(&jmessage.into()),
                    JValue::Object(&jsource),
                ],
            );
        });
    });
}

extern "C" fn ime_control(user_data: *mut c_void, op: i32, request_id: u64) {
    run_trampoline(user_data, |ctx| {
        upcall(ctx, 2, |env, receiver| {
            let _ = env.call_method(
                receiver,
                "onImeControl",
                "(IJ)V",
                &[JValue::Int(op), JValue::Long(request_id as i64)],
            );
        });
    });
}

extern "C" fn input_focus_changed(
    user_data: *mut c_void,
    focused: bool,
    info: *const JianFieldInfo,
) {
    run_trampoline(user_data, |ctx| {
        let (input_kind, return_key_hint) = match unsafe { info.as_ref() } {
            Some(info) => (info.input_kind as i32, info.return_key_hint as i32),
            None => (0, 0),
        };
        upcall(ctx, 2, |env, receiver| {
            let _ = env.call_method(
                receiver,
                "onInputFocusChanged",
                "(ZII)V",
                &[
                    JValue::Bool(focused as u8),
                    JValue::Int(input_kind),
                    JValue::Int(return_key_hint),
                ],
            );
        });
    });
}

extern "C" fn text_state_changed(user_data: *mut c_void) {
    run_trampoline(user_data, |ctx| {
        upcall(ctx, 1, |env, receiver| {
            let _ = env.call_method(receiver, "onTextStateChanged", "()V", &[]);
        });
    });
}

extern "C" fn capability_request(
    user_data: *mut c_void,
    request_id: u64,
    request: *const JianCapabilityRequest,
) {
    run_trampoline(user_data, |ctx| {
        let Some(request) = (unsafe { request.as_ref() }) else {
            return;
        };
        // The per-kind payload is marshalled to a JSON string plus optional
        // raw body bytes (HTTP fetch) — all owned, so no C pointer outlives
        // the call.
        let Some((payload_json, body)) = (unsafe { marshal::capability_request_to_json(request) })
        else {
            return;
        };
        upcall(ctx, 4, |env, receiver| {
            let Ok(jpayload) = env.new_string(&payload_json) else {
                return;
            };
            let jbody = match &body {
                Some(bytes) => match env.byte_array_from_slice(bytes) {
                    Ok(arr) => arr.into(),
                    Err(_) => return,
                },
                None => JObject::null(),
            };
            let _ = env.call_method(
                receiver,
                "onCapabilityRequest",
                "(JILjava/lang/String;[B)V",
                &[
                    JValue::Long(request_id as i64),
                    JValue::Int(request.kind as i32),
                    JValue::Object(&jpayload.into()),
                    JValue::Object(&jbody),
                ],
            );
        });
    });
}

extern "C" fn capability_cancelled(user_data: *mut c_void, request_id: u64) {
    run_trampoline(user_data, |ctx| {
        upcall(ctx, 1, |env, receiver| {
            let _ = env.call_method(
                receiver,
                "onCapabilityCancelled",
                "(J)V",
                &[JValue::Long(request_id as i64)],
            );
        });
    });
}

/// Copies a borrowed C string slice into an owned `String` (lossy — the
/// engine emits UTF-8, but a callback must never panic on malformed input).
///
/// # Safety
/// `ptr`/`len` must describe a readable byte range or `ptr` must be null.
unsafe fn borrowed_str(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    String::from_utf8_lossy(unsafe { slice::from_raw_parts(ptr, len) }).into_owned()
}
