use jian_engine_ffi::{
    jian_create, jian_destroy, jian_frame_cpu, jian_pointer, JianCallbacks, JianCreateDesc,
    JianEngine, JianImeControl, JianPointerPhase, JianRuntimeError, JianRuntimeErrorKind,
    JianStatus,
};
#[cfg(debug_assertions)]
use jian_engine_ffi::{
    jian_test_app_number, jian_test_node_rect, jian_test_viewport_number, JianRect,
};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::Mutex;

#[derive(Debug)]
struct ErrorRecord {
    kind: JianRuntimeErrorKind,
    message: String,
    source: Option<String>,
}

#[derive(Default)]
struct ErrorLog {
    records: Mutex<Vec<ErrorRecord>>,
}

unsafe extern "C" fn record_runtime_error(user_data: *mut c_void, error: *const JianRuntimeError) {
    let log = unsafe { &*user_data.cast::<ErrorLog>() };
    let error = unsafe { &*error };
    let message = unsafe { std::slice::from_raw_parts(error.message_ptr, error.message_len) };
    let source = if error.source_len == 0 {
        None
    } else {
        Some(
            String::from_utf8(
                unsafe { std::slice::from_raw_parts(error.source_ptr, error.source_len) }.to_vec(),
            )
            .unwrap(),
        )
    };
    log.records.lock().unwrap().push(ErrorRecord {
        kind: error.kind,
        message: String::from_utf8(message.to_vec()).unwrap(),
        source,
    });
}

unsafe extern "C" fn noop_ime_control(_user_data: *mut c_void, _op: i32, _request_id: u64) {}

fn callbacks(log: &ErrorLog, ime_control: JianImeControl) -> JianCallbacks {
    JianCallbacks {
        size: size_of::<JianCallbacks>(),
        user_data: (log as *const ErrorLog).cast_mut().cast(),
        needs_redraw: None,
        runtime_error: Some(record_runtime_error),
        ime_control,
        input_focus_changed: None,
        text_state_changed: None,
        capability_request: None,
        capability_cancelled: None,
    }
}

fn desc(document: &[u8], callbacks: *const JianCallbacks) -> JianCreateDesc {
    JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: document.as_ptr(),
        doc_len: document.len(),
        width: 100.0,
        height: 100.0,
        dpr: 1.0,
        storage_dir_ptr: ptr::null(),
        storage_dir_len: 0,
        callbacks,
        asset_base_ptr: ptr::null(),
        asset_base_len: 0,
    }
}

unsafe fn create(document: &[u8], callbacks: *const JianCallbacks) -> *mut JianEngine {
    unsafe { create_at_dpr(document, callbacks, 1.0) }
}

unsafe fn create_at_dpr(
    document: &[u8],
    callbacks: *const JianCallbacks,
    dpr: f32,
) -> *mut JianEngine {
    let call: unsafe extern "C" fn(*const JianCreateDesc, *mut *mut JianEngine) -> JianStatus =
        jian_create;
    let mut descriptor = desc(document, callbacks);
    descriptor.dpr = dpr;
    let mut engine = ptr::null_mut();
    assert_eq!(unsafe { call(&descriptor, &mut engine) }, JianStatus::Ok);
    assert!(!engine.is_null());
    engine
}

unsafe fn destroy(engine: *mut JianEngine) {
    let call: unsafe extern "C" fn(*mut JianEngine) -> JianStatus = jian_destroy;
    assert_eq!(unsafe { call(engine) }, JianStatus::Ok);
}

unsafe fn pointer(
    engine: *mut JianEngine,
    id: u32,
    phase: JianPointerPhase,
    x: f32,
    y: f32,
    now_ms: u64,
) -> JianStatus {
    let call: unsafe extern "C" fn(*mut JianEngine, u32, i32, f32, f32, u64) -> JianStatus =
        jian_pointer;
    unsafe { call(engine, id, phase as i32, x, y, now_ms) }
}

#[test]
#[cfg(debug_assertions)]
fn two_finger_pinch_reaches_the_runtime_scale_handler() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "state":{"zoom":{"type":"float","default":1.0}},
      "children":[{"type":"frame","id":"root","width":100,"height":100,
        "events":{"onScaleUpdate":[{"set":{"$app.zoom":"$event.scale"}}]}}]
    }"#;
    let engine = unsafe { create(DOCUMENT, ptr::null()) };

    assert_eq!(
        unsafe { pointer(engine, 10, JianPointerPhase::Down, 20.0, 50.0, 10) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { pointer(engine, 11, JianPointerPhase::Down, 60.0, 50.0, 11) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { pointer(engine, 10, JianPointerPhase::Move, 10.0, 50.0, 12) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { pointer(engine, 11, JianPointerPhase::Move, 90.0, 50.0, 13) },
        JianStatus::Ok
    );

    let mut observed = 0.0_f64;
    let key = b"zoom";
    let probe: unsafe extern "C" fn(*mut JianEngine, *const u8, usize, *mut f64) -> JianStatus =
        jian_test_app_number;
    assert_eq!(
        unsafe { probe(engine, key.as_ptr(), key.len(), &mut observed) },
        JianStatus::Ok
    );
    assert!(
        (observed - 2.0).abs() < f64::EPSILON,
        "observed scale {observed}"
    );
    unsafe { destroy(engine) };
}

#[test]
#[cfg(debug_assertions)]
fn inset_channels_relayout_bound_geometry_without_reducing_viewport_width() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "children":[{"type":"frame","id":"root","layout":"vertical","alignItems":"start",
        "width":100,"height":100,"children":[
          {"type":"rectangle","id":"safe","width":1,"height":10,
           "bindings":{"width":"$viewport.safeArea.left"}},
          {"type":"rectangle","id":"keyboard","width":1,"height":10,
           "bindings":{"width":"$viewport.keyboard.height"}},
          {"type":"rectangle","id":"dpr","width":1,"height":10,
           "bindings":{"width":"$viewport.dpr"}},
          {"type":"rectangle","id":"full","width":1,"height":10,
           "bindings":{"width":"$viewport.width"}}
        ]}]
    }"#;
    let engine = unsafe { create_at_dpr(DOCUMENT, ptr::null(), 2.0) };
    let set_safe_area: unsafe extern "C" fn(*mut JianEngine, f32, f32, f32, f32) -> JianStatus =
        jian_engine_ffi::jian_set_safe_area;
    assert_eq!(
        unsafe { set_safe_area(engine, 0.0, 0.0, 0.0, 20.0) },
        JianStatus::Ok
    );
    let set_keyboard: unsafe extern "C" fn(*mut JianEngine, f32) -> JianStatus =
        jian_engine_ffi::jian_set_keyboard;
    assert_eq!(unsafe { set_keyboard(engine, 15.0) }, JianStatus::Ok);

    let node_rect: unsafe extern "C" fn(
        *mut JianEngine,
        *const u8,
        usize,
        *mut JianRect,
    ) -> JianStatus = jian_test_node_rect;
    let mut safe = JianRect::default();
    let mut keyboard = JianRect::default();
    let mut dpr = JianRect::default();
    let mut full = JianRect::default();
    assert_eq!(
        unsafe { node_rect(engine, b"safe".as_ptr(), 4, &mut safe) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { node_rect(engine, b"keyboard".as_ptr(), 8, &mut keyboard) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { node_rect(engine, b"dpr".as_ptr(), 3, &mut dpr) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { node_rect(engine, b"full".as_ptr(), 4, &mut full) },
        JianStatus::Ok
    );
    assert_eq!(safe.width, 20.0);
    assert_eq!(keyboard.width, 15.0);
    assert_eq!(dpr.width, 2.0);
    assert_eq!(full.width, 100.0);
    unsafe { destroy(engine) };
}

#[test]
#[cfg(debug_assertions)]
fn authored_viewport_write_warns_and_is_a_runtime_noop() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "children":[{"type":"frame","id":"root","width":100,"height":100,
        "events":{"onTap":[{"set":{"$viewport.width":"999"}}]}}]
    }"#;
    let log = Box::new(ErrorLog::default());
    let table = callbacks(&log, Some(noop_ime_control));
    let engine = unsafe { create(DOCUMENT, &table) };
    log.records.lock().unwrap().clear();

    assert_eq!(
        unsafe { pointer(engine, 1, JianPointerPhase::Down, 10.0, 10.0, 1) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { pointer(engine, 1, JianPointerPhase::Up, 10.0, 10.0, 2) },
        JianStatus::Ok
    );

    let records = log.records.lock().unwrap();
    assert!(records.iter().any(|record| {
        record.kind == JianRuntimeErrorKind::Warning
            && record.message.contains("$viewport is read-only")
            && record.source.as_deref() == Some("onTap")
    }));
    drop(records);
    let mut width = 0.0;
    let probe: unsafe extern "C" fn(*mut JianEngine, *const u8, usize, *mut f64) -> JianStatus =
        jian_test_viewport_number;
    assert_eq!(
        unsafe { probe(engine, b"width".as_ptr(), 5, &mut width) },
        JianStatus::Ok
    );
    assert_eq!(width, 100.0);
    unsafe { destroy(engine) };
}

#[test]
fn asynchronous_action_failure_is_a_callback_not_a_status() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "app":{"name":"ffi","version":"1","id":"ffi","capabilities":["network"]},
      "children":[{"type":"frame","id":"root","width":100,"height":100,
        "events":{"onTap":[{"fetch":{"url":"'https://example.invalid'"}}]}}]
    }"#;
    let log = Box::new(ErrorLog::default());
    let table = callbacks(&log, Some(noop_ime_control));
    let engine = unsafe { create(DOCUMENT, &table) };
    log.records.lock().unwrap().clear();

    assert_eq!(
        unsafe { pointer(engine, 2, JianPointerPhase::Down, 10.0, 10.0, 10) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { pointer(engine, 2, JianPointerPhase::Up, 10.0, 10.0, 11) },
        JianStatus::Ok
    );
    let mut pixels = vec![0; 100 * 100 * 4];
    let frame: unsafe extern "C" fn(*mut JianEngine, u64, *mut u8, usize, usize) -> JianStatus =
        jian_frame_cpu;
    assert_eq!(
        unsafe { frame(engine, 12, pixels.as_mut_ptr(), pixels.len(), 400) },
        JianStatus::Ok
    );

    let records = log.records.lock().unwrap();
    assert!(records.iter().any(|record| {
        record.kind == JianRuntimeErrorKind::Action
            && record
                .message
                .contains("capability_request callback is unavailable")
            && record.source.as_deref() == Some("onTap")
    }));
    drop(records);
    unsafe { destroy(engine) };
}

#[test]
fn create_reports_load_and_missing_ime_warnings_through_borrowed_payloads() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "children":[{"type":"frame","id":"root","width":100,"height":100,"minWidth":120}]
    }"#;
    let log = Box::new(ErrorLog::default());
    let table = callbacks(&log, None);
    let engine = unsafe { create(DOCUMENT, &table) };

    let records = log.records.lock().unwrap();
    assert!(records.iter().any(|record| {
        record.kind == JianRuntimeErrorKind::Warning && record.message.contains("ime_control")
    }));
    assert!(records.iter().any(|record| {
        record.kind == JianRuntimeErrorKind::Warning && record.message.contains("min/max")
    }));
    drop(records);
    unsafe { destroy(engine) };
}

// Regression: a responsive document must mount the variant that matches the
// *creation* viewport width, not the runtime's default 800x600. An iPhone
// Player creates the engine directly at its logical width (e.g. 402), which
// falls inside the mobile breakpoint; if initial mount ran against the stale
// default width it would wrongly land on the tablet variant and never correct
// itself when the host issues no follow-up resize.
//
// The `bound` rectangle additionally guards that the selected variant is
// mounted through the full layout path: its width binds to `$viewport.width`,
// so a complete mount resolves it to the creation width (402). A shortcut that
// mounted the wrong variant and merely swapped afterwards would skip layout-
// binding materialization and leave it at the authored `1`.
#[test]
#[cfg(debug_assertions)]
fn create_selects_initial_variant_for_creation_width() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "children":[
        {"type":"frame","id":"home-mobile","screen":"/home",
         "breakpoint":{"minWidth":0,"maxWidth":480},"width":320,"height":720,
         "children":[
           {"type":"rectangle","id":"probe","x":10,"y":10,"width":60,"height":40},
           {"type":"rectangle","id":"bound","x":10,"y":60,"width":1,"height":10,
            "bindings":{"width":"$viewport.width"}}]},
        {"type":"frame","id":"home-tablet","screen":"/home",
         "breakpoint":{"minWidth":480.5,"maxWidth":1024},"width":768,"height":720,
         "children":[
           {"type":"rectangle","id":"probe","x":10,"y":10,"width":90,"height":40},
           {"type":"rectangle","id":"bound","x":10,"y":60,"width":1,"height":10,
            "bindings":{"width":"$viewport.width"}}]}
      ]
    }"#;
    let mut descriptor = desc(DOCUMENT, ptr::null());
    descriptor.width = 402.0;
    descriptor.height = 874.0;
    descriptor.dpr = 3.0;
    let create_call: unsafe extern "C" fn(
        *const JianCreateDesc,
        *mut *mut JianEngine,
    ) -> JianStatus = jian_create;
    let mut engine = ptr::null_mut();
    assert_eq!(
        unsafe { create_call(&descriptor, &mut engine) },
        JianStatus::Ok
    );
    assert!(!engine.is_null());

    let node_rect: unsafe extern "C" fn(
        *mut JianEngine,
        *const u8,
        usize,
        *mut JianRect,
    ) -> JianStatus = jian_test_node_rect;
    let mut probe = JianRect::default();
    assert_eq!(
        unsafe { node_rect(engine, b"probe".as_ptr(), 5, &mut probe) },
        JianStatus::Ok
    );
    assert_eq!(
        probe.width, 60.0,
        "creation width 402 is inside the mobile breakpoint (0-480); the mobile \
         variant (probe width 60) must mount, not the tablet variant (probe width 90)"
    );
    let mut bound = JianRect::default();
    assert_eq!(
        unsafe { node_rect(engine, b"bound".as_ptr(), 5, &mut bound) },
        JianStatus::Ok
    );
    assert_eq!(
        bound.width, 402.0,
        "the mounted variant must resolve `$viewport.width` layout bindings for \
         the creation width; a post-mount swap shortcut would leave the authored 1"
    );
    unsafe { destroy(engine) };
}

// A device rotation reaches the runtime as a jian_resize on the host's layout
// pass. Portrait (402) sits in the mobile breakpoint; landscape (874) sits in
// the tablet breakpoint. The resize must swap the mounted variant live, and
// rotating back must swap it back — this is the responsive rotation acceptance
// exercised through the exact C-ABI the iOS Player drives.
#[test]
#[cfg(debug_assertions)]
fn rotation_resize_swaps_responsive_variant_live() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "children":[
        {"type":"frame","id":"home-mobile","screen":"/home",
         "breakpoint":{"minWidth":0,"maxWidth":480},"width":320,"height":720,
         "children":[
           {"type":"rectangle","id":"probe","x":10,"y":10,"width":60,"height":40},
           {"type":"rectangle","id":"bound","x":10,"y":60,"width":1,"height":10,
            "bindings":{"width":"$viewport.width"}}]},
        {"type":"frame","id":"home-tablet","screen":"/home",
         "breakpoint":{"minWidth":480.5,"maxWidth":1024},"width":768,"height":720,
         "children":[
           {"type":"rectangle","id":"probe","x":10,"y":10,"width":90,"height":40},
           {"type":"rectangle","id":"bound","x":10,"y":60,"width":1,"height":10,
            "bindings":{"width":"$viewport.width"}}]}
      ]
    }"#;
    let mut descriptor = desc(DOCUMENT, ptr::null());
    descriptor.width = 402.0;
    descriptor.height = 874.0;
    descriptor.dpr = 3.0;
    let create_call: unsafe extern "C" fn(
        *const JianCreateDesc,
        *mut *mut JianEngine,
    ) -> JianStatus = jian_create;
    let mut engine = ptr::null_mut();
    assert_eq!(
        unsafe { create_call(&descriptor, &mut engine) },
        JianStatus::Ok
    );

    let node_rect: unsafe extern "C" fn(
        *mut JianEngine,
        *const u8,
        usize,
        *mut JianRect,
    ) -> JianStatus = jian_test_node_rect;
    let resize: unsafe extern "C" fn(*mut JianEngine, f32, f32, f32) -> JianStatus =
        jian_engine_ffi::jian_resize;
    let width_of = move |engine: *mut JianEngine, id: &[u8]| -> f32 {
        let mut rect = JianRect::default();
        assert_eq!(
            unsafe { node_rect(engine, id.as_ptr(), id.len(), &mut rect) },
            JianStatus::Ok
        );
        rect.width
    };

    assert_eq!(
        width_of(engine, b"probe"),
        60.0,
        "portrait mounts the mobile variant"
    );
    assert_eq!(
        width_of(engine, b"bound"),
        402.0,
        "mobile variant resolves $viewport.width to the portrait width"
    );
    // Rotate to landscape: logical width crosses into the tablet breakpoint.
    assert_eq!(
        unsafe { resize(engine, 874.0, 402.0, 3.0) },
        JianStatus::Ok
    );
    assert_eq!(
        width_of(engine, b"probe"),
        90.0,
        "landscape width 874 must swap to the tablet variant live"
    );
    assert_eq!(
        width_of(engine, b"bound"),
        874.0,
        "the live variant swap must materialize the new variant's $viewport.width \
         layout bindings for the landscape width, not leave the authored 1"
    );
    // Rotate back to portrait: the mobile variant must return.
    assert_eq!(
        unsafe { resize(engine, 402.0, 874.0, 3.0) },
        JianStatus::Ok
    );
    assert_eq!(
        width_of(engine, b"probe"),
        60.0,
        "rotating back to portrait must restore the mobile variant"
    );
    assert_eq!(
        width_of(engine, b"bound"),
        402.0,
        "rotating back must re-materialize the mobile variant bindings"
    );
    unsafe { destroy(engine) };
}
