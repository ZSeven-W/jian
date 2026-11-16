use jian_engine_ffi::{
    jian_create, jian_destroy, jian_engine_set_reduce_motion, jian_frame_cpu, jian_pointer,
    JianCallbacks, JianCreateDesc, JianEngine, JianPointerPhase, JianStatus,
};
use std::cell::Cell;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

const DOC: &str = include_str!("../../jian-core/tests/fixtures/motion_opacity.op");

unsafe extern "C" fn redraw(user_data: *mut c_void, has_wake: bool, next: u64) {
    let slot = unsafe { &*user_data.cast::<Cell<Option<u64>>>() };
    slot.set(has_wake.then_some(next));
}

fn paint(engine: *mut JianEngine, now: u64) -> Vec<u8> {
    let mut rgba = vec![0u8; 64 * 64 * 4];
    assert_eq!(
        unsafe { jian_frame_cpu(engine, now, rgba.as_mut_ptr(), rgba.len(), 64 * 4) },
        JianStatus::Ok
    );
    rgba
}

fn create(source: &str, wake: &Cell<Option<u64>>) -> *mut JianEngine {
    let callbacks = JianCallbacks {
        size: size_of::<JianCallbacks>(),
        user_data: (wake as *const Cell<Option<u64>>).cast_mut().cast(),
        needs_redraw: Some(redraw),
        runtime_error: None,
        ime_control: None,
        input_focus_changed: None,
        text_state_changed: None,
        capability_request: None,
        capability_cancelled: None,
    };
    let desc = JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: source.as_ptr(),
        doc_len: source.len(),
        width: 64.0,
        height: 64.0,
        dpr: 1.0,
        storage_dir_ptr: ptr::null(),
        storage_dir_len: 0,
        callbacks: &callbacks,
        asset_base_ptr: ptr::null(),
        asset_base_len: 0,
    };
    let mut engine = ptr::null_mut();
    assert_eq!(unsafe { jian_create(&desc, &mut engine) }, JianStatus::Ok);
    engine
}

#[test]
fn embedded_frames_composite_subtree_opacity_and_arm_motion_deadlines() {
    let wake = Cell::new(None);
    let engine = create(DOC, &wake);
    let pixel = |x: usize| (8 * 64 + x) * 4;
    let first = paint(engine, 100);
    assert_eq!(first[pixel(8)], 255);
    assert_eq!(wake.get(), Some(116));
    let middle = paint(engine, 180);
    assert!((126..=129).contains(&middle[pixel(8)]));
    assert_eq!(
        middle[pixel(8)],
        middle[pixel(14)],
        "overlap must composite once"
    );
    assert_eq!(wake.get(), Some(196));
    assert_eq!(
        unsafe { jian_engine_set_reduce_motion(engine, true) },
        JianStatus::Ok
    );
    let reduced = paint(engine, 181);
    assert_eq!(reduced[pixel(14)], 0);
    assert_eq!(wake.get(), None);
    assert_eq!(paint(engine, 260), reduced);
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn delayed_navigation_starts_in_the_same_frame_and_uses_the_legacy_surface_size() {
    let source = r##"{"version":"1.1","children":[
      {"type":"frame","id":"home","screen":"/","width":64,"height":64,
        "fill":[{"type":"solid","color":"#ff0000"}],"children":[
          {"type":"rectangle","id":"go","x":8,"y":8,"width":32,"height":32,
            "fill":[{"type":"solid","color":"#ff0000"}],
            "events":{"onTap":[{"delay":{"ms":40}},{"push":{
              "path":"'/detail'","transition":{"kind":"slideUp","durationMs":160,"easing":"linear"}
            }}]}}
        ]},
      {"type":"frame","id":"detail","screen":"/detail","x":1000,"width":64,"height":64,
        "fill":[{"type":"solid","color":"#0000ff"}]}
    ]}"##;
    let wake = Cell::new(None);
    let engine = create(source, &wake);
    let pixel = |y: usize| (y * 64 + 32) * 4;
    assert_eq!(paint(engine, 0)[pixel(48)], 255);

    for phase in [JianPointerPhase::Down, JianPointerPhase::Up] {
        assert_eq!(
            unsafe { jian_pointer(engine, 1, phase as i32, 16.0, 16.0, 10) },
            JianStatus::Ok
        );
    }

    assert_eq!(paint(engine, 10)[pixel(48)], 255);
    assert_eq!(wake.get(), Some(50));
    assert_eq!(paint(engine, 50)[pixel(48)], 255);
    assert_eq!(
        wake.get(),
        Some(66),
        "the final async task must arm the transition"
    );
    let middle = paint(engine, 130);
    assert_eq!(&middle[pixel(16)..pixel(16) + 4], &[255, 0, 0, 255]);
    assert_eq!(&middle[pixel(48)..pixel(48) + 4], &[0, 0, 255, 255]);
    assert_eq!(
        &paint(engine, 210)[pixel(16)..pixel(16) + 4],
        &[0, 0, 255, 255]
    );
    assert_eq!(wake.get(), None);
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}
