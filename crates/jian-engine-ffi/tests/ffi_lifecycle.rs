use jian_engine_ffi::{
    jian_attach_surface, jian_create, jian_destroy, jian_frame, jian_frame_cpu,
    jian_get_pixel_size, jian_pointer, jian_resize, jian_resume, jian_set_keyboard,
    jian_set_safe_area, jian_suspend, JianCreateDesc, JianEngine, JianPointerPhase, JianStatus,
    JianSurfaceDesc,
};
#[cfg(debug_assertions)]
use jian_engine_ffi::{
    jian_test_get_insets, jian_test_suspended_status, JianInsets, JianTestCallClass,
};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

const DOC: &[u8] = br#"{"version":"0.8.0","children":[]}"#;

fn desc(width: f32, height: f32, dpr: f32) -> JianCreateDesc {
    JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: DOC.as_ptr(),
        doc_len: DOC.len(),
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

unsafe fn create(width: f32, height: f32, dpr: f32) -> *mut JianEngine {
    let mut engine = ptr::null_mut();
    assert_eq!(
        unsafe { jian_create(&desc(width, height, dpr), &mut engine) },
        JianStatus::Ok
    );
    engine
}

unsafe fn pixels(engine: *mut JianEngine) -> (u32, u32) {
    let mut width = 0;
    let mut height = 0;
    assert_eq!(
        unsafe { jian_get_pixel_size(engine, &mut width, &mut height) },
        JianStatus::Ok
    );
    (width, height)
}

#[test]
fn resize_rejects_nonfinite_and_over_cap_values_without_mutation() {
    let engine = unsafe { create(100.0, 80.0, 2.0) };
    assert_eq!(unsafe { pixels(engine) }, (200, 160));

    for (width, height, dpr) in [
        (f32::NAN, 80.0, 2.0),
        (100.0, f32::INFINITY, 2.0),
        (100.0, 80.0, 0.0),
        (100.0, 80.0, 16.1),
        (8192.0, 8192.0, 1.0),
        (20_000.0, 1.0, 1.0),
    ] {
        assert_eq!(
            unsafe { jian_resize(engine, width, height, dpr) },
            JianStatus::InvalidArg
        );
        assert_eq!(unsafe { pixels(engine) }, (200, 160));
    }
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn frame_without_attach_is_invalid_and_failed_attach_selects_no_mode() {
    let engine = unsafe { create(4.0, 3.0, 1.0) };
    assert_eq!(unsafe { jian_frame(engine, 1) }, JianStatus::InvalidArg);
    let invalid_surface = JianSurfaceDesc {
        size: size_of::<JianSurfaceDesc>(),
        handle: ptr::null_mut(),
    };
    assert_eq!(
        unsafe { jian_attach_surface(engine, &invalid_surface) },
        JianStatus::InvalidArg
    );

    let mut buffer = vec![0; 4 * 3 * 4];
    assert_eq!(
        unsafe { jian_frame_cpu(engine, 2, buffer.as_mut_ptr(), buffer.len(), 16) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn cpu_frame_validates_buffer_stride_length_and_overflow_before_selecting_mode() {
    let engine = unsafe { create(4.0, 3.0, 1.0) };
    let mut short = vec![0; 47];
    assert_eq!(
        unsafe { jian_frame_cpu(engine, 1, ptr::null_mut(), 0, 16) },
        JianStatus::InvalidArg
    );
    assert_eq!(
        unsafe { jian_frame_cpu(engine, 1, short.as_mut_ptr(), short.len(), 15) },
        JianStatus::InvalidArg
    );
    assert_eq!(
        unsafe { jian_frame_cpu(engine, 1, short.as_mut_ptr(), short.len(), 16) },
        JianStatus::InvalidArg
    );
    assert_eq!(
        unsafe {
            jian_frame_cpu(
                engine,
                1,
                ptr::NonNull::<u8>::dangling().as_ptr(),
                usize::MAX,
                usize::MAX,
            )
        },
        JianStatus::InvalidArg
    );

    let stride = 20;
    let mut output = vec![0x7f; stride * 3];
    assert_eq!(
        unsafe { jian_frame_cpu(engine, 2, output.as_mut_ptr(), output.len(), stride) },
        JianStatus::Ok
    );
    for row in output.chunks_exact(stride) {
        assert!(row[..16].chunks_exact(4).all(|pixel| pixel == [255; 4]));
        assert_eq!(&row[16..], &[0x7f; 4]);
    }

    let surface = JianSurfaceDesc {
        size: size_of::<JianSurfaceDesc>(),
        handle: ptr::NonNull::<c_void>::dangling().as_ptr(),
    };
    assert_eq!(
        unsafe { jian_attach_surface(engine, &surface) },
        JianStatus::InvalidArg
    );
    assert_eq!(unsafe { jian_frame(engine, 3) }, JianStatus::InvalidArg);
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
#[cfg(debug_assertions)]
fn suspended_table_is_enforced_through_ffi_calls() {
    let engine = unsafe { create(4.0, 3.0, 1.0) };
    let mut first = vec![0; 4 * 3 * 4];
    assert_eq!(
        unsafe { jian_frame_cpu(engine, 1, first.as_mut_ptr(), first.len(), 16) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { jian_suspend(engine) }, JianStatus::Ok);
    assert_eq!(unsafe { jian_suspend(engine) }, JianStatus::Ok);

    let mut untouched = vec![0x33; first.len()];
    let untouched_len = untouched.len();
    assert_eq!(unsafe { jian_frame(engine, 2) }, JianStatus::Suspended);
    assert_eq!(
        unsafe { jian_frame_cpu(engine, 2, untouched.as_mut_ptr(), untouched_len, 16,) },
        JianStatus::Suspended
    );
    assert!(untouched.iter().all(|byte| *byte == 0x33));
    assert_eq!(
        unsafe { jian_pointer(engine, 1, JianPointerPhase::Down as i32, 1.0, 1.0, 2,) },
        JianStatus::Suspended
    );

    assert_eq!(
        unsafe { jian_resize(engine, 5.0, 6.0, 2.0) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { pixels(engine) }, (10, 12));
    assert_eq!(
        unsafe { jian_set_safe_area(engine, 1.0, 1.0, 1.0, 1.0) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { jian_set_keyboard(engine, 2.0) }, JianStatus::Ok);
    for class in [
        JianTestCallClass::TextContent,
        JianTestCallClass::ImeText,
        JianTestCallClass::CapabilityResult,
        JianTestCallClass::RegisterFont,
    ] {
        assert_eq!(
            unsafe { jian_test_suspended_status(engine, class) },
            JianStatus::Ok
        );
    }
    assert_eq!(
        unsafe { jian_test_suspended_status(engine, JianTestCallClass::TextGeometry) },
        JianStatus::NotReady
    );

    let invalid_cpu_surface = JianSurfaceDesc {
        size: size_of::<JianSurfaceDesc>(),
        handle: ptr::NonNull::<c_void>::dangling().as_ptr(),
    };
    assert_eq!(
        unsafe { jian_resume(engine, &invalid_cpu_surface) },
        JianStatus::InvalidArg
    );
    assert_eq!(unsafe { jian_resume(engine, ptr::null()) }, JianStatus::Ok);
    let mut resumed = vec![0; 10 * 12 * 4];
    let resumed_len = resumed.len();
    assert_eq!(
        unsafe { jian_frame_cpu(engine, 3, resumed.as_mut_ptr(), resumed_len, 40,) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
#[cfg(debug_assertions)]
fn safe_area_and_keyboard_validate_then_clamp_on_later_resize() {
    let engine = unsafe { create(100.0, 80.0, 1.0) };
    for status in [
        unsafe { jian_set_safe_area(engine, -1.0, 0.0, 0.0, 0.0) },
        unsafe { jian_set_safe_area(engine, f32::NAN, 0.0, 0.0, 0.0) },
        unsafe { jian_set_safe_area(engine, 50.0, 0.0, 40.0, 0.0) },
        unsafe { jian_set_safe_area(engine, 0.0, 60.0, 0.0, 50.0) },
        unsafe { jian_set_keyboard(engine, -1.0) },
        unsafe { jian_set_keyboard(engine, 81.0) },
    ] {
        assert_eq!(status, JianStatus::InvalidArg);
    }
    assert_eq!(
        unsafe { jian_set_safe_area(engine, 30.0, 40.0, 20.0, 30.0) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { jian_set_keyboard(engine, 70.0) }, JianStatus::Ok);
    assert_eq!(
        unsafe { jian_resize(engine, 50.0, 40.0, 1.0) },
        JianStatus::Ok
    );

    let mut insets = JianInsets::default();
    let mut keyboard = 0.0;
    assert_eq!(
        unsafe { jian_test_get_insets(engine, &mut insets, &mut keyboard) },
        JianStatus::Ok
    );
    assert!((insets.top + insets.bottom - 40.0).abs() < 0.001);
    assert!((insets.left + insets.right - 50.0).abs() < 0.001);
    assert_eq!(keyboard, 40.0);
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn suspend_and_resume_before_mode_selection_are_null_noops() {
    let engine = unsafe { create(4.0, 3.0, 1.0) };
    assert_eq!(unsafe { jian_suspend(engine) }, JianStatus::Ok);
    assert_eq!(unsafe { jian_resume(engine, ptr::null()) }, JianStatus::Ok);
    assert_eq!(unsafe { jian_frame(engine, 1) }, JianStatus::InvalidArg);
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}

#[test]
fn pointer_uses_int32_phase_and_validates_numeric_input() {
    let engine = unsafe { create(20.0, 20.0, 1.0) };
    assert_eq!(
        unsafe { jian_pointer(engine, 7, 99, 1.0, 1.0, 1) },
        JianStatus::InvalidArg
    );
    assert_eq!(
        unsafe { jian_pointer(engine, 7, JianPointerPhase::Down as i32, f32::NAN, 1.0, 1,) },
        JianStatus::InvalidArg
    );
    assert_eq!(
        unsafe { jian_pointer(engine, 7, JianPointerPhase::Down as i32, 2.0e6, -2.0e6, 1,) },
        JianStatus::Ok
    );
    assert_eq!(unsafe { jian_destroy(engine) }, JianStatus::Ok);
}
