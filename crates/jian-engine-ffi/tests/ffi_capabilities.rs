use jian_engine_ffi::{
    jian_capability_result, jian_create, jian_destroy, jian_frame_cpu, jian_last_error,
    jian_pointer, jian_register_font, jian_test_app_number, jian_test_reload, JianCallbacks,
    JianCapabilityKind, JianCapabilityRequest, JianCapabilityResult, JianCapabilityResultData,
    JianConfirmResult, JianCreateDesc, JianEngine, JianHttpFetchResult, JianImageFetchResult,
    JianOpenUrlResult, JianPointerPhase, JianRuntimeError, JianRuntimeErrorKind, JianStatus,
};
#[cfg(feature = "textlayout")]
use jian_engine_ffi::{
    jian_ime_cancel, jian_ime_set_composing_text, jian_resize, jian_test_font_generation,
    jian_test_variant_build_count,
};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::Mutex;

#[derive(Debug)]
struct RequestRecord {
    id: u64,
    kind: JianCapabilityKind,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    timeout: Option<u64>,
}

#[derive(Debug)]
struct ErrorRecord {
    kind: JianRuntimeErrorKind,
    message: String,
}

#[derive(Default)]
struct CallbackLog {
    requests: Mutex<Vec<RequestRecord>>,
    errors: Mutex<Vec<ErrorRecord>>,
    cancelled: Mutex<Vec<u64>>,
    ime_requests: Mutex<Vec<u64>>,
}

unsafe fn copied(pointer: *const u8, length: usize) -> Vec<u8> {
    if length == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(pointer, length) }.to_vec()
    }
}

unsafe extern "C" fn capability_request(
    user_data: *mut c_void,
    request_id: u64,
    request: *const JianCapabilityRequest,
) {
    let log = unsafe { &*user_data.cast::<CallbackLog>() };
    let request = unsafe { &*request };
    let mut record = RequestRecord {
        id: request_id,
        kind: request.kind,
        method: String::new(),
        url: String::new(),
        headers: Vec::new(),
        body: None,
        timeout: None,
    };
    if request.kind == JianCapabilityKind::HttpFetch {
        let http = unsafe { request.data.http_fetch };
        record.method = String::from_utf8(unsafe { copied(http.method_ptr, http.method_len) })
            .expect("method is UTF-8");
        record.url =
            String::from_utf8(unsafe { copied(http.url_ptr, http.url_len) }).expect("URL is UTF-8");
        let headers = if http.headers_len == 0 {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(http.headers, http.headers_len) }
        };
        record.headers = headers
            .iter()
            .map(|header| {
                let name =
                    String::from_utf8(unsafe { copied(header.name_ptr, header.name_len) }).unwrap();
                let value =
                    String::from_utf8(unsafe { copied(header.value_ptr, header.value_len) })
                        .unwrap();
                (name, value)
            })
            .collect();
        record.body = http
            .has_body
            .then(|| unsafe { copied(http.body_ptr, http.body_len) });
        record.timeout = http.has_timeout.then_some(http.timeout_ms);
    } else if request.kind == JianCapabilityKind::ImageFetch {
        let image = unsafe { request.data.image_fetch };
        record.url = String::from_utf8(unsafe { copied(image.url_ptr, image.url_len) }).unwrap();
    } else if request.kind == JianCapabilityKind::Confirm {
        let confirm = unsafe { request.data.confirm };
        record.method =
            String::from_utf8(unsafe { copied(confirm.title_ptr, confirm.title_len) }).unwrap();
        record.url =
            String::from_utf8(unsafe { copied(confirm.message_ptr, confirm.message_len) }).unwrap();
    } else if request.kind == JianCapabilityKind::OpenUrl {
        let open = unsafe { request.data.open_url };
        record.url = String::from_utf8(unsafe { copied(open.url_ptr, open.url_len) }).unwrap();
    }
    log.requests.lock().unwrap().push(record);
}

unsafe extern "C" fn capability_cancelled(user_data: *mut c_void, request_id: u64) {
    let log = unsafe { &*user_data.cast::<CallbackLog>() };
    log.cancelled.lock().unwrap().push(request_id);
}

unsafe extern "C" fn runtime_error(user_data: *mut c_void, error: *const JianRuntimeError) {
    let log = unsafe { &*user_data.cast::<CallbackLog>() };
    let error = unsafe { &*error };
    log.errors.lock().unwrap().push(ErrorRecord {
        kind: error.kind,
        message: String::from_utf8(unsafe { copied(error.message_ptr, error.message_len) })
            .unwrap(),
    });
}

unsafe extern "C" fn record_ime(user_data: *mut c_void, _op: i32, request_id: u64) {
    let log = unsafe { &*user_data.cast::<CallbackLog>() };
    log.ime_requests.lock().unwrap().push(request_id);
}

fn callbacks(log: &CallbackLog) -> JianCallbacks {
    JianCallbacks {
        size: size_of::<JianCallbacks>(),
        user_data: (log as *const CallbackLog).cast_mut().cast(),
        needs_redraw: None,
        runtime_error: Some(runtime_error),
        ime_control: Some(record_ime),
        input_focus_changed: None,
        text_state_changed: None,
        capability_request: Some(capability_request),
        capability_cancelled: Some(capability_cancelled),
    }
}

fn capability_error(kind: JianCapabilityKind, message: &[u8]) -> JianCapabilityResult {
    match kind {
        JianCapabilityKind::HttpFetch => JianCapabilityResult {
            size: size_of::<JianCapabilityResult>(),
            kind: kind as i32,
            data: JianCapabilityResultData {
                http_fetch: JianHttpFetchResult {
                    ok: false,
                    status: 0,
                    headers: ptr::null(),
                    headers_len: 0,
                    body_ptr: ptr::null(),
                    body_len: 0,
                    error_ptr: message.as_ptr(),
                    error_len: message.len(),
                },
            },
        },
        JianCapabilityKind::ImageFetch => JianCapabilityResult {
            size: size_of::<JianCapabilityResult>(),
            kind: kind as i32,
            data: JianCapabilityResultData {
                image_fetch: JianImageFetchResult {
                    ok: false,
                    bytes_ptr: ptr::null(),
                    bytes_len: 0,
                    error_ptr: message.as_ptr(),
                    error_len: message.len(),
                },
            },
        },
        _ => panic!("unsupported test result kind"),
    }
}

unsafe fn complete(
    engine: *mut JianEngine,
    request_id: u64,
    result: &JianCapabilityResult,
) -> JianStatus {
    let function: unsafe extern "C" fn(
        *mut JianEngine,
        u64,
        *const JianCapabilityResult,
    ) -> JianStatus = jian_capability_result;
    unsafe { function(engine, request_id, result) }
}

unsafe fn last_error(engine: *mut JianEngine) -> String {
    let mut required = 0;
    assert_eq!(
        unsafe { jian_last_error(engine, ptr::null_mut(), 0, &mut required) },
        JianStatus::Ok
    );
    let mut bytes = vec![0; required];
    assert_eq!(
        unsafe { jian_last_error(engine, bytes.as_mut_ptr(), bytes.len(), &mut required) },
        JianStatus::Ok
    );
    String::from_utf8(bytes).unwrap()
}

fn descriptor(document: &[u8], callbacks: *const JianCallbacks) -> JianCreateDesc {
    JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: document.as_ptr(),
        doc_len: document.len(),
        width: 40.0,
        height: 40.0,
        dpr: 1.0,
        storage_dir_ptr: ptr::null(),
        storage_dir_len: 0,
        callbacks,
        asset_base_ptr: ptr::null(),
        asset_base_len: 0,
    }
}

unsafe fn create(document: &[u8], callbacks: *const JianCallbacks) -> *mut JianEngine {
    let create: unsafe extern "C" fn(*const JianCreateDesc, *mut *mut JianEngine) -> JianStatus =
        jian_create;
    let mut engine = ptr::null_mut();
    assert_eq!(
        unsafe { create(&descriptor(document, callbacks), &mut engine) },
        JianStatus::Ok
    );
    engine
}

unsafe fn tap(engine: *mut JianEngine, now: u64) {
    let pointer: unsafe extern "C" fn(*mut JianEngine, u32, i32, f32, f32, u64) -> JianStatus =
        jian_pointer;
    assert_eq!(
        unsafe { pointer(engine, 1, JianPointerPhase::Down as i32, 10.0, 10.0, now) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { pointer(engine, 1, JianPointerPhase::Up as i32, 10.0, 10.0, now + 1,) },
        JianStatus::Ok
    );
}

unsafe fn frame(engine: *mut JianEngine, now: u64) {
    let frame: unsafe extern "C" fn(*mut JianEngine, u64, *mut u8, usize, usize) -> JianStatus =
        jian_frame_cpu;
    let mut pixels = vec![0; 40 * 40 * 4];
    assert_eq!(
        unsafe { frame(engine, now, pixels.as_mut_ptr(), pixels.len(), 40 * 4) },
        JianStatus::Ok
    );
}

#[test]
fn fetch_request_and_result_drive_the_authored_success_chain() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "app":{"name":"ffi","version":"1","id":"ffi","capabilities":["network"]},
      "state":{"success":{"type":"int","default":0}},
      "children":[{"type":"frame","id":"button","width":40,"height":40,
        "events":{"onTap":[
          {"fetch":{"url":"'https://example.test/data'","method":"POST","body":null,
                    "timeout_ms":2500,"into":"$app.payload"}},
          {"set":{"$app.success":"1"}}
        ]}}]
    }"#;
    let log = Box::new(CallbackLog::default());
    let table = callbacks(&log);
    let engine = unsafe { create(DOCUMENT, &table) };

    unsafe { tap(engine, 1) };
    unsafe { frame(engine, 3) };
    let request = log.requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.kind, JianCapabilityKind::HttpFetch);
    assert_eq!(request.method, "POST");
    assert_eq!(request.url, "https://example.test/data");
    assert_eq!(request.body.as_deref(), Some(b"null".as_slice()));
    assert_eq!(request.timeout, Some(2500));
    assert!(request.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type") && value == "application/json"
    }));

    let body = br#"{"answer":42}"#;
    let response = JianHttpFetchResult {
        ok: true,
        status: 200,
        headers: ptr::null(),
        headers_len: 0,
        body_ptr: body.as_ptr(),
        body_len: body.len(),
        error_ptr: ptr::null(),
        error_len: 0,
    };
    let result = JianCapabilityResult {
        size: size_of::<JianCapabilityResult>(),
        kind: JianCapabilityKind::HttpFetch as i32,
        data: JianCapabilityResultData {
            http_fetch: response,
        },
    };
    let complete: unsafe extern "C" fn(
        *mut JianEngine,
        u64,
        *const JianCapabilityResult,
    ) -> JianStatus = jian_capability_result;
    assert_eq!(
        unsafe { complete(engine, request.id, &result) },
        JianStatus::Ok
    );
    unsafe { frame(engine, 4) };

    let mut success = 0.0;
    let probe: unsafe extern "C" fn(*mut JianEngine, *const u8, usize, *mut f64) -> JianStatus =
        jian_test_app_number;
    assert_eq!(
        unsafe { probe(engine, b"success".as_ptr(), 7, &mut success) },
        JianStatus::Ok
    );
    assert_eq!(success, 1.0);
    assert!(log.errors.lock().unwrap().is_empty());
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn failed_fetch_without_handler_is_an_async_action_diagnostic() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","responsive":true,
      "app":{"name":"ffi","version":"1","id":"ffi","capabilities":["network"]},
      "children":[{"type":"frame","id":"button","width":40,"height":40,
        "events":{"onTap":[{"fetch":{"url":"'https://example.test/fail'"}}]}}]
    }"#;
    let log = Box::new(CallbackLog::default());
    let table = callbacks(&log);
    let engine = unsafe { create(DOCUMENT, &table) };

    unsafe { tap(engine, 10) };
    unsafe { frame(engine, 12) };
    let request = log.requests.lock().unwrap().pop().unwrap();
    let result = capability_error(JianCapabilityKind::HttpFetch, b"offline");
    assert_eq!(
        unsafe { complete(engine, request.id, &result) },
        JianStatus::Ok
    );
    unsafe { frame(engine, 13) };

    let errors = log.errors.lock().unwrap();
    assert!(errors.iter().any(|error| {
        error.kind == JianRuntimeErrorKind::Action && error.message.contains("offline")
    }));
    drop(errors);
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn image_failure_is_a_warning_and_never_an_action_error() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","responsive":true,
      "app":{"name":"ffi","version":"1","id":"ffi","capabilities":["network"]},
      "children":[{"type":"image","id":"hero","src":"https://example.test/bad.png",
                   "width":20,"height":20}]
    }"#;
    let log = Box::new(CallbackLog::default());
    let table = callbacks(&log);
    let engine = unsafe { create(DOCUMENT, &table) };

    unsafe { frame(engine, 20) };
    let request = log.requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.kind, JianCapabilityKind::ImageFetch);
    let result = capability_error(JianCapabilityKind::ImageFetch, b"bad image");
    assert_eq!(
        unsafe { complete(engine, request.id, &result) },
        JianStatus::Ok
    );
    unsafe { frame(engine, 21) };
    unsafe { frame(engine, 22) };

    let errors = log.errors.lock().unwrap();
    assert!(errors.iter().any(|error| {
        error.kind == JianRuntimeErrorKind::Warning && error.message.contains("bad image")
    }));
    assert!(!errors
        .iter()
        .any(|error| error.kind == JianRuntimeErrorKind::Action));
    drop(errors);
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn reload_retires_request_and_rejects_a_late_result() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","responsive":true,
      "app":{"name":"ffi","version":"1","id":"ffi","capabilities":["network"]},
      "children":[{"type":"frame","id":"button","width":40,"height":40,
        "events":{"onTap":[{"fetch":{"url":"'https://example.test/slow'"}}]}}]
    }"#;
    const REPLACEMENT: &[u8] = br#"{
      "version":"1.2","responsive":true,
      "app":{"name":"ffi","version":"1","id":"ffi","capabilities":[]},"children":[]
    }"#;
    let log = Box::new(CallbackLog::default());
    let table = callbacks(&log);
    let engine = unsafe { create(DOCUMENT, &table) };
    unsafe { tap(engine, 30) };
    unsafe { frame(engine, 32) };
    let request = log.requests.lock().unwrap().pop().unwrap();

    let reload: unsafe extern "C" fn(*mut JianEngine, *const u8, usize) -> JianStatus =
        jian_test_reload;
    assert_eq!(
        unsafe { reload(engine, REPLACEMENT.as_ptr(), REPLACEMENT.len()) },
        JianStatus::Ok
    );
    assert_eq!(log.cancelled.lock().unwrap().as_slice(), &[request.id]);
    let result = capability_error(JianCapabilityKind::HttpFetch, b"late");
    assert_eq!(
        unsafe { complete(engine, request.id, &result) },
        JianStatus::InvalidArg
    );
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn invalid_font_returns_invalid_arg_and_last_error() {
    const DOCUMENT: &[u8] = br#"{"version":"1.2","children":[]}"#;
    let engine = unsafe { create(DOCUMENT, ptr::null()) };
    let register: unsafe extern "C" fn(*mut JianEngine, *const u8, usize) -> JianStatus =
        jian_register_font;
    let garbage = b"not a font";
    assert_eq!(
        unsafe { register(engine, garbage.as_ptr(), garbage.len()) },
        JianStatus::InvalidArg
    );
    assert!(unsafe { last_error(engine) }.contains("font"));
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn oversized_image_result_becomes_a_warning_with_ok_ffi_status() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","responsive":true,
      "app":{"name":"ffi","version":"1","id":"ffi","capabilities":["network"]},
      "children":[{"type":"image","id":"hero","src":"https://example.test/huge.png",
                   "width":20,"height":20}]
    }"#;
    let log = Box::new(CallbackLog::default());
    let table = callbacks(&log);
    let engine = unsafe { create(DOCUMENT, &table) };
    unsafe { frame(engine, 40) };
    let request = log.requests.lock().unwrap().pop().unwrap();
    let oversized = vec![0_u8; 64 * 1024 * 1024 + 1];
    let result = JianCapabilityResult {
        size: size_of::<JianCapabilityResult>(),
        kind: JianCapabilityKind::ImageFetch as i32,
        data: JianCapabilityResultData {
            image_fetch: JianImageFetchResult {
                ok: true,
                bytes_ptr: oversized.as_ptr(),
                bytes_len: oversized.len(),
                error_ptr: ptr::null(),
                error_len: 0,
            },
        },
    };
    assert_eq!(
        unsafe { complete(engine, request.id, &result) },
        JianStatus::Ok
    );
    unsafe { frame(engine, 41) };
    unsafe { frame(engine, 42) };
    let errors = log.errors.lock().unwrap();
    assert!(errors.iter().any(|error| {
        error.kind == JianRuntimeErrorKind::Warning && error.message.contains("64 MiB")
    }));
    assert!(!errors
        .iter()
        .any(|error| error.kind == JianRuntimeErrorKind::Action));
    drop(errors);
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn confirm_and_open_url_roundtrip_and_open_failure_continues() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","responsive":true,
      "app":{"name":"ffi","version":"1","id":"ffi","capabilities":["network"]},
      "state":{"confirmed":{"type":"int","default":0},
               "continued":{"type":"int","default":0}},
      "children":[{"type":"frame","id":"button","width":40,"height":40,
        "events":{"onTap":[
          {"confirm":{"title":"'Title'","message":"'Proceed?'",
                      "on_confirm":[{"set":{"$app.confirmed":"1"}}]}},
          {"open_url":{"url":"'https://example.test/open'"}},
          {"set":{"$app.continued":"1"}}
        ]}}]
    }"#;
    let log = Box::new(CallbackLog::default());
    let table = callbacks(&log);
    let engine = unsafe { create(DOCUMENT, &table) };
    unsafe { tap(engine, 50) };
    unsafe { frame(engine, 52) };
    let confirm = log.requests.lock().unwrap().pop().unwrap();
    assert_eq!(confirm.kind, JianCapabilityKind::Confirm);
    assert_eq!(
        (confirm.method.as_str(), confirm.url.as_str()),
        ("Title", "Proceed?")
    );
    let result = JianCapabilityResult {
        size: size_of::<JianCapabilityResult>(),
        kind: JianCapabilityKind::Confirm as i32,
        data: JianCapabilityResultData {
            confirm: JianConfirmResult { value: true },
        },
    };
    assert_eq!(
        unsafe { complete(engine, confirm.id, &result) },
        JianStatus::Ok
    );
    unsafe { frame(engine, 53) };
    let open = log.requests.lock().unwrap().pop().unwrap();
    assert_eq!(open.kind, JianCapabilityKind::OpenUrl);
    assert_eq!(open.url, "https://example.test/open");
    let message = b"platform rejected URL";
    let result = JianCapabilityResult {
        size: size_of::<JianCapabilityResult>(),
        kind: JianCapabilityKind::OpenUrl as i32,
        data: JianCapabilityResultData {
            open_url: JianOpenUrlResult {
                ok: false,
                error_ptr: message.as_ptr(),
                error_len: message.len(),
            },
        },
    };
    assert_eq!(
        unsafe { complete(engine, open.id, &result) },
        JianStatus::Ok
    );

    for key in ["confirmed", "continued"] {
        let mut value = 0.0;
        assert_eq!(
            unsafe { jian_test_app_number(engine, key.as_ptr(), key.len(), &mut value) },
            JianStatus::Ok
        );
        assert_eq!(value, 1.0);
    }
    assert!(log.errors.lock().unwrap().iter().any(|error| {
        error.kind == JianRuntimeErrorKind::Warning
            && error.message.contains("platform rejected URL")
    }));
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn storage_actions_use_the_create_descriptor_directory() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","responsive":true,
      "app":{"name":"ffi","version":"1","id":"ffi","capabilities":["storage"]},
      "children":[{"type":"frame","id":"button","width":40,"height":40,
        "events":{"onTap":[{"storage_set":{"session":"'persisted'"}}]}}]
    }"#;
    let root = std::env::temp_dir().join(format!(
        "jian-ffi-storage-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root_text = root.to_string_lossy().into_owned();
    let mut desc = descriptor(DOCUMENT, ptr::null());
    desc.storage_dir_ptr = root_text.as_ptr();
    desc.storage_dir_len = root_text.len();
    let mut engine = ptr::null_mut();
    let create_fn: unsafe extern "C" fn(*const JianCreateDesc, *mut *mut JianEngine) -> JianStatus =
        jian_create;
    assert_eq!(unsafe { create_fn(&desc, &mut engine) }, JianStatus::Ok);
    unsafe { tap(engine, 60) };
    unsafe { frame(engine, 62) };

    let entries: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1);
    let stored = std::fs::read_to_string(&entries[0]).unwrap();
    assert!(stored.contains("session"));
    assert!(stored.contains("persisted"));
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "textlayout")]
#[test]
fn font_generation_change_rebuilds_a_parked_variant_once_at_commit() {
    const DOCUMENT: &[u8] = include_bytes!("../../jian-core/tests/fixtures/m1_acceptance.json");
    const FONT: &[u8] = include_bytes!("../../jian-host-web/assets/fonts/Roboto-Regular.ttf");
    let log = Box::new(CallbackLog::default());
    let table = callbacks(&log);
    let engine = unsafe { create(DOCUMENT, &table) };
    assert_eq!(
        unsafe { jian_resize(engine, 320.0, 720.0, 1.0) },
        JianStatus::Ok
    );

    let pointer: unsafe extern "C" fn(*mut JianEngine, u32, i32, f32, f32, u64) -> JianStatus =
        jian_pointer;
    assert_eq!(
        unsafe { pointer(engine, 1, JianPointerPhase::Down as i32, 30.0, 70.0, 1) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { pointer(engine, 1, JianPointerPhase::Up as i32, 30.0, 70.0, 2) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { jian_ime_set_composing_text(engine, b"x".as_ptr(), 1, 1, 1) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { jian_resize(engine, 600.0, 720.0, 1.0) },
        JianStatus::Ok
    );
    let request_id = *log.ime_requests.lock().unwrap().last().unwrap();

    let mut before = 0;
    assert_eq!(
        unsafe { jian_test_font_generation(engine, &mut before) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { jian_register_font(engine, FONT.as_ptr(), FONT.len()) },
        JianStatus::Ok
    );
    let mut after = 0;
    assert_eq!(
        unsafe { jian_test_font_generation(engine, &mut after) },
        JianStatus::Ok
    );
    assert!(after > before);
    assert_eq!(
        unsafe { jian_ime_cancel(engine, request_id) },
        JianStatus::Ok
    );

    let mut build_count = 0;
    assert_eq!(
        unsafe { jian_test_variant_build_count(engine, &mut build_count) },
        JianStatus::Ok
    );
    assert_eq!(build_count, 2);
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}
