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
use std::sync::{Mutex, MutexGuard};

const WIDTH: usize = 200;
const HEIGHT: usize = 40;

// Skia's process-global CPU/font state is not a useful part of these C-ABI
// assertions. Keep the three raster tests deterministic under libtest, just
// as production hosts keep one renderer on their render thread.
static CPU_FRAME_TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock_cpu_frame_test() -> MutexGuard<'static, ()> {
    CPU_FRAME_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
    let _serial = lock_cpu_frame_test();
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
    let paint = |engine, buffer: &mut Vec<u8>, tick: u64| {
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

/// Rightmost column holding ink — with a caret drawn past the glyphs, this is
/// the caret itself.
#[cfg(feature = "textlayout")]
fn rightmost_ink(buffer: &[u8], stride: usize) -> Option<usize> {
    (0..WIDTH).rev().find(|&x| {
        (0..HEIGHT).any(|y| {
            let p = y * stride + x * 4;
            buffer[p] < 128 && buffer[p + 1] < 128 && buffer[p + 2] < 128
        })
    })
}

fn changed_column_range(before: &[u8], after: &[u8], stride: usize) -> Option<(usize, usize)> {
    let changed = |x: usize| {
        (0..HEIGHT).any(|y| {
            let p = y * stride + x * 4;
            before[p..p + 4] != after[p..p + 4]
        })
    };
    let first = (0..WIDTH).find(|&x| changed(x))?;
    let last = (0..WIDTH).rev().find(|&x| changed(x))?;
    Some((first, last))
}

/// The PAINTED caret must sit where the engine says the caret is.
///
/// Regression: the live emitter placed it with a monospace approximation
/// (`font_size * 0.55` per character) while the glyphs were drawn with real
/// proportional metrics, so the error accumulated with length — visible as a
/// caret drifting further right the more you typed. The engine already
/// computes a true caret rect from measured glyphs; the painter just never
/// asked for it.
#[test]
fn painted_caret_blinks_and_matches_shaped_geometry_when_enabled() {
    let _serial = lock_cpu_frame_test();
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

    // Narrow glyphs: a fixed per-character advance overshoots them badly, so
    // the approximation and the truth are far apart.
    let insert: unsafe extern "C" fn(*mut JianEngine, *const u8, usize) -> JianStatus =
        jian_text_insert;
    let typed = b"iiiiiiiiiiii";
    assert_eq!(
        unsafe { insert(engine, typed.as_ptr(), typed.len()) },
        JianStatus::Ok
    );

    let frame: unsafe extern "C" fn(*mut JianEngine, u64, *mut u8, usize, usize) -> JianStatus =
        jian_frame_cpu;
    let stride = WIDTH * 4;
    let mut with_caret = vec![0u8; stride * HEIGHT];
    assert_eq!(
        unsafe {
            frame(
                engine,
                16,
                with_caret.as_mut_ptr(),
                with_caret.len(),
                stride,
            )
        },
        JianStatus::Ok
    );

    // Render the identical focused field in the blink-off half-period. Pixel
    // subtraction isolates the caret without assuming a platform font's
    // antialiasing threshold (and without a fake "blur" click inside the same
    // full-width field).
    let mut without_caret = vec![0u8; stride * HEIGHT];
    assert_eq!(
        unsafe {
            frame(
                engine,
                600,
                without_caret.as_mut_ptr(),
                without_caret.len(),
                stride,
            )
        },
        JianStatus::Ok
    );
    let (caret_left, caret_right) = changed_column_range(&with_caret, &without_caret, stride)
        .expect("the blink-on and blink-off frames must differ at the painted caret");
    assert!(
        caret_right.saturating_sub(caret_left) <= 2,
        "caret raster unexpectedly spans columns {caret_left}..={caret_right}"
    );
    // The lightweight build intentionally uses the documented character-width
    // estimate. Production mobile builds enable `textlayout`; only that build
    // owns shaped geometry and can enforce the no-drift assertion.
    #[cfg(feature = "textlayout")]
    {
        let glyph_end =
            rightmost_ink(&without_caret, stride).expect("the text must still be painted");
        let gap = caret_right as i64 - glyph_end as i64;
        assert!(
            (0..=8).contains(&gap),
            "the caret sits {gap}px past the last glyph (caret x={caret_right}, glyphs end at \
             {glyph_end}); a fixed per-character advance drifts further right the more is typed"
        );
    }

    unsafe { jian_destroy(engine) };
}

/// A text field must not paint outside its own box.
///
/// Regression: `m4_media`'s `longfield` is a 280x36 `text_input` holding 5400
/// characters, and it painted every one of them straight down the screen over
/// the buttons below it. The nodes do not overlap — the field simply never
/// clipped its content.
#[test]
fn a_text_field_clips_its_content_to_its_own_box() {
    let _serial = lock_cpu_frame_test();
    const TALL: usize = 160;
    const FIELD_BOTTOM: usize = 40;
    // The field occupies the top 40px; everything below must stay clean.
    const DOC: &[u8] = br#"{
      "version":"1.2","formatVersion":"1.2",
      "children":[{"type":"frame","id":"root","width":200,"height":160,
        "children":[{"type":"text_input","id":"field","x":0,"y":0,
                     "width":150,"height":40,
                     "value":"the quick brown fox jumps over the lazy dog and keeps going well past the end of this rather small box so that any unclipped overflow is impossible to miss"}]}]
    }"#;

    let desc = JianCreateDesc {
        size: size_of::<JianCreateDesc>(),
        doc_ptr: DOC.as_ptr(),
        doc_len: DOC.len(),
        width: WIDTH as f32,
        height: TALL as f32,
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
    let mut buffer = vec![0u8; stride * TALL];
    assert_eq!(
        unsafe { frame(engine, 16, buffer.as_mut_ptr(), buffer.len(), stride) },
        JianStatus::Ok
    );

    // Ink strictly below the field is overflow, by construction: nothing else
    // is authored down there.
    let mut escaped = 0;
    for y in (FIELD_BOTTOM + 4)..TALL {
        for x in 0..WIDTH {
            let p = y * stride + x * 4;
            if buffer[p] < 128 && buffer[p + 1] < 128 && buffer[p + 2] < 128 {
                escaped += 1;
            }
        }
    }
    assert_eq!(
        escaped, 0,
        "{escaped} ink pixels painted below the field's 40px box; a text field must clip \
         its content to its own bounds"
    );

    unsafe { jian_destroy(engine) };
}
