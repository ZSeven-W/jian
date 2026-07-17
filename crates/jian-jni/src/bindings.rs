//! JNI natives for `dev.jian.player.JianNative` (Task 5 Step 4).
//!
//! Every native validates its `jlong` handle against the tombstoning
//! [`registry`](crate::registry) first (a closed/unknown handle returns
//! [`STATUS_CLOSING`]), then dispatches the engine work onto that engine's
//! dedicated thread via [`EngineThread`] — engine pointers are only ever
//! dereferenced there. The caller frame owns argument conversion; owned
//! results come back through the blocking barrier.

#![cfg(target_os = "android")]

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::{Arc, OnceLock};

use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString};
use jni::sys::{jfloat, jint, jlong};
use jni::{JNIEnv, JavaVM};

use jian_engine_ffi::{
    jian_attach_surface, jian_create, jian_destroy, jian_frame, jian_pointer, jian_register_font,
    jian_resize, jian_resume, jian_set_keyboard, jian_set_safe_area, jian_suspend, JianCreateDesc,
    JianEngine, JianStatus, JianSurfaceDesc,
};

use crate::callbacks::{build_callbacks, drop_ctx, EngineCtx};
use crate::registry::{Registry, HANDLE_FAILURE};
use crate::{EngineThread, STATUS_CLOSING};

/// The process `JavaVM`, captured in [`JNI_OnLoad`]. Every engine thread
/// attaches through it and every callback resolves its `JNIEnv` from it.
static VM: OnceLock<JavaVM> = OnceLock::new();

/// The process-global engine registry (handles → records).
fn registry() -> &'static Registry<EngineRecord> {
    static REGISTRY: OnceLock<Registry<EngineRecord>> = OnceLock::new();
    REGISTRY.get_or_init(Registry::new)
}

/// A raw engine pointer. Dereferenced ONLY on the owning engine thread; the
/// `Send` impl carries it across the dispatch barrier and the registry.
#[derive(Clone, Copy)]
struct EnginePtr(*mut JianEngine);
// SAFETY: the pointer is only ever used on the engine thread; the wrapper
// exists solely to move it through the queue and registry.
unsafe impl Send for EnginePtr {}

impl EnginePtr {
    /// Returns the raw pointer. Taking `self` by value makes a closure
    /// capture the whole (Send) wrapper, not the raw field (disjoint
    /// closure capture would otherwise grab the `!Send` pointer).
    fn get(self) -> *mut JianEngine {
        self.0
    }
}

/// The callback context pointer (freed once, in the destroy final job).
#[derive(Clone, Copy)]
struct CtxPtr(*mut EngineCtx);
// SAFETY: freed exactly once on the engine thread after jian_destroy.
unsafe impl Send for CtxPtr {}

impl CtxPtr {
    fn get(self) -> *mut EngineCtx {
        self.0
    }
}

/// Per-engine record held in the registry. The thread is behind an `Arc` so
/// a native can clone the dispatch handle out from under the registry lock
/// and release the lock BEFORE the blocking `call()` — otherwise a callback
/// re-entering a native would deadlock against the held registry mutex.
struct EngineRecord {
    thread: Arc<EngineThread>,
    engine: EnginePtr,
    ctx: CtxPtr,
}

/// Reconstructs an owned `JavaVM` handle from the captured one (JavaVM is not
/// `Clone`; the raw VM pointer is process-global and stable).
fn clone_vm(vm: &JavaVM) -> JavaVM {
    // SAFETY: the pointer came from a live JavaVM captured in JNI_OnLoad.
    unsafe { JavaVM::from_raw(vm.get_java_vm_pointer()) }.expect("valid JavaVM pointer")
}

/// Records the process `JavaVM` so engine threads can attach and callbacks
/// can resolve a `JNIEnv`. The JVM calls this exactly once at library load
/// with a valid `vm` (the raw pointer's validity is the JVM's contract, not
/// something a Rust caller could violate — hence the lint allowance).
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "system" fn JNI_OnLoad(vm: *mut jni::sys::JavaVM, _reserved: *mut c_void) -> jint {
    if let Ok(vm) = unsafe { JavaVM::from_raw(vm) } {
        let _ = VM.set(vm);
    }
    jni::sys::JNI_VERSION_1_6
}

/// `JianNative.nativeCreate` — spawns the engine thread, attaches it to the
/// VM, and creates the engine ON that thread. Returns the handle, or `0` on
/// failure (the reason is readable via `nativeLastError(0)`). The whole body
/// is guarded: `EngineThread::spawn` can panic (OS refuses the thread), and
/// that panic must never cross the non-unwinding `extern "system"` boundary.
#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeCreate<'local>(
    env: JNIEnv<'local>,
    class: JClass<'local>,
    doc: JByteArray<'local>,
    w: jfloat,
    h: jfloat,
    dpr: jfloat,
    storage_dir: JString<'local>,
    asset_base: JString<'local>,
    receiver: JObject<'local>,
) -> jlong {
    match catch_unwind(AssertUnwindSafe(|| {
        create_impl(
            env,
            class,
            doc,
            w,
            h,
            dpr,
            storage_dir,
            asset_base,
            receiver,
        )
    })) {
        Ok(handle) => handle,
        Err(payload) => {
            crate::engine_thread::drop_guarded(payload);
            registry().set_create_error("nativeCreate panicked");
            HANDLE_FAILURE
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn create_impl<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    doc: JByteArray<'local>,
    w: jfloat,
    h: jfloat,
    dpr: jfloat,
    storage_dir: JString<'local>,
    asset_base: JString<'local>,
    receiver: JObject<'local>,
) -> jlong {
    let Some(vm) = VM.get() else {
        registry().set_create_error("JavaVM not captured (JNI_OnLoad did not run)");
        return HANDLE_FAILURE;
    };

    let doc_bytes = match env.convert_byte_array(&doc) {
        Ok(bytes) => bytes,
        Err(_) => {
            registry().set_create_error("could not read document bytes");
            return HANDLE_FAILURE;
        }
    };
    let storage = jstring_to_string(&mut env, &storage_dir).unwrap_or_default();
    let asset = if asset_base.is_null() {
        None
    } else {
        jstring_to_string(&mut env, &asset_base)
    };
    let receiver: GlobalRef = match env.new_global_ref(&receiver) {
        Ok(r) => r,
        Err(_) => {
            registry().set_create_error("could not globalize the callback receiver");
            return HANDLE_FAILURE;
        }
    };

    // EngineCtx is Send (JavaVM + GlobalRef). It is kept HERE (on the
    // already-attached JNI caller thread) until attachment succeeds — if the
    // engine thread can never attach, dropping the ctx's GlobalRef there
    // would leak (DeleteGlobalRef needs an attached thread), so it is
    // disposed on the caller instead.
    let ctx = Box::new(EngineCtx::new(clone_vm(vm), receiver));
    let thread = Arc::new(EngineThread::spawn("jian-engine"));

    // Step 1: attach the engine thread to the VM (permanent — detaches at
    // thread exit) so its callbacks have a JNIEnv.
    let attach_vm = clone_vm(vm);
    let attached = thread.call(move || attach_vm.attach_current_thread_permanently().is_ok());
    if !matches!(attached, crate::Dispatch::Done(true)) {
        thread.close(|| {});
        drop(ctx); // dispose the GlobalRef HERE (attached caller thread)
        registry().set_create_error("engine thread could not attach to the JVM");
        return HANDLE_FAILURE;
    }

    // Step 2: build the callbacks table and create the engine, both on the
    // now-attached engine thread. The !Send JianCallbacks table never leaves
    // it; the ctx is moved in only now (its later disposal is on this
    // attached thread).
    let created = thread.call(move || {
        let (callbacks, ctx_ptr) = build_callbacks(ctx);
        let mut engine: *mut JianEngine = ptr::null_mut();
        let desc = JianCreateDesc {
            size: std::mem::size_of::<JianCreateDesc>(),
            doc_ptr: doc_bytes.as_ptr(),
            doc_len: doc_bytes.len(),
            width: w,
            height: h,
            dpr,
            storage_dir_ptr: storage.as_ptr(),
            storage_dir_len: storage.len(),
            callbacks: &callbacks,
            asset_base_ptr: asset.as_ref().map_or(ptr::null(), |a| a.as_ptr()),
            asset_base_len: asset.as_ref().map_or(0, |a| a.len()),
        };
        let status = unsafe { jian_create(&desc, &mut engine) };
        // `callbacks`/`doc_bytes`/`storage`/`asset` stay alive until here.
        (status as i32, engine as usize, ctx_ptr as usize)
    });

    let (status, engine_raw, ctx_raw) = match created {
        crate::Dispatch::Done(v) => v,
        crate::Dispatch::Closing => {
            thread.close(|| {});
            registry().set_create_error("engine thread closed during create");
            return HANDLE_FAILURE;
        }
    };

    if status != 0 || engine_raw == 0 {
        // Free the context on the engine thread (attached), then tear down.
        let ctx = CtxPtr(ctx_raw as *mut EngineCtx);
        thread.close(move || unsafe { drop_ctx(ctx.get()) });
        registry().set_create_error(format!("jian_create failed (status {status})"));
        return HANDLE_FAILURE;
    }

    registry().insert(EngineRecord {
        thread,
        engine: EnginePtr(engine_raw as *mut JianEngine),
        ctx: CtxPtr(ctx_raw as *mut EngineCtx),
    })
}

/// `JianNative.nativeLastError` — the last error text for a handle (or the
/// create-failure text for handle `0`); empty for an unknown handle.
#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeLastError<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
) -> jni::sys::jstring {
    let message = registry().last_error(engine);
    match env.new_string(message) {
        Ok(s) => s.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// `JianNative.nativeDestroy` — §6.7 teardown. Tombstones the handle, then
/// closes the engine thread with a final job that destroys the engine,
/// releases any attached window, and frees the callback context — strictly
/// last, on the engine thread. A callback-origin destroy (engine thread,
/// inside a callback frame) DEFERS via `close_deferred` per the no-re-entry
/// rule; otherwise it blocks on `close`.
#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeDestroy<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
) {
    let Some(record) = registry().take_for_close(engine) else {
        return; // unknown or already destroyed
    };
    let EngineRecord {
        thread,
        engine: engine_ptr,
        ctx,
    } = record;
    let final_job = move || {
        // A Poisoned destroy means an internal panic left the engine (and any
        // EGL surface borrowing the window) live: releasing the window would
        // dangle, so it is released ONLY after an Ok destroy that guarantees
        // teardown. The context is freed regardless — the thread is exiting,
        // so no further callback can read it.
        let status = unsafe { jian_destroy(engine_ptr.get()) };
        if matches!(status, JianStatus::Ok) {
            crate::window::take_current_window();
        }
        unsafe { drop_ctx(ctx.get()) };
    };
    if thread.is_engine_thread() && crate::engine_thread::in_callback_frame() {
        thread.close_deferred(final_job);
    } else {
        thread.close(final_job);
    }
}

/// Dispatches `f` onto the handle's engine thread and returns its owned
/// result. `None` when the handle is unknown/tombstoned (no dispatch) or the
/// queue is closing — callers map that to `STATUS_CLOSING` / `null`.
pub(crate) fn with_engine<R: Send + 'static>(
    handle: jlong,
    f: impl FnOnce(*mut JianEngine) -> R + Send + 'static,
) -> Option<R> {
    // Clone the dispatch handle + engine pointer under the lock, then RELEASE
    // it before the blocking call — the engine thread must never contend for
    // the registry mutex while a native waits on it (callback re-entry).
    let (thread, engine) = registry().with(handle, |rec| (rec.thread.clone(), rec.engine))?;
    // A panicking engine job is re-raised on THIS (caller) thread by call();
    // catch it at the dispatch boundary so it never crosses the non-unwinding
    // JNI ABI and aborts the process. A panicked call maps to `None` (→
    // STATUS_CLOSING / null), like a closed queue.
    let dispatched = catch_unwind(AssertUnwindSafe(|| thread.call(move || f(engine.get()))));
    match dispatched {
        Ok(crate::Dispatch::Done(r)) => Some(r),
        Ok(crate::Dispatch::Closing) => None,
        Err(payload) => {
            // Guarded disposal: a panic_any payload whose own Drop panics
            // would otherwise re-panic across the JNI ABI.
            crate::engine_thread::drop_guarded(payload);
            None
        }
    }
}

/// Dispatches an engine call returning a `JianStatus`, mapped to the `jint`
/// the Kotlin contract expects (unknown/closing → `STATUS_CLOSING`).
pub(crate) fn call_status(
    handle: jlong,
    f: impl FnOnce(*mut JianEngine) -> JianStatus + Send + 'static,
) -> jint {
    with_engine(handle, move |e| f(e) as jint).unwrap_or(STATUS_CLOSING)
}

/// Reads a (non-null) `JString` into an owned `String`.
fn jstring_to_string(env: &mut JNIEnv, s: &JString) -> Option<String> {
    env.get_string(s).ok().map(|s| s.into())
}

// ---- Lifecycle natives ---------------------------------------------------

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeAttachSurface<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    surface: JObject<'local>,
) -> jint {
    attach_or_resume(&mut env, engine, surface, false)
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeResume<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    surface: JObject<'local>,
) -> jint {
    // A null Surface resume is invalid (§6.2); a real Surface follows the
    // acquire-then-resume path identical to attach.
    if surface.is_null() {
        return STATUS_CLOSING;
    }
    attach_or_resume(&mut env, engine, surface, true)
}

/// Shared acquire → attach/resume → release-on-failure path (spec §6.7): the
/// caller frame globalizes the Surface; the window is acquired and released
/// ONLY on the engine thread.
fn attach_or_resume(env: &mut JNIEnv, engine: jlong, surface: JObject, resume: bool) -> jint {
    let Ok(surface) = env.new_global_ref(&surface) else {
        return STATUS_CLOSING;
    };
    with_engine(engine, move |e| {
        let Some(vm) = VM.get() else {
            return JianStatus::InvalidArg as jint;
        };
        let Ok(jenv) = vm.get_env() else {
            return JianStatus::InvalidArg as jint;
        };
        // SAFETY: on the engine thread with its env; surface is a live global.
        let window = unsafe { crate::window::acquire(&jenv, surface.as_obj()) };
        if window.is_null() {
            return JianStatus::InvalidArg as jint;
        }
        let desc = JianSurfaceDesc {
            size: std::mem::size_of::<JianSurfaceDesc>(),
            handle: window.cast::<c_void>(),
        };
        let status = if resume {
            unsafe { jian_resume(e, &desc) }
        } else {
            unsafe { jian_attach_surface(e, &desc) }
        };
        if matches!(status, JianStatus::Ok | JianStatus::Poisoned) {
            // Ok: the engine owns an EGL surface on this window. Poisoned: an
            // internal panic returned AFTER partial mutation, so the engine
            // MAY have installed the surface (borrowing this window) before
            // unwinding — never release a possibly-borrowed window. Both
            // retain it (releasing any previous), and destroy releases it.
            unsafe { crate::window::set_current_window(window) };
        } else {
            // A clean failure: Task 3's error-arm ordering guarantees no EGL
            // object survives, so the old surface (if any) still owns
            // CURRENT_WINDOW — leave it and release ONLY the fresh window.
            unsafe { crate::window::release(window) };
        }
        status as jint
    })
    .unwrap_or(STATUS_CLOSING)
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeSuspend<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
) -> jint {
    with_engine(engine, move |e| {
        let status = unsafe { jian_suspend(e) };
        // Release the window ONLY after a status that guarantees the engine
        // tore its EGL surface down synchronously (Ok). On a non-Ok status
        // (e.g. WrongThread re-entry) the surface may still borrow the
        // window, so it is kept until destroy/next-suspend releases it.
        if status as i32 == 0 {
            crate::window::take_current_window();
        }
        status as jint
    })
    .unwrap_or(STATUS_CLOSING)
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeResize<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    w: jfloat,
    h: jfloat,
    dpr: jfloat,
) -> jint {
    call_status(engine, move |e| unsafe { jian_resize(e, w, h, dpr) })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeSetSafeArea<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    t: jfloat,
    r: jfloat,
    b: jfloat,
    l: jfloat,
) -> jint {
    call_status(engine, move |e| unsafe {
        jian_set_safe_area(e, t, r, b, l)
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeSetKeyboard<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    h: jfloat,
) -> jint {
    call_status(engine, move |e| unsafe { jian_set_keyboard(e, h) })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeFrame<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    t_ms: jlong,
) -> jint {
    call_status(engine, move |e| {
        // Mark this scope so needs_redraw upcalls report fromFrame = true.
        let _origin = crate::callbacks::FrameOriginGuard::enter();
        unsafe { jian_frame(e, t_ms as u64) }
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_jian_player_JianNative_nativePointer<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    id: jint,
    phase: jint,
    x: jfloat,
    y: jfloat,
    t_ms: jlong,
) -> jint {
    call_status(engine, move |e| unsafe {
        jian_pointer(e, id as u32, phase, x, y, t_ms as u64)
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeRegisterFont<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    bytes: JByteArray<'local>,
) -> jint {
    let Ok(bytes) = env.convert_byte_array(&bytes) else {
        return STATUS_CLOSING;
    };
    call_status(engine, move |e| unsafe {
        jian_register_font(e, bytes.as_ptr(), bytes.len())
    })
}

// ---- Debug fault seams (debug-hooks builds only) -------------------------

/// Arms the next attach/resume to fail after the window/EGL acquisition
/// point (Task 3 Step 1b) — proving the caller's immediate-release path.
#[cfg(all(feature = "debug-hooks", debug_assertions))]
#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeDebugFailNextAttach<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
) -> jint {
    call_status(engine, move |e| unsafe {
        jian_engine_ffi::jian_test_force_attach_failure(e)
    })
}

/// Arms the next `draw_frame` to return `Err(ContextLost)` (Task 3 Step 1b).
#[cfg(all(feature = "debug-hooks", debug_assertions))]
#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeDebugLoseContext<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
) -> jint {
    call_status(engine, move |e| unsafe {
        jian_engine_ffi::jian_test_force_context_loss(e)
    })
}
