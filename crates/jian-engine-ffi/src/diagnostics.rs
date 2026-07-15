use crate::JianStatus;
use jian_core::runtime::Runtime;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JianRuntimeErrorKind {
    Layout = 0,
    Action = 1,
    Internal = 2,
    Warning = 3,
}

/// Borrowed diagnostic payload, valid only for the callback duration.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianRuntimeError {
    pub size: usize,
    pub kind: JianRuntimeErrorKind,
    pub message_ptr: *const u8,
    pub message_len: usize,
    pub source_ptr: *const u8,
    pub source_len: usize,
}

pub type JianRuntimeErrorCallback =
    unsafe extern "C" fn(user_data: *mut c_void, error: *const JianRuntimeError);

/// Task 5 consumes this callback; Task 4 needs the tail field to report when it is absent.
pub type JianImeControl = unsafe extern "C" fn(user_data: *mut c_void, op: i32, request_id: u64);

pub(crate) fn emit(
    callback: Option<JianRuntimeErrorCallback>,
    user_data: *mut c_void,
    kind: JianRuntimeErrorKind,
    message: &str,
    source: Option<&str>,
) {
    let Some(callback) = callback else {
        return;
    };
    let (source_ptr, source_len) = source
        .map(|value| (value.as_ptr(), value.len()))
        .unwrap_or((ptr::null(), 0));
    let error = JianRuntimeError {
        size: size_of::<JianRuntimeError>(),
        kind,
        message_ptr: message.as_ptr(),
        message_len: message.len(),
        source_ptr,
        source_len,
    };
    unsafe { callback(user_data, &error) };
}

pub(crate) fn drain_runtime(
    runtime: &mut Runtime,
    callback: Option<JianRuntimeErrorCallback>,
    user_data: *mut c_void,
) {
    for warning in runtime.take_load_warnings() {
        emit(
            callback,
            user_data,
            JianRuntimeErrorKind::Warning,
            &warning,
            Some("document"),
        );
    }
    for error in runtime.take_layout_errors() {
        emit(
            callback,
            user_data,
            JianRuntimeErrorKind::Layout,
            &error,
            Some("runtime"),
        );
    }
    for reported in runtime.take_action_outcomes() {
        for warning in reported.outcome.warnings {
            emit(
                callback,
                user_data,
                JianRuntimeErrorKind::Warning,
                &warning.message,
                reported.source.as_deref(),
            );
        }
        if let Err(error) = reported.outcome.result {
            emit(
                callback,
                user_data,
                JianRuntimeErrorKind::Action,
                &error.to_string(),
                reported.source.as_deref(),
            );
        }
    }
}

pub(crate) fn emit_call_error(
    callback: Option<JianRuntimeErrorCallback>,
    user_data: *mut c_void,
    status: JianStatus,
    message: &str,
) {
    if status == JianStatus::LayoutError {
        emit(
            callback,
            user_data,
            JianRuntimeErrorKind::Layout,
            message,
            None,
        );
    }
}
