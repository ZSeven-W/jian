use jian_core::geometry::Affine2;
use jian_core::render::{
    collect_scene_paint_commands_with_state, RenderBackend, ScenePaintCommand,
};
use jian_core::runtime::Runtime;
use jian_skia::{SkiaBackend, SkiaSurface};

pub(crate) fn prepare_commands(
    runtime: &mut Runtime,
    backend: &mut SkiaBackend,
) -> Vec<ScenePaintCommand> {
    runtime.prepare_frame(backend, 0);
    runtime
        .document
        .as_ref()
        .map(|document| {
            collect_scene_paint_commands_with_state(document, &runtime.layout, &runtime.state)
        })
        .unwrap_or_default()
}

pub(crate) fn paint_commands(
    backend: &mut SkiaBackend,
    surface: &mut SkiaSurface,
    commands: Vec<ScenePaintCommand>,
    dpr: f32,
) {
    backend.begin_frame(surface, 0xffffffff);
    let scaled = (dpr - 1.0).abs() > f32::EPSILON;
    if scaled {
        backend.push_transform(&Affine2::scale(dpr, dpr));
    }
    for command in commands {
        match command {
            ScenePaintCommand::PushClip(rect) => backend.push_clip(rect),
            ScenePaintCommand::PushTransform(transform) => backend.push_transform(&transform),
            ScenePaintCommand::Pop => backend.pop(),
            ScenePaintCommand::ApplyBlur(sigma) => backend.apply_blur(sigma),
            ScenePaintCommand::ApplyShadow(shadow) => backend.apply_shadow(&shadow),
            ScenePaintCommand::PushLayer(bounds) => backend.push_layer(bounds),
            ScenePaintCommand::PopLayer => backend.pop_layer(),
            ScenePaintCommand::Draw(op) => backend.draw(&op),
            ScenePaintCommand::RichText { run, plan } => {
                backend.draw_text_runs(&run, &plan.spans);
            }
        }
    }
    if scaled {
        backend.pop();
    }
    backend.end_frame(surface);
}
