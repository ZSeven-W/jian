//! Registered-image rendering through the FFI CPU frame path.
//!
//! Regression (M4 plan Task 3c): the FFI painted through a bare
//! `SkiaBackend`, whose `register_image` drops the bytes after validation —
//! a successfully resolved local/remote image could never RENDER through the
//! C ABI, only the placeholder. The fix routes registration and keyed draws
//! through the desktop-proven `RegisteredBackend` + `InstanceImageRegistry`
//! composition.

use jian_engine_ffi::{
    jian_create, jian_destroy, jian_frame_cpu, JianCreateDesc, JianEngine, JianStatus,
};
use std::mem::size_of;
use std::ptr;

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
