//! C ABI ↔ Java marshalling helpers (Task 5 Step 4).
//!
//! The capability-request payload is serialized to a JSON string (plus, for
//! an HTTP fetch, the raw request body as separate bytes) so the Kotlin
//! `JianCapabilities` can decode it without a bespoke JNI struct. The JSON
//! SCHEMA is defined here and MUST match the Kotlin decoder:
//!
//! | kind | JSON object |
//! |------|-------------|
//! | 0 HttpFetch      | `{"method","url","headers":[[n,v]…],"hasBody","timeoutMs":<u64>\|null}` (body → bytes) |
//! | 1 Confirm        | `{"title","message"}` |
//! | 2 ClipboardRead  | `{}` |
//! | 3 ClipboardWrite | `{"text"}` |
//! | 4 ImageFetch     | `{"url"}` |
//! | 5 OpenUrl        | `{"url"}` |
//!
//! The pure builders below are host-tested; the `#[cfg(target_os =
//! "android")]` union reader is a thin unsafe wrapper over them.

/// Capability-kind discriminants (mirror `JianCapabilityKind` in jian.h).
pub const KIND_HTTP_FETCH: i32 = 0;
pub const KIND_CONFIRM: i32 = 1;
pub const KIND_CLIPBOARD_READ: i32 = 2;
pub const KIND_CLIPBOARD_WRITE: i32 = 3;
pub const KIND_IMAGE_FETCH: i32 = 4;
pub const KIND_OPEN_URL: i32 = 5;

/// Appends a JSON-escaped, double-quoted string to `out`.
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_field(out: &mut String, first: &mut bool, key: &str, value: &str) {
    if !*first {
        out.push(',');
    }
    *first = false;
    push_json_string(out, key);
    out.push(':');
    push_json_string(out, value);
}

/// `{"method","url","headers":[[n,v]…],"hasBody","timeoutMs":…}`.
pub fn http_fetch_json(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    has_body: bool,
    timeout_ms: Option<u64>,
) -> String {
    let mut out = String::from("{");
    let mut first = true;
    push_field(&mut out, &mut first, "method", method);
    push_field(&mut out, &mut first, "url", url);
    out.push_str(",\"headers\":[");
    for (i, (name, value)) in headers.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('[');
        push_json_string(&mut out, name);
        out.push(',');
        push_json_string(&mut out, value);
        out.push(']');
    }
    out.push(']');
    out.push_str(",\"hasBody\":");
    out.push_str(if has_body { "true" } else { "false" });
    out.push_str(",\"timeoutMs\":");
    match timeout_ms {
        Some(ms) => out.push_str(&ms.to_string()),
        None => out.push_str("null"),
    }
    out.push('}');
    out
}

pub fn confirm_json(title: &str, message: &str) -> String {
    let mut out = String::from("{");
    let mut first = true;
    push_field(&mut out, &mut first, "title", title);
    push_field(&mut out, &mut first, "message", message);
    out.push('}');
    out
}

pub fn single_field_json(key: &str, value: &str) -> String {
    let mut out = String::from("{");
    let mut first = true;
    push_field(&mut out, &mut first, key, value);
    out.push('}');
    out
}

/// Serializes a borrowed C capability request to `(payload_json, body)`.
/// `body` is `Some` only for an HTTP fetch that carries a request body.
/// Returns `None` for an unknown kind.
///
/// # Safety
/// `request` must be a live `JianCapabilityRequest`; the union member named
/// by `kind` must be initialized with valid pointer/length pairs.
#[cfg(target_os = "android")]
pub unsafe fn capability_request_to_json(
    request: &jian_engine_ffi::JianCapabilityRequest,
) -> Option<(String, Option<Vec<u8>>)> {
    use std::slice;

    // SAFETY: `ptr`/`len` describe a readable range or `ptr` is null.
    unsafe fn owned(ptr: *const u8, len: usize) -> String {
        if ptr.is_null() || len == 0 {
            return String::new();
        }
        String::from_utf8_lossy(unsafe { slice::from_raw_parts(ptr, len) }).into_owned()
    }

    match request.kind as i32 {
        KIND_HTTP_FETCH => {
            let r = unsafe { &request.data.http_fetch };
            let method = unsafe { owned(r.method_ptr, r.method_len) };
            let url = unsafe { owned(r.url_ptr, r.url_len) };
            let mut headers = Vec::with_capacity(r.headers_len);
            if !r.headers.is_null() {
                let slice = unsafe { slice::from_raw_parts(r.headers, r.headers_len) };
                for h in slice {
                    headers.push((unsafe { owned(h.name_ptr, h.name_len) }, unsafe {
                        owned(h.value_ptr, h.value_len)
                    }));
                }
            }
            let timeout = if r.has_timeout {
                Some(r.timeout_ms)
            } else {
                None
            };
            let body = if r.has_body && !r.body_ptr.is_null() {
                Some(unsafe { slice::from_raw_parts(r.body_ptr, r.body_len) }.to_vec())
            } else {
                None
            };
            Some((
                http_fetch_json(&method, &url, &headers, r.has_body, timeout),
                body,
            ))
        }
        KIND_CONFIRM => {
            let r = unsafe { &request.data.confirm };
            let title = unsafe { owned(r.title_ptr, r.title_len) };
            let message = unsafe { owned(r.message_ptr, r.message_len) };
            Some((confirm_json(&title, &message), None))
        }
        KIND_CLIPBOARD_READ => Some((String::from("{}"), None)),
        KIND_CLIPBOARD_WRITE => {
            let r = unsafe { &request.data.clipboard_write };
            let text = unsafe { owned(r.text_ptr, r.text_len) };
            Some((single_field_json("text", &text), None))
        }
        KIND_IMAGE_FETCH => {
            let r = unsafe { &request.data.image_fetch };
            let url = unsafe { owned(r.url_ptr, r.url_len) };
            Some((single_field_json("url", &url), None))
        }
        KIND_OPEN_URL => {
            let r = unsafe { &request.data.open_url };
            let url = unsafe { owned(r.url_ptr, r.url_len) };
            Some((single_field_json("url", &url), None))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quotes_backslashes_and_controls() {
        let mut out = String::new();
        push_json_string(&mut out, "a\"b\\c\nd\te\u{01}");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\te\\u0001\"");
    }

    #[test]
    fn http_fetch_json_shape() {
        let headers = vec![
            ("Accept".to_string(), "application/json".to_string()),
            ("X-A".to_string(), "1".to_string()),
        ];
        let json = http_fetch_json("POST", "http://h/ok", &headers, true, Some(30000));
        assert_eq!(
            json,
            r#"{"method":"POST","url":"http://h/ok","headers":[["Accept","application/json"],["X-A","1"]],"hasBody":true,"timeoutMs":30000}"#
        );
    }

    #[test]
    fn http_fetch_json_null_timeout_no_headers() {
        let json = http_fetch_json("GET", "http://h/x", &[], false, None);
        assert_eq!(
            json,
            r#"{"method":"GET","url":"http://h/x","headers":[],"hasBody":false,"timeoutMs":null}"#
        );
    }

    #[test]
    fn confirm_and_single_field() {
        assert_eq!(
            confirm_json("Title", "Msg"),
            r#"{"title":"Title","message":"Msg"}"#
        );
        assert_eq!(
            single_field_json("url", "http://x"),
            r#"{"url":"http://x"}"#
        );
    }
}
