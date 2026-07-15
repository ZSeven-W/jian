#[cfg(debug_assertions)]
use jian_engine_ffi::jian_test_force_panic;
use jian_engine_ffi::{
    jian_create, jian_destroy, jian_get_pixel_size, jian_last_error, jian_resize, JianCallbacks,
    JianCreateDesc, JianEngine, JianStatus,
};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

const DOC: &[u8] = br#"{"version":"0.8.0","children":[]}"#;

fn exact_desc(doc: &[u8], width: f32, height: f32, dpr: f32) -> JianCreateDesc {
    JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: doc.as_ptr(),
        doc_len: doc.len(),
        width,
        height,
        dpr,
        storage_dir_ptr: ptr::null(),
        storage_dir_len: 0,
        callbacks: ptr::null(),
        asset_base_ptr: ptr::null(),
        asset_base_len: 0,
    }
}

unsafe fn create(desc: &JianCreateDesc) -> Result<*mut JianEngine, JianStatus> {
    let call: unsafe extern "C" fn(*const JianCreateDesc, *mut *mut JianEngine) -> JianStatus =
        jian_create;
    let mut engine = ptr::null_mut();
    let status = unsafe { call(desc, &mut engine) };
    if status == JianStatus::Ok {
        Ok(engine)
    } else {
        assert!(engine.is_null());
        Err(status)
    }
}

unsafe fn destroy(engine: *mut JianEngine) -> JianStatus {
    let call: unsafe extern "C" fn(*mut JianEngine) -> JianStatus = jian_destroy;
    unsafe { call(engine) }
}

fn thread_error() -> String {
    let call: unsafe extern "C" fn(*mut JianEngine, *mut u8, usize, *mut usize) -> JianStatus =
        jian_last_error;
    let mut required = 0;
    assert_eq!(
        unsafe { call(ptr::null_mut(), ptr::null_mut(), 0, &mut required) },
        JianStatus::Ok
    );
    let mut bytes = vec![0; required];
    assert_eq!(
        unsafe {
            call(
                ptr::null_mut(),
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
fn status_values_are_the_stable_int32_contract() {
    let values = [
        JianStatus::Ok,
        JianStatus::InvalidArg,
        JianStatus::BadDocument,
        JianStatus::LayoutError,
        JianStatus::GpuError,
        JianStatus::OutOfMemory,
        JianStatus::WrongThread,
        JianStatus::Suspended,
        JianStatus::Busy,
        JianStatus::NoFocus,
        JianStatus::NotReady,
        JianStatus::Poisoned,
    ];
    assert_eq!(size_of::<JianStatus>(), size_of::<i32>());
    assert_eq!(
        values.map(|value| value as i32),
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
}

#[test]
fn exact_create_and_destroy_work_through_raw_function_pointers() {
    let engine = unsafe { create(&exact_desc(DOC, 16.0, 12.0, 2.0)).unwrap() };
    assert!(!engine.is_null());
    assert_eq!(unsafe { destroy(engine) }, JianStatus::Ok);
}

#[repr(C)]
struct MinimumCreateDesc {
    size: usize,
    doc_ptr: *const u8,
    doc_len: usize,
    width: f32,
    height: f32,
    dpr: f32,
}

#[test]
fn create_descriptor_supports_tail_growth_but_rejects_unknown_tail() {
    let minimum = MinimumCreateDesc {
        size: size_of::<MinimumCreateDesc>(),
        doc_ptr: DOC.as_ptr(),
        doc_len: DOC.len(),
        width: 10.0,
        height: 8.0,
        dpr: 1.0,
    };
    let mut engine = ptr::null_mut();
    assert_eq!(
        unsafe {
            jian_create(
                (&minimum as *const MinimumCreateDesc).cast::<JianCreateDesc>(),
                &mut engine,
            )
        },
        JianStatus::Ok
    );
    assert_eq!(unsafe { destroy(engine) }, JianStatus::Ok);

    let mut larger = exact_desc(DOC, 10.0, 8.0, 1.0);
    larger.size += 8;
    assert_eq!(unsafe { create(&larger) }, Err(JianStatus::InvalidArg));
}

#[test]
fn callbacks_table_has_independent_tail_growth_validation() {
    let minimum = JianCallbacks {
        size: size_of::<usize>(),
        user_data: ptr::null_mut(),
        needs_redraw: None,
        runtime_error: None,
        ime_control: None,
        input_focus_changed: None,
        text_state_changed: None,
    };
    let mut desc = exact_desc(DOC, 10.0, 8.0, 1.0);
    desc.callbacks = &minimum;
    let engine = unsafe { create(&desc).unwrap() };
    assert_eq!(unsafe { destroy(engine) }, JianStatus::Ok);

    let larger = JianCallbacks {
        size: size_of::<JianCallbacks>() + 8,
        user_data: ptr::null_mut(),
        needs_redraw: None,
        runtime_error: None,
        ime_control: None,
        input_focus_changed: None,
        text_state_changed: None,
    };
    desc.callbacks = &larger;
    assert_eq!(unsafe { create(&desc) }, Err(JianStatus::InvalidArg));
}

#[test]
fn create_validation_uses_thread_local_last_error() {
    let mut invalid = exact_desc(b"not json", 10.0, 8.0, 1.0);
    assert_eq!(unsafe { create(&invalid) }, Err(JianStatus::BadDocument));
    assert!(thread_error().contains("document"));

    invalid = exact_desc(DOC, 10.0, 8.0, f32::NAN);
    assert_eq!(unsafe { create(&invalid) }, Err(JianStatus::InvalidArg));
    assert!(thread_error().contains("dpr"));

    invalid = exact_desc(DOC, 10.0, 8.0, 1.0);
    invalid.doc_ptr = ptr::null();
    assert_eq!(unsafe { create(&invalid) }, Err(JianStatus::InvalidArg));

    invalid = exact_desc(DOC, 10.0, 8.0, 1.0);
    invalid.doc_ptr = ptr::NonNull::<u8>::dangling().as_ptr();
    invalid.doc_len = 256 * 1024 * 1024 + 1;
    assert_eq!(unsafe { create(&invalid) }, Err(JianStatus::InvalidArg));

    let bad_utf8 = [0xff];
    invalid = exact_desc(DOC, 10.0, 8.0, 1.0);
    invalid.storage_dir_ptr = bad_utf8.as_ptr();
    invalid.storage_dir_len = bad_utf8.len();
    assert_eq!(unsafe { create(&invalid) }, Err(JianStatus::InvalidArg));
}

#[test]
fn owner_thread_check_is_unconditional() {
    let engine = unsafe { create(&exact_desc(DOC, 10.0, 8.0, 1.0)).unwrap() };
    let address = engine as usize;
    let status = std::thread::spawn(move || {
        let mut width = 0;
        let mut height = 0;
        unsafe { jian_get_pixel_size(address as *mut JianEngine, &mut width, &mut height) }
    })
    .join()
    .unwrap();
    assert_eq!(status, JianStatus::WrongThread);
    assert_eq!(unsafe { destroy(engine) }, JianStatus::Ok);
}

#[test]
#[cfg(debug_assertions)]
fn panic_poisoning_allows_only_destroy() {
    let engine = unsafe { create(&exact_desc(DOC, 10.0, 8.0, 1.0)).unwrap() };
    let panic_call: unsafe extern "C" fn(*mut JianEngine) -> JianStatus = jian_test_force_panic;
    assert_eq!(unsafe { panic_call(engine) }, JianStatus::Poisoned);
    assert_eq!(
        unsafe { jian_resize(engine, 20.0, 20.0, 1.0) },
        JianStatus::Poisoned
    );
    assert_eq!(unsafe { destroy(engine) }, JianStatus::Ok);
}

struct ReentryContext {
    engine: AtomicUsize,
    status: AtomicI32,
}

unsafe extern "C" fn reenter_from_callback(
    user_data: *mut c_void,
    _has_next_wake: bool,
    _next_wake_ms: u64,
) {
    let context = unsafe { &*(user_data.cast::<ReentryContext>()) };
    let engine = context.engine.load(Ordering::SeqCst);
    if engine == 0 {
        return;
    }
    let mut width = 0;
    let mut height = 0;
    let status = unsafe { jian_get_pixel_size(engine as *mut JianEngine, &mut width, &mut height) };
    context.status.store(status as i32, Ordering::SeqCst);
}

#[test]
fn synchronous_callback_reentry_is_wrong_thread() {
    let context = ReentryContext {
        engine: AtomicUsize::new(0),
        status: AtomicI32::new(-1),
    };
    let callbacks = JianCallbacks {
        size: size_of::<JianCallbacks>(),
        user_data: (&context as *const ReentryContext).cast_mut().cast(),
        needs_redraw: Some(reenter_from_callback),
        runtime_error: None,
        ime_control: None,
        input_focus_changed: None,
        text_state_changed: None,
    };
    let mut desc = exact_desc(DOC, 10.0, 8.0, 1.0);
    desc.callbacks = &callbacks;
    let engine = unsafe { create(&desc).unwrap() };
    context.engine.store(engine as usize, Ordering::SeqCst);

    assert_eq!(
        unsafe { jian_resize(engine, 12.0, 9.0, 1.0) },
        JianStatus::Ok
    );
    assert_eq!(
        context.status.load(Ordering::SeqCst),
        JianStatus::WrongThread as i32
    );
    assert_eq!(unsafe { destroy(engine) }, JianStatus::Ok);
}
