//! Text / IME / capability-result JNI natives (Task 5 Step 4, part 2).
//!
//! Text content crosses the ABI as UTF-8 bytes; all offsets are UTF-16
//! code-unit offsets (the `jian.h` surrounding-text contract). Owned results
//! (`nativeTextGetState`, `nativeTextGetRange`, `nativeTextCaretRect`) come
//! back through the blocking barrier and the caller frame fills the Java
//! object / array. Dispatch and handle validation reuse
//! [`crate::bindings::with_engine`] / [`call_status`](crate::bindings::call_status).

#![cfg(target_os = "android")]

use std::ptr;

use jni::objects::{JByteArray, JClass, JFloatArray, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jlong};
use jni::JNIEnv;

use jian_engine_ffi::{
    jian_capability_result, jian_ime_cancel, jian_ime_commit, jian_ime_set_composing_region,
    jian_ime_set_composing_text, jian_text_batch_begin, jian_text_batch_end, jian_text_caret_rect,
    jian_text_get_range, jian_text_get_state, jian_text_insert, jian_text_replace_range,
    jian_text_set_selection, JianCapabilityResult, JianCapabilityResultData,
    JianClipboardReadResult, JianClipboardWriteResult, JianConfirmResult, JianEngine, JianHeader,
    JianHttpFetchResult, JianImageFetchResult, JianOpenUrlResult, JianRect, JianStatus,
    JianTextState,
};

use crate::bindings::{call_status, with_engine};
use crate::marshal;

/// Reads a (non-null) `JString` into owned UTF-8 bytes (empty on error/null).
fn jbytes(env: &mut JNIEnv, s: &JString) -> Vec<u8> {
    if s.is_null() {
        return Vec::new();
    }
    env.get_string(s)
        .map(|s| {
            let s: String = s.into();
            s.into_bytes()
        })
        .unwrap_or_default()
}

// ---- Status-returning text / IME natives ---------------------------------

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeTextInsert<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    text: JString<'local>,
) -> jint {
    let bytes = jbytes(&mut env, &text);
    call_status(engine, move |e| unsafe {
        jian_text_insert(e, bytes.as_ptr(), bytes.len())
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeTextReplaceRange<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    start: jint,
    end: jint,
    text: JString<'local>,
) -> jint {
    let bytes = jbytes(&mut env, &text);
    call_status(engine, move |e| unsafe {
        jian_text_replace_range(e, start as u32, end as u32, bytes.as_ptr(), bytes.len())
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeTextSetSelection<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    start: jint,
    end: jint,
) -> jint {
    call_status(engine, move |e| unsafe {
        jian_text_set_selection(e, start as u32, end as u32)
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeImeSetComposingRegion<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    start: jint,
    end: jint,
) -> jint {
    call_status(engine, move |e| unsafe {
        jian_ime_set_composing_region(e, start as u32, end as u32)
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeImeSetComposingText<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    text: JString<'local>,
    sel_start: jint,
    sel_end: jint,
) -> jint {
    let bytes = jbytes(&mut env, &text);
    call_status(engine, move |e| unsafe {
        jian_ime_set_composing_text(
            e,
            bytes.as_ptr(),
            bytes.len(),
            sel_start as u32,
            sel_end as u32,
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeImeCommit<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    text: JString<'local>,
    new_cursor_position: jint,
    request_id: jlong,
) -> jint {
    let bytes = jbytes(&mut env, &text);
    call_status(engine, move |e| unsafe {
        jian_ime_commit(
            e,
            bytes.as_ptr(),
            bytes.len(),
            new_cursor_position,
            request_id as u64,
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeImeCancel<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    request_id: jlong,
) -> jint {
    call_status(engine, move |e| unsafe {
        jian_ime_cancel(e, request_id as u64)
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeTextBatchBegin<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
) -> jint {
    call_status(engine, move |e| unsafe { jian_text_batch_begin(e) })
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeTextBatchEnd<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
) -> jint {
    call_status(engine, move |e| unsafe { jian_text_batch_end(e) })
}

// ---- Owned-result natives ------------------------------------------------

/// Owned snapshot of the surrounding-text state (the C `text_ptr` is only
/// valid until the next call, so it is copied here on the engine thread).
struct OwnedTextState {
    status: jint,
    text: String,
    window_start: i32,
    selection_start: i32,
    selection_end: i32,
    has_composing: bool,
    composing_start: i32,
    composing_end: i32,
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeTextGetState<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    out: JObject<'local>,
) -> jint {
    let Some(state) = with_engine(engine, move |e| {
        let mut c = JianTextState {
            size: std::mem::size_of::<JianTextState>(),
            text_ptr: ptr::null(),
            text_len: 0,
            window_start: 0,
            selection_start: 0,
            selection_end: 0,
            has_composing: false,
            composing_start: 0,
            composing_end: 0,
        };
        let status = unsafe { jian_text_get_state(e, &mut c) };
        let text = if c.text_ptr.is_null() || c.text_len == 0 {
            String::new()
        } else {
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(c.text_ptr, c.text_len) })
                .into_owned()
        };
        OwnedTextState {
            status: status as jint,
            text,
            window_start: c.window_start as i32,
            selection_start: c.selection_start as i32,
            selection_end: c.selection_end as i32,
            has_composing: c.has_composing,
            composing_start: c.composing_start as i32,
            composing_end: c.composing_end as i32,
        }
    }) else {
        return crate::STATUS_CLOSING;
    };

    // Fill the caller-provided JianTextState Java object.
    if let Ok(jtext) = env.new_string(&state.text) {
        let _ = env.set_field(
            &out,
            "text",
            "Ljava/lang/String;",
            JValue::Object(&jtext.into()),
        );
    }
    let _ = env.set_field(&out, "windowStart", "I", JValue::Int(state.window_start));
    let _ = env.set_field(
        &out,
        "selectionStart",
        "I",
        JValue::Int(state.selection_start),
    );
    let _ = env.set_field(&out, "selectionEnd", "I", JValue::Int(state.selection_end));
    let _ = env.set_field(
        &out,
        "hasComposing",
        "Z",
        JValue::Bool(state.has_composing as jboolean),
    );
    let _ = env.set_field(
        &out,
        "composingStart",
        "I",
        JValue::Int(state.composing_start),
    );
    let _ = env.set_field(&out, "composingEnd", "I", JValue::Int(state.composing_end));
    state.status
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeTextGetRange<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    start: jint,
    end: jint,
) -> jni::sys::jstring {
    // On the engine thread: size, allocate, fill (two-pass), UTF-8 → owned.
    let text = with_engine(engine, move |e| {
        let (start, end) = (start as u32, end as u32);
        let mut required: usize = 0;
        let status =
            unsafe { jian_text_get_range(e, start, end, ptr::null_mut(), 0, &mut required) };
        if status as i32 != 0 {
            return None;
        }
        let mut buffer = vec![0u8; required];
        let mut written = required;
        let status = unsafe {
            jian_text_get_range(e, start, end, buffer.as_mut_ptr(), required, &mut written)
        };
        if status as i32 != 0 {
            return None;
        }
        buffer.truncate(written.min(buffer.len()));
        Some(String::from_utf8_lossy(&buffer).into_owned())
    })
    .flatten();

    match text {
        Some(text) => match env.new_string(text) {
            Ok(s) => s.into_raw(),
            Err(_) => ptr::null_mut(),
        },
        // null = NoFocus / NotReady / Closing (the contract; the status is
        // available via nativeLastError only on real errors).
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeTextCaretRect<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    out: JFloatArray<'local>,
) -> jint {
    let Some((status, rect)) = with_engine(engine, move |e| {
        let mut rect = JianRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
        let status = unsafe { jian_text_caret_rect(e, &mut rect) };
        (status as jint, [rect.x, rect.y, rect.width, rect.height])
    }) else {
        return crate::STATUS_CLOSING;
    };
    let _ = env.set_float_array_region(&out, 0, &rect);
    status
}

// ---- Capability result ---------------------------------------------------

/// `JianNative.nativeCapabilityResult` — routes the Java result back through
/// the C `JianCapabilityResult` union by `kind` and delivers it to the
/// engine. Owned buffers (headers, body, text, error) are built on the
/// caller frame and kept alive across the (blocking) engine call.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_jian_player_JianNative_nativeCapabilityResult<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    engine: jlong,
    request_id: jlong,
    kind: jint,
    ok: jboolean,
    http_status: jint,
    headers_json: JString<'local>,
    bytes: JByteArray<'local>,
    bool_value: jboolean,
    error: JString<'local>,
) -> jint {
    let ok = ok != 0;
    let bool_value = bool_value != 0;
    let body = if bytes.is_null() {
        Vec::new()
    } else {
        env.convert_byte_array(&bytes).unwrap_or_default()
    };
    let error: Option<String> = if error.is_null() {
        None
    } else {
        env.get_string(&error).ok().map(|s| s.into())
    };
    let headers_pairs = if headers_json.is_null() {
        Vec::new()
    } else {
        parse_flat_headers(&jstring(&mut env, &headers_json))
    };

    with_engine(engine, move |e| {
        deliver_capability(
            e,
            request_id as u64,
            kind,
            ok,
            http_status,
            bool_value,
            &headers_pairs,
            &body,
            error.as_deref(),
        ) as jint
    })
    .unwrap_or(crate::STATUS_CLOSING)
}

/// Reads a (non-null) `JString` into an owned `String`.
fn jstring(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(|s| s.into()).unwrap_or_default()
}

/// Parses a flat `{"name":"value",…}` JSON object into header pairs; a
/// malformed payload yields no headers (best-effort — never panics).
fn parse_flat_headers(json: &str) -> Vec<(String, String)> {
    match serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json) {
        Ok(map) => map
            .into_iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_owned())))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Builds the per-kind `JianCapabilityResult` and hands it to the engine.
/// All pointer fields borrow the caller's owned buffers, which outlive this
/// (blocking) call.
#[allow(clippy::too_many_arguments)]
fn deliver_capability(
    engine: *mut JianEngine,
    request_id: u64,
    kind: jint,
    ok: bool,
    http_status: jint,
    bool_value: bool,
    headers: &[(String, String)],
    body: &[u8],
    error: Option<&str>,
) -> JianStatus {
    let error_ptr = error.map_or(ptr::null(), |e| e.as_ptr());
    let error_len = error.map_or(0, |e| e.len());

    let data = match kind {
        marshal::KIND_HTTP_FETCH => {
            let c_headers: Vec<JianHeader> = headers
                .iter()
                .map(|(n, v)| JianHeader {
                    name_ptr: n.as_ptr(),
                    name_len: n.len(),
                    value_ptr: v.as_ptr(),
                    value_len: v.len(),
                })
                .collect();
            let data = JianCapabilityResultData {
                http_fetch: JianHttpFetchResult {
                    ok,
                    status: http_status as u16,
                    headers: c_headers.as_ptr(),
                    headers_len: c_headers.len(),
                    body_ptr: body.as_ptr(),
                    body_len: body.len(),
                    error_ptr,
                    error_len,
                },
            };
            // c_headers must outlive the call — do it inline here.
            return call_result(engine, request_id, kind, data);
        }
        marshal::KIND_CONFIRM => JianCapabilityResultData {
            confirm: JianConfirmResult { value: bool_value },
        },
        marshal::KIND_IMAGE_FETCH => JianCapabilityResultData {
            image_fetch: JianImageFetchResult {
                ok,
                bytes_ptr: body.as_ptr(),
                bytes_len: body.len(),
                error_ptr,
                error_len,
            },
        },
        marshal::KIND_CLIPBOARD_READ => JianCapabilityResultData {
            clipboard_read: JianClipboardReadResult {
                ok,
                text_ptr: body.as_ptr(),
                text_len: body.len(),
                error_ptr,
                error_len,
            },
        },
        marshal::KIND_CLIPBOARD_WRITE => JianCapabilityResultData {
            clipboard_write: JianClipboardWriteResult {
                ok,
                error_ptr,
                error_len,
            },
        },
        marshal::KIND_OPEN_URL => JianCapabilityResultData {
            open_url: JianOpenUrlResult {
                ok,
                error_ptr,
                error_len,
            },
        },
        // An unknown kind never reaches the engine: rejecting here avoids
        // relying on callee discriminant validation to keep the union sound.
        _ => return JianStatus::InvalidArg,
    };
    call_result(engine, request_id, kind, data)
}

fn call_result(
    engine: *mut JianEngine,
    request_id: u64,
    kind: jint,
    data: JianCapabilityResultData,
) -> JianStatus {
    let result = JianCapabilityResult {
        size: std::mem::size_of::<JianCapabilityResult>(),
        kind,
        data,
    };
    unsafe { jian_capability_result(engine, request_id, &result) }
}
