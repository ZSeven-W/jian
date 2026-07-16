use crate::capabilities::{JianCapabilityCancelled, JianCapabilityRequestCallback};
use crate::diagnostics::JianRuntimeErrorCallback;
use crate::error::{read_utf8, FfiError, FfiResult, DOCUMENT_CAP, STRING_CAP};
use crate::ime::{JianImeControl, JianInputFocusChanged, JianTextStateChanged};
use std::ffi::c_void;
use std::mem::{offset_of, size_of};
use std::ptr;

pub type JianNeedsRedraw =
    Option<unsafe extern "C" fn(user_data: *mut c_void, has_next_wake: bool, next_wake_ms: u64)>;

/// Callback table. Future callbacks grow only at the tail.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianCallbacks {
    pub size: usize,
    pub user_data: *mut c_void,
    pub needs_redraw: JianNeedsRedraw,
    pub runtime_error: JianRuntimeErrorCallback,
    pub ime_control: JianImeControl,
    pub input_focus_changed: JianInputFocusChanged,
    pub text_state_changed: JianTextStateChanged,
    pub capability_request: JianCapabilityRequestCallback,
    pub capability_cancelled: JianCapabilityCancelled,
}

/// Engine construction descriptor. `asset_base` is the v1 tail.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianCreateDesc {
    pub size: usize,
    pub doc_ptr: *const u8,
    pub doc_len: usize,
    pub width: f32,
    pub height: f32,
    pub dpr: f32,
    pub storage_dir_ptr: *const u8,
    pub storage_dir_len: usize,
    pub callbacks: *const JianCallbacks,
    pub asset_base_ptr: *const u8,
    pub asset_base_len: usize,
}

/// Platform surface descriptor. On iOS `handle` is a borrowed CAMetalLayer*.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianSurfaceDesc {
    pub size: usize,
    pub handle: *mut c_void,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JianPointerPhase {
    Down = 0,
    Move = 1,
    Up = 2,
    Cancel = 3,
}

/// Debug-only lifecycle categories used to exercise future suspended rows.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JianTestCallClass {
    TextContent = 0,
    ImeText = 1,
    CapabilityResult = 2,
    RegisterFont = 3,
    TextGeometry = 4,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Callbacks {
    pub user_data: *mut c_void,
    pub needs_redraw: JianNeedsRedraw,
    pub runtime_error: JianRuntimeErrorCallback,
    pub ime_control: JianImeControl,
    pub input_focus_changed: JianInputFocusChanged,
    pub text_state_changed: JianTextStateChanged,
    pub capability_request: JianCapabilityRequestCallback,
    pub capability_cancelled: JianCapabilityCancelled,
}

pub(crate) struct CreateOptions {
    pub document: String,
    pub width: f32,
    pub height: f32,
    pub dpr: f32,
    pub storage_dir: Option<String>,
    pub callbacks: Callbacks,
    pub asset_base: Option<String>,
}

unsafe fn read_covered<T: Copy>(base: *const u8, size: usize, offset: usize) -> Option<T> {
    let end = offset.checked_add(size_of::<T>())?;
    if end > size {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(base.add(offset).cast::<T>()) })
}

unsafe fn parse_callbacks(pointer: *const JianCallbacks) -> FfiResult<Callbacks> {
    if pointer.is_null() {
        return Ok(Callbacks::default());
    }
    let base = pointer.cast::<u8>();
    let size = unsafe { ptr::read_unaligned(pointer.cast::<usize>()) };
    if size < size_of::<usize>() {
        return Err(FfiError::invalid("callbacks size is below the minimum"));
    }
    if size > size_of::<JianCallbacks>() {
        return Err(FfiError::invalid(
            "callbacks size exceeds the known version",
        ));
    }
    Ok(Callbacks {
        user_data: unsafe {
            read_covered(base, size, offset_of!(JianCallbacks, user_data))
                .unwrap_or(ptr::null_mut())
        },
        needs_redraw: unsafe {
            read_covered(base, size, offset_of!(JianCallbacks, needs_redraw)).unwrap_or(None)
        },
        runtime_error: unsafe {
            read_covered(base, size, offset_of!(JianCallbacks, runtime_error)).unwrap_or(None)
        },
        ime_control: unsafe {
            read_covered(base, size, offset_of!(JianCallbacks, ime_control)).unwrap_or(None)
        },
        input_focus_changed: unsafe {
            read_covered(base, size, offset_of!(JianCallbacks, input_focus_changed)).unwrap_or(None)
        },
        text_state_changed: unsafe {
            read_covered(base, size, offset_of!(JianCallbacks, text_state_changed)).unwrap_or(None)
        },
        capability_request: unsafe {
            read_covered(base, size, offset_of!(JianCallbacks, capability_request)).unwrap_or(None)
        },
        capability_cancelled: unsafe {
            read_covered(base, size, offset_of!(JianCallbacks, capability_cancelled))
                .unwrap_or(None)
        },
    })
}

pub(crate) unsafe fn parse_create(pointer: *const JianCreateDesc) -> FfiResult<CreateOptions> {
    if pointer.is_null() {
        return Err(FfiError::invalid("create descriptor is null"));
    }
    let base = pointer.cast::<u8>();
    let size = unsafe { ptr::read_unaligned(pointer.cast::<usize>()) };
    let minimum = offset_of!(JianCreateDesc, dpr) + size_of::<f32>();
    if size < minimum {
        return Err(FfiError::invalid(
            "create descriptor size is below the required prefix",
        ));
    }
    if size > size_of::<JianCreateDesc>() {
        return Err(FfiError::invalid(
            "create descriptor size exceeds the known version",
        ));
    }

    macro_rules! required {
        ($field:ident, $ty:ty) => {
            unsafe {
                read_covered::<$ty>(base, size, offset_of!(JianCreateDesc, $field))
                    .ok_or_else(|| FfiError::invalid(concat!(stringify!($field), " is missing")))?
            }
        };
    }
    macro_rules! optional {
        ($field:ident, $ty:ty, $default:expr) => {
            unsafe {
                read_covered::<$ty>(base, size, offset_of!(JianCreateDesc, $field))
                    .unwrap_or($default)
            }
        };
    }

    let doc_ptr = required!(doc_ptr, *const u8);
    let doc_len = required!(doc_len, usize);
    let document = unsafe { read_utf8(doc_ptr, doc_len, DOCUMENT_CAP, "document") }?;
    let storage_ptr = optional!(storage_dir_ptr, *const u8, ptr::null());
    let storage_len = optional!(storage_dir_len, usize, 0);
    let storage_dir = if storage_ptr.is_null() && storage_len == 0 {
        None
    } else {
        Some(unsafe { read_utf8(storage_ptr, storage_len, STRING_CAP, "storage_dir") }?)
    };
    let callback_pointer = optional!(callbacks, *const JianCallbacks, ptr::null());
    let callbacks = unsafe { parse_callbacks(callback_pointer)? };
    let asset_ptr = optional!(asset_base_ptr, *const u8, ptr::null());
    let asset_len = optional!(asset_base_len, usize, 0);
    let asset_base = if asset_ptr.is_null() && asset_len == 0 {
        None
    } else {
        Some(unsafe { read_utf8(asset_ptr, asset_len, STRING_CAP, "asset_base") }?)
    };

    Ok(CreateOptions {
        document,
        width: required!(width, f32),
        height: required!(height, f32),
        dpr: required!(dpr, f32),
        storage_dir,
        callbacks,
        asset_base,
    })
}

pub(crate) unsafe fn surface_handle(pointer: *const JianSurfaceDesc) -> FfiResult<*mut c_void> {
    if pointer.is_null() {
        return Err(FfiError::invalid("surface descriptor is null"));
    }
    let size = unsafe { ptr::read_unaligned(pointer.cast::<usize>()) };
    if size != size_of::<JianSurfaceDesc>() {
        return Err(FfiError::invalid("surface descriptor size is invalid"));
    }
    let handle = unsafe { ptr::read_unaligned(ptr::addr_of!((*pointer).handle)) };
    if handle.is_null() {
        return Err(FfiError::invalid("surface handle is null"));
    }
    Ok(handle)
}
