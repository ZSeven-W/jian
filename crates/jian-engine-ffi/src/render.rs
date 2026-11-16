use jian_core::geometry::Affine2;
use jian_core::render::{paint_scene_commands, RenderBackend, ScenePaintCommand};
use jian_core::runtime::Runtime;
use jian_skia::{InstanceImageRegistry, RegisteredBackend, SkiaBackend, SkiaSurface};

pub(crate) fn prepare_commands(
    runtime: &mut Runtime,
    backend: &mut SkiaBackend,
    images: &mut InstanceImageRegistry,
    backend_generation: u64,
) -> Vec<ScenePaintCommand> {
    // Registration must reach the instance registry, not the bare backend:
    // `SkiaBackend::register_image` validates and DROPS the bytes, so a keyed
    // draw would only ever paint the placeholder (M4 plan Task 3c).
    let mut registered = RegisteredBackend {
        inner: backend,
        images,
    };
    runtime.prepare_frame(&mut registered, backend_generation);
    runtime.standalone_paint_commands()
}

pub(crate) fn paint_commands(
    inner: &mut SkiaBackend,
    images: &mut InstanceImageRegistry,
    surface: &mut SkiaSurface,
    commands: Vec<ScenePaintCommand>,
    dpr: f32,
) {
    // Keyed image draws translate to byte draws through the same registry
    // the prepare pass registered into.
    let mut backend = RegisteredBackend { inner, images };
    let backend = &mut backend;
    backend.begin_frame(surface, 0xffffffff);
    let scaled = (dpr - 1.0).abs() > f32::EPSILON;
    if scaled {
        backend.push_transform(&Affine2::scale(dpr, dpr));
    }
    paint_scene_commands(backend, commands);
    if scaled {
        backend.pop();
    }
    backend.end_frame(surface);
}
