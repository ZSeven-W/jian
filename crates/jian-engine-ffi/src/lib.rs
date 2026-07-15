//! C ABI boundary for native Jian Player shells.

mod desc;
mod diagnostics;
mod error;
mod input;
mod lifecycle;
mod render;
mod status;
mod viewport;

pub use desc::{
    JianCallbacks, JianCreateDesc, JianPointerPhase, JianSurfaceDesc, JianTestCallClass,
};
pub use diagnostics::{
    JianImeControl, JianRuntimeError, JianRuntimeErrorCallback, JianRuntimeErrorKind,
};
pub use input::jian_pointer;
#[cfg(debug_assertions)]
pub use input::jian_test_app_number;
pub use lifecycle::JianEngine;
pub use status::JianStatus;
pub use viewport::{jian_set_keyboard, jian_set_safe_area, JianInsets, JianRect};
#[cfg(debug_assertions)]
pub use viewport::{jian_test_node_rect, jian_test_viewport_number};

use crate::desc::{parse_create, surface_handle};
use crate::error::{
    clear_create_error, create_error, set_create_error, write_bytes, FfiError, FfiResult,
};
use crate::lifecycle::{call_engine, destroy_engine, Lifecycle};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

/// Create an engine from a versioned descriptor.
///
/// # Safety
///
/// `desc` must expose its declared prefix and `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn jian_create(
    desc: *const JianCreateDesc,
    out: *mut *mut JianEngine,
) -> JianStatus {
    clear_create_error();
    if out.is_null() {
        set_create_error("engine output pointer is null");
        return JianStatus::InvalidArg;
    }
    unsafe { out.write(ptr::null_mut()) };
    let outcome = catch_unwind(AssertUnwindSafe(|| -> FfiResult<*mut JianEngine> {
        let options = unsafe { parse_create(desc) }?;
        let lifecycle = Lifecycle::new(options)?;
        Ok(Box::into_raw(Box::new(JianEngine::new(lifecycle))))
    }));
    match outcome {
        Ok(Ok(engine)) => {
            unsafe { out.write(engine) };
            JianStatus::Ok
        }
        Ok(Err(error)) => {
            set_create_error(error.message);
            error.status
        }
        Err(_) => {
            set_create_error("panic while creating the Jian engine");
            JianStatus::Poisoned
        }
    }
}

/// Destroy an engine on its owner thread.
///
/// # Safety
///
/// `engine` must be live, returned by `jian_create`, and not yet destroyed.
#[no_mangle]
pub unsafe extern "C" fn jian_destroy(engine: *mut JianEngine) -> JianStatus {
    unsafe { destroy_engine(engine) }
}

/// Copy the last error into a caller-owned byte buffer.
///
/// # Safety
///
/// A non-null `engine` must be live; output pointers must cover their sizes.
#[no_mangle]
pub unsafe extern "C" fn jian_last_error(
    engine: *mut JianEngine,
    buffer: *mut u8,
    length: usize,
    required: *mut usize,
) -> JianStatus {
    if engine.is_null() {
        return match catch_unwind(AssertUnwindSafe(|| unsafe {
            write_bytes(create_error().as_bytes(), buffer, length, required)
        })) {
            Ok(Ok(())) => JianStatus::Ok,
            Ok(Err(error)) => error.status,
            Err(_) => JianStatus::Poisoned,
        };
    }
    unsafe {
        call_engine(engine, |_| {
            let value = (&*engine).error();
            write_bytes(value.as_bytes(), buffer, length, required)
        })
    }
}

/// Select GPU mode using a borrowed platform surface.
///
/// # Safety
///
/// Pointers must be live and the surface must outlive attach through suspend.
#[no_mangle]
pub unsafe extern "C" fn jian_attach_surface(
    engine: *mut JianEngine,
    desc: *const JianSurfaceDesc,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let handle = surface_handle(desc)?;
            lifecycle.attach_surface(handle)
        })
    }
}

/// Suspend rendering and synchronously stop using a borrowed surface.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_suspend(engine: *mut JianEngine) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            lifecycle.suspend();
            Ok(())
        })
    }
}

/// Resume rendering, optionally with a new borrowed GPU surface.
///
/// # Safety
///
/// `engine` must be live; a non-null descriptor must be readable.
#[no_mangle]
pub unsafe extern "C" fn jian_resume(
    engine: *mut JianEngine,
    desc: *const JianSurfaceDesc,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            let handle = if desc.is_null() {
                None
            } else {
                Some(surface_handle(desc)?)
            };
            lifecycle.resume(handle)
        })
    }
}

/// Pump and present one GPU frame.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_frame(engine: *mut JianEngine, now_ms: u64) -> JianStatus {
    unsafe { call_engine(engine, |lifecycle| lifecycle.frame_gpu(now_ms)) }
}

/// Pump and copy one CPU frame into caller-owned RGBA8888 storage.
///
/// # Safety
///
/// `engine` must be live and `buffer` must cover `buffer_len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn jian_frame_cpu(
    engine: *mut JianEngine,
    now_ms: u64,
    buffer: *mut u8,
    buffer_len: usize,
    stride: usize,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            lifecycle.frame_cpu(now_ms, buffer, buffer_len, stride)
        })
    }
}

/// Change the logical viewport and device-pixel ratio.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_resize(
    engine: *mut JianEngine,
    width: f32,
    height: f32,
    dpr: f32,
) -> JianStatus {
    unsafe { call_engine(engine, |lifecycle| lifecycle.resize(width, height, dpr)) }
}

/// Return the current physical pixel dimensions.
///
/// # Safety
///
/// `engine` must be live and both output pointers must be writable.
#[no_mangle]
pub unsafe extern "C" fn jian_get_pixel_size(
    engine: *mut JianEngine,
    width: *mut u32,
    height: *mut u32,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if width.is_null() || height.is_null() {
                return Err(FfiError::invalid("pixel-size output pointer is null"));
            }
            let value = lifecycle.pixel_size();
            width.write(value.0);
            height.write(value.1);
            Ok(())
        })
    }
}

/// Debug-build panic hook used only to prove catch_unwind poisoning.
#[cfg(debug_assertions)]
#[doc(hidden)]
#[no_mangle]
pub unsafe extern "C" fn jian_test_force_panic(engine: *mut JianEngine) -> JianStatus {
    unsafe {
        call_engine(engine, |_| -> FfiResult<()> {
            panic!("intentional FFI poison test")
        })
    }
}

/// Debug-build probe for lifecycle rows whose public ABI lands in Tasks 4-6.
#[cfg(debug_assertions)]
#[doc(hidden)]
#[no_mangle]
pub unsafe extern "C" fn jian_test_suspended_status(
    engine: *mut JianEngine,
    class: JianTestCallClass,
) -> JianStatus {
    let mut value = JianStatus::Poisoned;
    let status = unsafe {
        call_engine(engine, |lifecycle| {
            value = lifecycle.test_suspended_status(class);
            Ok(())
        })
    };
    if status == JianStatus::Ok {
        value
    } else {
        status
    }
}

#[cfg(debug_assertions)]
#[doc(hidden)]
#[no_mangle]
pub unsafe extern "C" fn jian_test_get_insets(
    engine: *mut JianEngine,
    insets: *mut JianInsets,
    keyboard: *mut f32,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if insets.is_null() || keyboard.is_null() {
                return Err(FfiError::invalid("inset probe output pointer is null"));
            }
            let values = lifecycle.insets();
            insets.write(values.0);
            keyboard.write(values.1);
            Ok(())
        })
    }
}
