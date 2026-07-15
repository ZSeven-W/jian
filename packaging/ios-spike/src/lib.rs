#![cfg(target_os = "ios")]

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};

use jian_core::geometry::rect;
use jian_core::render::{DrawOp, Paint, RenderBackend};
use jian_core::scene::Color;
use jian_skia::surface::metal::MetalSurface;
use jian_skia::SkiaBackend;

pub struct JianIosSpike {
    surface: MetalSurface,
    backend: SkiaBackend,
}

/// Creates a renderer for a shell-owned `CAMetalLayer *`.
///
/// The caller must keep the layer alive and unchanged until
/// `jian_ios_spike_destroy` returns.
#[no_mangle]
pub unsafe extern "C" fn jian_ios_spike_create(layer: *mut c_void) -> *mut JianIosSpike {
    catch_unwind(AssertUnwindSafe(|| {
        let surface = MetalSurface::from_ca_metal_layer(layer).ok()?;
        Some(Box::into_raw(Box::new(JianIosSpike {
            surface,
            backend: SkiaBackend::new(),
        })))
    }))
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut())
}

/// Draws and presents one white frame with a centered red rectangle.
///
/// Returns 1 when presented, 0 when the layer had no drawable, and -1 on
/// invalid input or a rendering error.
#[no_mangle]
pub unsafe extern "C" fn jian_ios_spike_draw_red(spike: *mut JianIosSpike) -> i32 {
    if spike.is_null() {
        return -1;
    }

    catch_unwind(AssertUnwindSafe(|| {
        let spike = &mut *spike;
        let (surface, backend) = (&mut spike.surface, &mut spike.backend);
        surface
            .draw_frame(|frame| {
                let width = frame.width() as f32;
                let height = frame.height() as f32;
                backend.begin_frame(frame, 0xffffffff);
                backend.draw(&DrawOp::Rect {
                    rect: rect(width * 0.2, height * 0.2, width * 0.6, height * 0.6),
                    paint: Paint::solid(Color::rgb(0xff, 0x00, 0x00)),
                });
                backend.end_frame(frame);
            })
            .map(i32::from)
            .unwrap_or(-1)
    }))
    .unwrap_or(-1)
}

/// Destroys a renderer. Passing null is a no-op.
#[no_mangle]
pub unsafe extern "C" fn jian_ios_spike_destroy(spike: *mut JianIosSpike) {
    if !spike.is_null() {
        drop(Box::from_raw(spike));
    }
}
