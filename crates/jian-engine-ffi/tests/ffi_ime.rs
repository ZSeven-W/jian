use jian_engine_ffi::{
    jian_create, jian_destroy, jian_ime_cancel, jian_ime_commit, jian_ime_set_composing_region,
    jian_ime_set_composing_text, jian_pointer, jian_text_batch_begin, jian_text_batch_end,
    jian_text_caret_rect, jian_text_caret_rect_for_offset, jian_text_get_range,
    jian_text_get_state, jian_text_insert, jian_text_rects_for_range, jian_text_replace_range,
    jian_text_set_selection, JianCallbacks, JianCreateDesc, JianEngine, JianFieldInfo,
    JianImeControlOp, JianInputKind, JianPointerPhase, JianRect, JianReturnKeyHint, JianStatus,
    JianTextRect, JianTextState,
};
#[cfg(feature = "textlayout")]
use jian_engine_ffi::{jian_text_position_at_point, jian_text_range_at_point, JianTextGranularity};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::Mutex;

const EMPTY_DOC: &[u8] = br#"{"version":"0.8.0","children":[]}"#;

#[derive(Clone, Debug, Eq, PartialEq)]
struct FocusRecord {
    focused: bool,
    kind: Option<JianInputKind>,
    return_key: Option<JianReturnKeyHint>,
}

#[derive(Default)]
struct CallbackLog {
    focus: Mutex<Vec<FocusRecord>>,
    text_changes: Mutex<usize>,
    ime: Mutex<Vec<(JianImeControlOp, u64)>>,
}

unsafe extern "C" fn focus_changed(
    user_data: *mut c_void,
    focused: bool,
    info: *const JianFieldInfo,
) {
    let log = unsafe { &*user_data.cast::<CallbackLog>() };
    let (kind, return_key) = if info.is_null() {
        (None, None)
    } else {
        let info = unsafe { &*info };
        (Some(info.input_kind), Some(info.return_key_hint))
    };
    log.focus.lock().unwrap().push(FocusRecord {
        focused,
        kind,
        return_key,
    });
}

unsafe extern "C" fn text_changed(user_data: *mut c_void) {
    let log = unsafe { &*user_data.cast::<CallbackLog>() };
    *log.text_changes.lock().unwrap() += 1;
}

unsafe extern "C" fn ime_control(user_data: *mut c_void, op: i32, request_id: u64) {
    let log = unsafe { &*user_data.cast::<CallbackLog>() };
    let op = JianImeControlOp::try_from(op).unwrap();
    log.ime.lock().unwrap().push((op, request_id));
}

fn callbacks(log: &CallbackLog, with_ime: bool) -> JianCallbacks {
    JianCallbacks {
        size: size_of::<JianCallbacks>(),
        user_data: (log as *const CallbackLog).cast_mut().cast(),
        needs_redraw: None,
        runtime_error: None,
        ime_control: with_ime.then_some(ime_control),
        input_focus_changed: Some(focus_changed),
        text_state_changed: Some(text_changed),
        capability_request: None,
        capability_cancelled: None,
    }
}

fn descriptor(document: &[u8], callbacks: *const JianCallbacks) -> JianCreateDesc {
    JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: document.as_ptr(),
        doc_len: document.len(),
        width: 320.0,
        height: 240.0,
        dpr: 1.0,
        storage_dir_ptr: ptr::null(),
        storage_dir_len: 0,
        callbacks,
        asset_base_ptr: ptr::null(),
        asset_base_len: 0,
    }
}

unsafe fn create(document: &[u8], callbacks: *const JianCallbacks) -> *mut JianEngine {
    let call: unsafe extern "C" fn(*const JianCreateDesc, *mut *mut JianEngine) -> JianStatus =
        jian_create;
    let mut engine = ptr::null_mut();
    assert_eq!(
        unsafe { call(&descriptor(document, callbacks), &mut engine) },
        JianStatus::Ok
    );
    engine
}

unsafe fn destroy(engine: *mut JianEngine) {
    let call: unsafe extern "C" fn(*mut JianEngine) -> JianStatus = jian_destroy;
    assert_eq!(unsafe { call(engine) }, JianStatus::Ok);
}

unsafe fn tap(engine: *mut JianEngine, id: u32, x: f32, y: f32) {
    let call: unsafe extern "C" fn(*mut JianEngine, u32, i32, f32, f32, u64) -> JianStatus =
        jian_pointer;
    assert_eq!(
        unsafe { call(engine, id, JianPointerPhase::Down as i32, x, y, 1) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { call(engine, id, JianPointerPhase::Up as i32, x, y, 2) },
        JianStatus::Ok
    );
}

fn one_field(value: &str) -> Vec<u8> {
    format!(
        r#"{{"version":"0.8.0","children":[{{"type":"text_input","id":"field","x":0,"y":0,"width":300,"height":40,"value":{}}}]}}"#,
        serde_json::to_string(value).unwrap()
    )
    .into_bytes()
}

unsafe fn get_state(engine: *mut JianEngine) -> JianTextState {
    let call: unsafe extern "C" fn(*mut JianEngine, *mut JianTextState) -> JianStatus =
        jian_text_get_state;
    let mut state = JianTextState::default();
    assert_eq!(unsafe { call(engine, &mut state) }, JianStatus::Ok);
    state
}

unsafe fn get_range(engine: *mut JianEngine, start: u32, end: u32) -> String {
    let call: unsafe extern "C" fn(
        *mut JianEngine,
        u32,
        u32,
        *mut u8,
        usize,
        *mut usize,
    ) -> JianStatus = jian_text_get_range;
    let mut required = 0;
    assert_eq!(
        unsafe { call(engine, start, end, ptr::null_mut(), 0, &mut required) },
        JianStatus::Ok
    );
    let mut bytes = vec![0; required];
    assert_eq!(
        unsafe {
            call(
                engine,
                start,
                end,
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut required,
            )
        },
        JianStatus::Ok
    );
    String::from_utf8(bytes).unwrap()
}

#[test]
fn set_marked_text_then_commit_reaches_durable_runtime_text() {
    let document = one_field("");
    let engine = unsafe { create(&document, ptr::null()) };
    unsafe { tap(engine, 1, 10.0, 10.0) };

    let marked: unsafe extern "C" fn(*mut JianEngine, *const u8, usize, u32, u32) -> JianStatus =
        jian_ime_set_composing_text;
    assert_eq!(
        unsafe { marked(engine, b"ni".as_ptr(), 2, 2, 2) },
        JianStatus::Ok
    );
    let marked_state = unsafe { get_state(engine) };
    assert!(marked_state.has_composing);
    assert_eq!(
        (marked_state.composing_start, marked_state.composing_end),
        (0, 2)
    );
    assert_eq!(unsafe { get_range(engine, 0, u32::MAX) }, "ni");

    let commit: unsafe extern "C" fn(*mut JianEngine, *const u8, usize, i32, u64) -> JianStatus =
        jian_ime_commit;
    let text = "你";
    assert_eq!(
        unsafe { commit(engine, text.as_ptr(), text.len(), 1, 0) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { get_range(engine, 0, u32::MAX) }, "你");
    assert!(!unsafe { get_state(engine) }.has_composing);
    unsafe { destroy(engine) };
}

#[test]
fn utf16_ranges_snap_surrogate_interiors_and_arbitrary_long_ranges_copy_exactly() {
    let long = format!("{}A😀中Z{}", "prefix-".repeat(900), "-suffix".repeat(900));
    let document = one_field(&long);
    let engine = unsafe { create(&document, ptr::null()) };
    unsafe { tap(engine, 1, 10.0, 10.0) };

    let marker = long.find("A😀中Z").unwrap();
    let start = long[..marker].encode_utf16().count() as u32;
    let select: unsafe extern "C" fn(*mut JianEngine, u32, u32) -> JianStatus =
        jian_text_set_selection;
    assert_eq!(
        unsafe { select(engine, start + 2, start + 4) },
        JianStatus::Ok
    );
    let state = unsafe { get_state(engine) };
    assert_eq!(state.selection_start, start + 1);
    assert_eq!(state.selection_end, start + 4);
    assert_eq!(unsafe { get_range(engine, start + 1, start + 4) }, "😀中");

    let arbitrary_start = 133_u32;
    let arbitrary_end = long.encode_utf16().count() as u32 - 211;
    let start_byte = jian_core::render::utf16_to_byte_offset(&long, arbitrary_start);
    let end_byte = jian_core::render::utf16_to_byte_offset(&long, arbitrary_end);
    assert_eq!(
        unsafe { get_range(engine, arbitrary_start, arbitrary_end) },
        long[start_byte..end_byte]
    );
    unsafe { destroy(engine) };
}

#[test]
fn every_text_mutator_reaches_the_focused_runtime_state() {
    let document = one_field("abcd");
    let engine = unsafe { create(&document, ptr::null()) };
    unsafe { tap(engine, 1, 10.0, 10.0) };

    let replace: unsafe extern "C" fn(*mut JianEngine, u32, u32, *const u8, usize) -> JianStatus =
        jian_text_replace_range;
    assert_eq!(
        unsafe { replace(engine, 3, 1, b"XY".as_ptr(), 2) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { get_range(engine, 0, u32::MAX) }, "aXYd");

    assert_eq!(
        unsafe { jian_text_set_selection(engine, 1, 3) },
        JianStatus::Ok
    );
    let insert: unsafe extern "C" fn(*mut JianEngine, *const u8, usize) -> JianStatus =
        jian_text_insert;
    assert_eq!(
        unsafe { insert(engine, "中".as_ptr(), "中".len()) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { get_range(engine, 0, u32::MAX) }, "a中d");

    let region: unsafe extern "C" fn(*mut JianEngine, u32, u32) -> JianStatus =
        jian_ime_set_composing_region;
    assert_eq!(unsafe { region(engine, 2, 1) }, JianStatus::Ok);
    assert!(unsafe { get_state(engine) }.has_composing);
    assert_eq!(unsafe { jian_ime_cancel(engine, 0) }, JianStatus::Ok);
    assert_eq!(unsafe { get_range(engine, 0, u32::MAX) }, "ad");
    unsafe { destroy(engine) };
}

#[test]
fn nested_batches_and_focus_changes_obey_callback_suppression() {
    const DOCUMENT: &[u8] = br#"{
      "version":"0.8.0","children":[
        {"type":"text_input","id":"a","x":0,"y":0,"width":140,"height":40},
        {"type":"number_input","id":"b","x":0,"y":60,"width":140,"height":40}
      ]}"#;
    let log = CallbackLog::default();
    let table = callbacks(&log, true);
    let engine = unsafe { create(DOCUMENT, &table) };
    unsafe { tap(engine, 1, 10.0, 10.0) };
    assert_eq!(
        log.focus.lock().unwrap().last().cloned(),
        Some(FocusRecord {
            focused: true,
            kind: Some(JianInputKind::Text),
            return_key: Some(JianReturnKeyHint::Default),
        })
    );
    *log.text_changes.lock().unwrap() = 0;

    assert_eq!(unsafe { jian_text_batch_begin(engine) }, JianStatus::Ok);
    assert_eq!(unsafe { jian_text_batch_begin(engine) }, JianStatus::Ok);
    assert_eq!(
        unsafe { jian_text_insert(engine, b"x".as_ptr(), 1) },
        JianStatus::Ok
    );
    assert_eq!(*log.text_changes.lock().unwrap(), 0);
    assert_eq!(unsafe { jian_text_batch_end(engine) }, JianStatus::Ok);
    assert_eq!(*log.text_changes.lock().unwrap(), 0);
    assert_eq!(unsafe { jian_text_batch_end(engine) }, JianStatus::Ok);
    assert_eq!(*log.text_changes.lock().unwrap(), 1);
    assert_eq!(
        unsafe { jian_text_batch_end(engine) },
        JianStatus::InvalidArg
    );

    assert_eq!(unsafe { jian_text_batch_begin(engine) }, JianStatus::Ok);
    assert_eq!(
        unsafe { jian_text_insert(engine, b"y".as_ptr(), 1) },
        JianStatus::Ok
    );
    unsafe { tap(engine, 2, 10.0, 70.0) };
    assert_eq!(*log.text_changes.lock().unwrap(), 2);
    assert_eq!(
        unsafe { jian_text_batch_end(engine) },
        JianStatus::InvalidArg
    );
    assert_eq!(
        log.focus.lock().unwrap().last().unwrap().kind,
        Some(JianInputKind::Number)
    );
    unsafe { destroy(engine) };
}

#[test]
fn batch_depth_is_bounded_at_sixty_four_through_the_abi() {
    let document = one_field("");
    let engine = unsafe { create(&document, ptr::null()) };
    unsafe { tap(engine, 1, 10.0, 10.0) };
    for _ in 0..64 {
        assert_eq!(unsafe { jian_text_batch_begin(engine) }, JianStatus::Ok);
    }
    assert_eq!(
        unsafe { jian_text_batch_begin(engine) },
        JianStatus::InvalidArg
    );
    for _ in 0..64 {
        assert_eq!(unsafe { jian_text_batch_end(engine) }, JianStatus::Ok);
    }
    unsafe { destroy(engine) };
}

#[test]
fn no_focus_is_a_noop_except_nonzero_handshake_confirmations() {
    let engine = unsafe { create(EMPTY_DOC, ptr::null()) };
    assert_eq!(
        unsafe { jian_text_insert(engine, b"x".as_ptr(), 1) },
        JianStatus::NoFocus
    );
    assert_eq!(
        unsafe { jian_text_replace_range(engine, 0, 1, b"x".as_ptr(), 1) },
        JianStatus::NoFocus
    );
    assert_eq!(
        unsafe { jian_text_set_selection(engine, 0, 1) },
        JianStatus::NoFocus
    );
    assert_eq!(
        unsafe { jian_ime_set_composing_region(engine, 0, 1) },
        JianStatus::NoFocus
    );
    assert_eq!(
        unsafe { jian_ime_set_composing_text(engine, b"x".as_ptr(), 1, 0, 1) },
        JianStatus::NoFocus
    );
    assert_eq!(
        unsafe { jian_ime_commit(engine, b"x".as_ptr(), 1, 1, 0) },
        JianStatus::NoFocus
    );
    assert_eq!(unsafe { jian_ime_cancel(engine, 0) }, JianStatus::NoFocus);
    assert_eq!(
        unsafe { jian_ime_commit(engine, b"x".as_ptr(), 1, 1, 99) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { jian_ime_cancel(engine, 100) }, JianStatus::Ok);

    let mut state = JianTextState::default();
    assert_eq!(
        unsafe { jian_text_get_state(engine, &mut state) },
        JianStatus::NoFocus
    );
    unsafe { destroy(engine) };
}

#[test]
fn null_ime_control_cancels_locally_when_composition_target_changes() {
    const DOCUMENT: &[u8] = br#"{
      "version":"0.8.0","children":[
        {"type":"text_input","id":"a","x":0,"y":0,"width":140,"height":40},
        {"type":"text_input","id":"b","x":0,"y":60,"width":140,"height":40}
      ]}"#;
    let engine = unsafe { create(DOCUMENT, ptr::null()) };
    unsafe { tap(engine, 1, 10.0, 10.0) };
    assert_eq!(
        unsafe { jian_ime_set_composing_text(engine, b"ni".as_ptr(), 2, 2, 2) },
        JianStatus::Ok
    );
    unsafe { tap(engine, 2, 10.0, 70.0) };
    unsafe { tap(engine, 3, 10.0, 10.0) };
    let state = unsafe { get_state(engine) };
    assert!(!state.has_composing);
    assert_eq!((state.selection_start, state.selection_end), (0, 0));
    assert_eq!(unsafe { get_range(engine, 0, u32::MAX) }, "");
    unsafe { destroy(engine) };
}

#[test]
fn ime_control_request_id_commits_the_detached_composition_once() {
    const DOCUMENT: &[u8] = br#"{
      "version":"0.8.0","children":[
        {"type":"text_input","id":"a","x":0,"y":0,"width":140,"height":40},
        {"type":"text_input","id":"b","x":0,"y":60,"width":140,"height":40}
      ]}"#;
    let log = CallbackLog::default();
    let table = callbacks(&log, true);
    let engine = unsafe { create(DOCUMENT, &table) };
    unsafe { tap(engine, 1, 10.0, 10.0) };
    assert_eq!(
        unsafe { jian_ime_set_composing_text(engine, b"ni".as_ptr(), 2, 2, 2) },
        JianStatus::Ok
    );
    unsafe { tap(engine, 2, 10.0, 70.0) };
    let (op, request_id) = *log.ime.lock().unwrap().last().unwrap();
    assert_eq!(op, JianImeControlOp::Dismiss);
    assert_ne!(request_id, 0);

    let committed = "你";
    assert_eq!(
        unsafe { jian_ime_commit(engine, committed.as_ptr(), committed.len(), 1, request_id,) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { jian_ime_commit(engine, committed.as_ptr(), committed.len(), 1, request_id,) },
        JianStatus::Ok
    );
    unsafe { tap(engine, 3, 10.0, 10.0) };
    assert_eq!(unsafe { get_range(engine, 0, u32::MAX) }, "你");
    unsafe { destroy(engine) };
}

#[test]
fn geometry_count_contract_and_caret_entry_points_are_reached() {
    let document = one_field("alpha beta gamma delta epsilon zeta");
    let engine = unsafe { create(&document, ptr::null()) };
    unsafe { tap(engine, 1, 10.0, 10.0) };

    let mut needed = usize::MAX;
    let rects: unsafe extern "C" fn(
        *mut JianEngine,
        u32,
        u32,
        *mut JianTextRect,
        usize,
        *mut usize,
    ) -> JianStatus = jian_text_rects_for_range;
    assert_eq!(
        unsafe { rects(engine, 0, u32::MAX, ptr::null_mut(), 0, &mut needed) },
        JianStatus::Ok
    );
    assert!(needed > 0);

    assert_eq!(
        unsafe { jian_text_set_selection(engine, 5, 5) },
        JianStatus::Ok
    );
    let mut current = JianRect::default();
    let caret: unsafe extern "C" fn(*mut JianEngine, *mut JianRect) -> JianStatus =
        jian_text_caret_rect;
    assert_eq!(unsafe { caret(engine, &mut current) }, JianStatus::Ok);
    let mut explicit = JianRect::default();
    let caret_at: unsafe extern "C" fn(*mut JianEngine, u32, *mut JianRect) -> JianStatus =
        jian_text_caret_rect_for_offset;
    assert_eq!(
        unsafe { caret_at(engine, 5, &mut explicit) },
        JianStatus::Ok
    );
    assert_eq!(current, explicit);
    unsafe { destroy(engine) };
}

#[test]
#[cfg(feature = "textlayout")]
fn shaped_hit_testing_round_trips_and_uax29_ranges_cross_the_raw_ffi() {
    let document = one_field("A👩‍💻B 设计 OpenAI");
    let engine = unsafe { create(&document, ptr::null()) };
    unsafe { tap(engine, 1, 10.0, 10.0) };

    let mut caret = JianRect::default();
    assert_eq!(
        unsafe { jian_text_caret_rect_for_offset(engine, 6, &mut caret) },
        JianStatus::Ok
    );
    let mut hit = u32::MAX;
    assert_eq!(
        unsafe {
            jian_text_position_at_point(
                engine,
                caret.x + 0.1,
                caret.y + caret.height * 0.5,
                &mut hit,
            )
        },
        JianStatus::Ok
    );
    assert_eq!(hit, 6);

    let mut start = 0;
    let mut end = 0;
    let mut emoji = JianRect::default();
    assert_eq!(
        unsafe { jian_text_caret_rect_for_offset(engine, 2, &mut emoji) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe {
            jian_text_range_at_point(
                engine,
                emoji.x,
                emoji.y + emoji.height * 0.5,
                JianTextGranularity::Character as i32,
                &mut start,
                &mut end,
            )
        },
        JianStatus::Ok
    );
    assert_eq!((start, end), (1, 6));

    let mut latin = JianRect::default();
    assert_eq!(
        unsafe { jian_text_caret_rect_for_offset(engine, 11, &mut latin) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe {
            jian_text_range_at_point(
                engine,
                latin.x,
                latin.y + latin.height * 0.5,
                JianTextGranularity::Word as i32,
                &mut start,
                &mut end,
            )
        },
        JianStatus::Ok
    );
    assert_eq!((start, end), (11, 17));
    unsafe { destroy(engine) };
}
