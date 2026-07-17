//! §6.5 font-registration invalidation fanout through the C ABI.
//!
//! Regression (M4 plan Task 3d): `jian_register_font` mutated the
//! process-global registry but neither relayouted the registering engine nor
//! gave idle engines a pump-side generation check — committed trees kept
//! stale glyph geometry indefinitely.

#![cfg(all(debug_assertions, feature = "textlayout"))]

use jian_engine_ffi::{
    jian_create, jian_destroy, jian_frame_cpu, jian_register_font, jian_test_node_rect,
    JianCreateDesc, JianEngine, JianRect, JianStatus,
};
use std::mem::size_of;
use std::ptr;

const ROBOTO: &[u8] = include_bytes!("../../jian-host-web/assets/fonts/Roboto-Regular.ttf");

// A fit-content text in a family that is unknown before registration (system
// fallback) and Roboto afterwards — its intrinsic width is the metric probe.
const DOCUMENT: &[u8] = br#"{
  "version":"1.2","formatVersion":"1.2","responsive":true,
  "children":[{"type":"frame","id":"root","width":400,"height":120,
    "children":[{"type":"text","id":"probe","x":10,"y":10,
                 "width":"fit_content","height":"fit_content",
                 "fontFamily":"Roboto","fontSize":24,
                 "content":"Millimeter jelly waffle 1234567890"}]}]
}"#;

unsafe fn create_engine() -> *mut JianEngine {
    let desc = JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: DOCUMENT.as_ptr(),
        doc_len: DOCUMENT.len(),
        width: 400.0,
        height: 120.0,
        dpr: 1.0,
        storage_dir_ptr: ptr::null(),
        storage_dir_len: 0,
        callbacks: ptr::null(),
        asset_base_ptr: ptr::null(),
        asset_base_len: 0,
    };
    let create: unsafe extern "C" fn(*const JianCreateDesc, *mut *mut JianEngine) -> JianStatus =
        jian_create;
    let mut engine = ptr::null_mut();
    assert_eq!(unsafe { create(&desc, &mut engine) }, JianStatus::Ok);
    engine
}

unsafe fn probe_width(engine: *mut JianEngine) -> f32 {
    let node_rect: unsafe extern "C" fn(
        *mut JianEngine,
        *const u8,
        usize,
        *mut JianRect,
    ) -> JianStatus = jian_test_node_rect;
    let mut rect = JianRect::default();
    assert_eq!(
        unsafe { node_rect(engine, b"probe".as_ptr(), 5, &mut rect) },
        JianStatus::Ok
    );
    rect.width
}

unsafe fn pump_frame(engine: *mut JianEngine, now_ms: u64) {
    let frame: unsafe extern "C" fn(*mut JianEngine, u64, *mut u8, usize, usize) -> JianStatus =
        jian_frame_cpu;
    let stride = 400 * 4;
    let mut buffer = vec![0u8; stride * 120];
    assert_eq!(
        unsafe { frame(engine, now_ms, buffer.as_mut_ptr(), buffer.len(), stride) },
        JianStatus::Ok
    );
}

#[test]
fn font_registration_fans_out_to_registering_and_idle_engines() {
    let registering = unsafe { create_engine() };
    let idle = unsafe { create_engine() };

    let before_registering = unsafe { probe_width(registering) };
    let before_idle = unsafe { probe_width(idle) };
    assert!(before_registering > 0.0 && before_idle > 0.0);

    let register: unsafe extern "C" fn(*mut JianEngine, *const u8, usize) -> JianStatus =
        jian_register_font;
    assert_eq!(
        unsafe { register(registering, ROBOTO.as_ptr(), ROBOTO.len()) },
        JianStatus::Ok
    );

    // §6.5: the REGISTERING engine relayouts immediately — no frame needed.
    let after_registering = unsafe { probe_width(registering) };
    assert!(
        (after_registering - before_registering).abs() > 0.5,
        "the registering engine must re-measure immediately after \
         jian_register_font (width stayed {before_registering})"
    );

    // §6.5: every OTHER engine catches the generation in its next pump.
    let after_idle = unsafe { probe_width(idle) };
    unsafe { pump_frame(idle, 32) };
    let after_idle_pump = unsafe { probe_width(idle) };
    assert!(
        (after_idle_pump - before_idle).abs() > 0.5,
        "an idle engine's next pump must catch the font generation and \
         relayout (width stayed {before_idle}, pre-pump {after_idle})"
    );
    // Both engines now measure with the same registered face, so their
    // widths agree — a secondary signal that guards against fallback-metric
    // coincidences. (Documented caveat: a host with Roboto installed as a
    // SYSTEM font would collapse the before/after delta; macOS does not
    // ship Roboto.)
    assert!(
        (after_registering - after_idle_pump).abs() < 0.01,
        "both engines must converge on the registered face \
         ({after_registering} vs {after_idle_pump})"
    );

    unsafe {
        jian_destroy(registering);
        jian_destroy(idle);
    }
}
