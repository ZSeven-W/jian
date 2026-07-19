use jian_core::geometry::Affine2;
use jian_core::render::{
    collect_scene_paint_commands_with_widgets, RenderBackend, ScenePaintCommand, WidgetRenderCtx,
    WidgetTheme,
};
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
    // WITH widgets: this host takes input, so a text field must paint what the
    // user typed rather than the value the document was authored with. The
    // state-only collector leaves every edit invisible.
    let theme = WidgetTheme::default();
    let focused = runtime.focused_widget_id();
    let widgets = WidgetRenderCtx {
        states: &runtime.widget_states,
        theme: &theme,
        focused_id: focused.as_deref(),
        now_ms: runtime.last_now_ms(),
    };
    runtime
        .document
        .as_ref()
        .map(|document| {
            collect_scene_paint_commands_with_widgets(
                document,
                &runtime.layout,
                &runtime.state,
                &widgets,
            )
        })
        .unwrap_or_default()
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
