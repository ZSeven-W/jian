//! Edited text must REACH THE SCREEN through the C ABI's frame path.
//!
//! Regression: every production host collected its scene through a
//! `..._with_state` collector, which passes no widget context, so
//! `emit_live_text_input` was never reached and a `text_input` always painted
//! the schema's authored `value`. Typing changed `nativeTextGetState` and
//! nothing else — on Android the engine reported `mobilea ` while the screen
//! kept showing `mobile`, through a commit AND a full surface rebuild. The
//! widget-aware collector existed but only tests called it.

use jian_engine_ffi::{
    jian_create, jian_destroy, jian_frame_cpu, jian_pointer, jian_text_insert, JianCreateDesc,
    JianEngine, JianPointerPhase, JianStatus,
};
use std::mem::size_of;
use std::ptr;

const WIDTH: usize = 200;
const HEIGHT: usize = 40;

/// A `text_input` filling the surface, starting empty.
const DOCUMENT: &[u8] = br#"{
  "version":"1.2","formatVersion":"1.2",
  "children":[{"type":"frame","id":"root","width":200,"height":40,
    "children":[{"type":"text_input","id":"field","x":0,"y":0,
                 "width":200,"height":40,"value":""}]}]
}"#;

/// Pixels dark enough to be a glyph, over the light input background.
fn ink(buffer: &[u8], stride: usize) -> usize {
    let mut count = 0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let p = y * stride + x * 4;
            // BGRA/RGBA agnostic: a glyph is dark in every colour channel.
            if buffer[p] < 128 && buffer[p + 1] < 128 && buffer[p + 2] < 128 {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn typed_text_is_painted_by_the_cpu_frame() {
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
    let mut paint = |engine, buffer: &mut Vec<u8>, tick: u64| {
        assert_eq!(
            unsafe { frame(engine, tick * 16, buffer.as_mut_ptr(), buffer.len(), stride) },
            JianStatus::Ok
        );
    };

    // Focus the field, so the comparison below differs ONLY by typed text and
    // never by the focus ring or the caret appearing.
    let pointer: unsafe extern "C" fn(*mut JianEngine, u32, i32, f32, f32, u64) -> JianStatus =
        jian_pointer;
    assert_eq!(
        unsafe { pointer(engine, 1, JianPointerPhase::Down as i32, 10.0, 20.0, 0) },
        JianStatus::Ok
    );
    assert_eq!(
        unsafe { pointer(engine, 1, JianPointerPhase::Up as i32, 10.0, 20.0, 8) },
        JianStatus::Ok
    );
    tick += 1;
    paint(engine, &mut buffer, tick);
    let empty_ink = ink(&buffer, stride);

    let insert: unsafe extern "C" fn(*mut JianEngine, *const u8, usize) -> JianStatus =
        jian_text_insert;
    let typed = b"MMMMMMMM";
    assert_eq!(
        unsafe { insert(engine, typed.as_ptr(), typed.len()) },
        JianStatus::Ok
    );

    tick += 1;
    paint(engine, &mut buffer, tick);
    let typed_ink = ink(&buffer, stride);

    // Eight glyphs are worth far more ink than a caret; the point is that the
    // typed text is on the surface at all.
    assert!(
        typed_ink > empty_ink + 100,
        "typed text must be painted: ink {empty_ink} (empty) -> {typed_ink} (after typing \
         'MMMMMMMM'); an unchanged count means the frame still paints the authored value"
    );

    unsafe { jian_destroy(engine) };
}
