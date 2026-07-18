//! Registered-image rendering through the FFI CPU frame path.
//!
//! Regression (M4 plan Task 3c): the FFI painted through a bare
//! `SkiaBackend`, whose `register_image` drops the bytes after validation —
//! a successfully resolved local/remote image could never RENDER through the
//! C ABI, only the placeholder. The fix routes registration and keyed draws
//! through the desktop-proven `RegisteredBackend` + `InstanceImageRegistry`
//! composition.

use jian_engine_ffi::{
    jian_capability_result, jian_create, jian_destroy, jian_frame_cpu, JianCallbacks,
    JianCapabilityKind, JianCapabilityRequest, JianCapabilityResult, JianCapabilityResultData,
    JianCreateDesc, JianEngine, JianImageFetchResult, JianStatus,
};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::Mutex;

/// 1x1 solid-red PNG.
const RED_PNG: [u8; 69] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xC9, 0xFE, 0x92, 0xEF, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

const WIDTH: usize = 32;
const HEIGHT: usize = 32;

#[test]
fn resolved_local_image_renders_through_the_cpu_frame() {
    // Unique asset dir under the system temp root; std-only.
    let dir = std::env::temp_dir().join(format!(
        "jian-ffi-images-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hero.png"), RED_PNG).unwrap();
    let asset_base = dir.to_str().unwrap().as_bytes().to_vec();

    // The image covers the whole 32x32 surface so any interior pixel probes it.
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "children":[{"type":"frame","id":"root","width":32,"height":32,
        "children":[{"type":"image","id":"hero","x":0,"y":0,"width":32,"height":32,
                     "src":"hero.png"}]}]
    }"#;

    let desc = JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: DOCUMENT.as_ptr(),
        doc_len: DOCUMENT.len(),
        width: WIDTH as f32,
        height: HEIGHT as f32,
        dpr: 1.0,
        storage_dir_ptr: ptr::null(),
        storage_dir_len: 0,
        callbacks: ptr::null(),
        asset_base_ptr: asset_base.as_ptr(),
        asset_base_len: asset_base.len(),
    };
    let create: unsafe extern "C" fn(*const JianCreateDesc, *mut *mut JianEngine) -> JianStatus =
        jian_create;
    let mut engine = ptr::null_mut();
    assert_eq!(unsafe { create(&desc, &mut engine) }, JianStatus::Ok);

    let frame: unsafe extern "C" fn(*mut JianEngine, u64, *mut u8, usize, usize) -> JianStatus =
        jian_frame_cpu;
    let stride = WIDTH * 4;
    let mut buffer = vec![0u8; stride * HEIGHT];

    // The local read resolves through the async image pipeline: pump a few
    // frames so admit -> resolve -> register -> keyed draw all complete.
    let mut center = [0u8; 4];
    for tick in 1..=5u64 {
        assert_eq!(
            unsafe { frame(engine, tick * 16, buffer.as_mut_ptr(), buffer.len(), stride) },
            JianStatus::Ok
        );
        let offset = (HEIGHT / 2) * stride + (WIDTH / 2) * 4;
        center.copy_from_slice(&buffer[offset..offset + 4]);
        if center[0] > 200 && center[1] < 60 && center[2] < 60 {
            break;
        }
    }

    assert!(
        center[0] > 200 && center[1] < 60 && center[2] < 60,
        "the resolved local PNG must paint red through the CPU frame \
         (registered bytes must reach the keyed draw), got RGBA {center:?}"
    );

    unsafe { jian_destroy(engine) };
    let _ = std::fs::remove_dir_all(&dir);
}

/// Requests captured from the ImageFetch capability callback.
static IMAGE_REQUESTS: Mutex<Vec<u64>> = Mutex::new(Vec::new());

unsafe extern "C" fn record_image_request(
    _user_data: *mut c_void,
    request_id: u64,
    request: *const JianCapabilityRequest,
) {
    if unsafe { &*request }.kind == JianCapabilityKind::ImageFetch {
        IMAGE_REQUESTS.lock().unwrap().push(request_id);
    }
}

/// A REMOTE image resolved through the capability round-trip must reach the
/// screen exactly like the local one above.
///
/// Regression (found on the M4 Android player): the host delivers the fetched
/// bytes, `jian_capability_result` returns `Ok`, and no warning is raised —
/// but the node keeps painting its placeholder, because the resolved bytes
/// never reach a painted frame on their own.
#[test]
fn resolved_remote_image_renders_through_the_cpu_frame() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "app":{"name":"ffi images","version":"1.0.0","id":"dev.jian.test.images",
             "capabilities":["network"]},
      "children":[{"type":"frame","id":"root","width":32,"height":32,
        "children":[{"type":"image","id":"hero","x":0,"y":0,"width":32,"height":32,
                     "src":"https://images.test/hero.png"}]}]
    }"#;

    IMAGE_REQUESTS.lock().unwrap().clear();
    let callbacks = JianCallbacks {
        size: size_of::<JianCallbacks>(),
        user_data: ptr::null_mut(),
        needs_redraw: None,
        runtime_error: None,
        ime_control: None,
        input_focus_changed: None,
        text_state_changed: None,
        capability_request: Some(record_image_request),
        capability_cancelled: None,
    };
    let desc = JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: DOCUMENT.as_ptr(),
        doc_len: DOCUMENT.len(),
        width: WIDTH as f32,
        height: HEIGHT as f32,
        dpr: 1.0,
        storage_dir_ptr: ptr::null(),
        storage_dir_len: 0,
        callbacks: &callbacks,
        asset_base_ptr: ptr::null(),
        asset_base_len: 0,
    };
    let create: unsafe extern "C" fn(*const JianCreateDesc, *mut *mut JianEngine) -> JianStatus =
        jian_create;
    let mut engine = ptr::null_mut();
    assert_eq!(unsafe { create(&desc, &mut engine) }, JianStatus::Ok);

    let frame: unsafe extern "C" fn(*mut JianEngine, u64, *mut u8, usize, usize) -> JianStatus =
        jian_frame_cpu;
    let stride = WIDTH * 4;
    let mut buffer = vec![0u8; stride * HEIGHT];
    let mut tick = 0u64;
    let paint = |engine, tick: u64, buffer: &mut Vec<u8>| {
        assert_eq!(
            unsafe { frame(engine, tick * 16, buffer.as_mut_ptr(), buffer.len(), stride) },
            JianStatus::Ok
        );
    };

    // Frames until the engine asks the host to fetch the remote source.
    for _ in 0..5 {
        tick += 1;
        paint(engine, tick, &mut buffer);
        if !IMAGE_REQUESTS.lock().unwrap().is_empty() {
            break;
        }
    }
    let request_id = *IMAGE_REQUESTS
        .lock()
        .unwrap()
        .first()
        .expect("the engine must emit an ImageFetch request for a remote src");

    // The host answers exactly as the Android player does.
    let result = JianCapabilityResult {
        size: size_of::<JianCapabilityResult>(),
        kind: JianCapabilityKind::ImageFetch as i32,
        data: JianCapabilityResultData {
            image_fetch: JianImageFetchResult {
                ok: true,
                bytes_ptr: RED_PNG.as_ptr(),
                bytes_len: RED_PNG.len(),
                error_ptr: ptr::null(),
                error_len: 0,
            },
        },
    };
    let deliver: unsafe extern "C" fn(
        *mut JianEngine,
        u64,
        *const JianCapabilityResult,
    ) -> JianStatus = jian_capability_result;
    assert_eq!(
        unsafe { deliver(engine, request_id, &result) },
        JianStatus::Ok,
        "the engine must accept the fetched bytes"
    );

    // Exactly ONE frame — all a host owes after delivering a result. The
    // engine polls the resolver future and consumes the completion in the same
    // pump, so the bytes must be on screen when this frame returns. Needing a
    // second frame is the bug: nothing asks for one, so the image would stay a
    // placeholder until unrelated input happened to wake the host.
    tick += 1;
    paint(engine, tick, &mut buffer);
    let offset = (HEIGHT / 2) * stride + (WIDTH / 2) * 4;
    let mut center = [0u8; 4];
    center.copy_from_slice(&buffer[offset..offset + 4]);

    assert!(
        center[0] > 200 && center[1] < 60 && center[2] < 60,
        "the fetched remote PNG must paint red in the FIRST frame after delivery, \
         got RGBA {center:?}"
    );

    unsafe { jian_destroy(engine) };
}

/// The multi-node acceptance document differs from the isolated one in a way
/// worth pinning: a SECOND remote image whose request never completes. If a
/// still-pending sibling can stall the resolved one, an app would show a
/// permanent placeholder next to a hung download.
#[test]
fn a_pending_sibling_does_not_stall_the_resolved_image() {
    const DOCUMENT: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2","responsive":true,
      "app":{"name":"ffi images","version":"1.0.0","id":"dev.jian.test.images2",
             "capabilities":["network"]},
      "children":[{"type":"frame","id":"root","width":32,"height":32,
        "children":[{"type":"image","id":"hero","x":0,"y":0,"width":32,"height":32,
                     "src":"https://images.test/hero.png"},
                    {"type":"image","id":"hang","x":0,"y":0,"width":1,"height":1,
                     "src":"https://images.test/never.png"}]}]
    }"#;

    IMAGE_REQUESTS.lock().unwrap().clear();
    let callbacks = JianCallbacks {
        size: size_of::<JianCallbacks>(),
        user_data: ptr::null_mut(),
        needs_redraw: None,
        runtime_error: None,
        ime_control: None,
        input_focus_changed: None,
        text_state_changed: None,
        capability_request: Some(record_image_request),
        capability_cancelled: None,
    };
    let desc = JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: DOCUMENT.as_ptr(),
        doc_len: DOCUMENT.len(),
        width: WIDTH as f32,
        height: HEIGHT as f32,
        dpr: 1.0,
        storage_dir_ptr: ptr::null(),
        storage_dir_len: 0,
        callbacks: &callbacks,
        asset_base_ptr: ptr::null(),
        asset_base_len: 0,
    };
    let create: unsafe extern "C" fn(*const JianCreateDesc, *mut *mut JianEngine) -> JianStatus =
        jian_create;
    let mut engine = ptr::null_mut();
    assert_eq!(unsafe { create(&desc, &mut engine) }, JianStatus::Ok);

    let frame: unsafe extern "C" fn(*mut JianEngine, u64, *mut u8, usize, usize) -> JianStatus =
        jian_frame_cpu;
    let stride = WIDTH * 4;
    let mut buffer = vec![0u8; stride * HEIGHT];
    let mut tick = 0u64;

    // Both requests go out; only `hero` is ever answered.
    for _ in 0..5 {
        tick += 1;
        assert_eq!(
            unsafe { frame(engine, tick * 16, buffer.as_mut_ptr(), buffer.len(), stride) },
            JianStatus::Ok
        );
        if IMAGE_REQUESTS.lock().unwrap().len() >= 2 {
            break;
        }
    }
    let requests = IMAGE_REQUESTS.lock().unwrap().clone();
    assert!(
        requests.len() >= 2,
        "both remote sources must be requested, got {requests:?}"
    );
    let hero = requests[0];

    let result = JianCapabilityResult {
        size: size_of::<JianCapabilityResult>(),
        kind: JianCapabilityKind::ImageFetch as i32,
        data: JianCapabilityResultData {
            image_fetch: JianImageFetchResult {
                ok: true,
                bytes_ptr: RED_PNG.as_ptr(),
                bytes_len: RED_PNG.len(),
                error_ptr: ptr::null(),
                error_len: 0,
            },
        },
    };
    let deliver: unsafe extern "C" fn(
        *mut JianEngine,
        u64,
        *const JianCapabilityResult,
    ) -> JianStatus = jian_capability_result;
    assert_eq!(unsafe { deliver(engine, hero, &result) }, JianStatus::Ok);

    tick += 1;
    assert_eq!(
        unsafe { frame(engine, tick * 16, buffer.as_mut_ptr(), buffer.len(), stride) },
        JianStatus::Ok
    );
    let offset = (HEIGHT / 2) * stride + (WIDTH / 2) * 4;
    let mut center = [0u8; 4];
    center.copy_from_slice(&buffer[offset..offset + 4]);

    assert!(
        center[0] > 200 && center[1] < 60 && center[2] < 60,
        "the answered image must paint even while a sibling request is still \
         in flight, got RGBA {center:?}"
    );

    unsafe { jian_destroy(engine) };
}
