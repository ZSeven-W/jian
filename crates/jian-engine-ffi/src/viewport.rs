#[cfg(debug_assertions)]
use crate::error::{read_utf8, FfiError};
use crate::lifecycle::call_engine;
use crate::{JianEngine, JianStatus};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JianInsets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JianRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Update the four logical safe-area insets.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_set_safe_area(
    engine: *mut JianEngine,
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            lifecycle.set_safe_area(JianInsets {
                top,
                right,
                bottom,
                left,
            })
        })
    }
}

/// Update the logical keyboard occlusion height.
///
/// # Safety
///
/// `engine` must be live and called on its owner thread.
#[no_mangle]
pub unsafe extern "C" fn jian_set_keyboard(engine: *mut JianEngine, height: f32) -> JianStatus {
    unsafe { call_engine(engine, |lifecycle| lifecycle.set_keyboard(height)) }
}

#[cfg(debug_assertions)]
#[doc(hidden)]
#[no_mangle]
pub unsafe extern "C" fn jian_test_node_rect(
    engine: *mut JianEngine,
    id_ptr: *const u8,
    id_len: usize,
    output: *mut JianRect,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if output.is_null() {
                return Err(FfiError::invalid("node-rect output pointer is null"));
            }
            let id = read_utf8(id_ptr, id_len, crate::error::STRING_CAP, "node id")?;
            let rect = lifecycle
                .node_rect(&id)
                .ok_or_else(|| FfiError::invalid("node has no layout rectangle"))?;
            output.write(rect);
            Ok(())
        })
    }
}

#[cfg(debug_assertions)]
#[doc(hidden)]
#[no_mangle]
pub unsafe extern "C" fn jian_test_viewport_number(
    engine: *mut JianEngine,
    key_ptr: *const u8,
    key_len: usize,
    value: *mut f64,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if value.is_null() {
                return Err(FfiError::invalid("viewport-number output pointer is null"));
            }
            let key = read_utf8(key_ptr, key_len, crate::error::STRING_CAP, "viewport key")?;
            let number = lifecycle
                .viewport_number(&key)
                .ok_or_else(|| FfiError::invalid("viewport state is not numeric"))?;
            value.write(number);
            Ok(())
        })
    }
}
