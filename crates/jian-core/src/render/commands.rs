//! RenderCommand — a recordable drawing instruction. Used by the `CaptureBackend`
//! in tests and by replay tools. For live rendering the backend issues calls
//! directly on the trait without constructing this enum.

use super::paint::{DrawOp, ShadowSpec, TextRun};
use super::text::RichTextPlan;
use crate::geometry::{Affine2, Rect};

#[derive(Debug, Clone)]
pub enum RenderCommand {
    BeginFrame { clear: u32 /* color packed */ },
    EndFrame,
    PushClip { rect: Rect },
    PushClipPath, // path itself is separate; this is a stub marker
    PushTransform { m: [f32; 6] },
    Pop,
    Draw(DrawOp),
    PushLayer { bounds: Rect },
    PushOpacity(f32),
    PopLayer,
    ApplyBlur { sigma: f32 },
    ApplyShadow(ShadowSpec),
}

/// Structured production scene stream retaining node state scopes and rich
/// paragraph metadata.
#[derive(Debug, Clone)]
pub enum ScenePaintCommand {
    PushClip(Rect),
    PushTransform(Affine2),
    Pop,
    ApplyBlur(f32),
    ApplyShadow(ShadowSpec),
    PushLayer(Rect),
    PushOpacity(f32),
    PopLayer,
    Draw(DrawOp),
    RichText { run: TextRun, plan: RichTextPlan },
}

pub fn affine_to_array(a: &Affine2) -> [f32; 6] {
    [a.m11, a.m12, a.m21, a.m22, a.m31, a.m32]
}

/// Replay the same scene stream on the desktop, embedded and web backends.
pub fn paint_scene_commands(
    backend: &mut impl super::RenderBackend,
    commands: Vec<ScenePaintCommand>,
) {
    for command in commands {
        match command {
            ScenePaintCommand::PushClip(rect) => backend.push_clip(rect),
            ScenePaintCommand::PushTransform(transform) => backend.push_transform(&transform),
            ScenePaintCommand::Pop => backend.pop(),
            ScenePaintCommand::ApplyBlur(sigma) => backend.apply_blur(sigma),
            ScenePaintCommand::ApplyShadow(shadow) => backend.apply_shadow(&shadow),
            ScenePaintCommand::PushLayer(bounds) => backend.push_layer(bounds),
            ScenePaintCommand::PushOpacity(alpha) => backend.push_opacity(alpha),
            ScenePaintCommand::PopLayer => backend.pop_layer(),
            ScenePaintCommand::Draw(op) => backend.draw(&op),
            ScenePaintCommand::RichText { run, plan } => backend.draw_text_plan(&run, &plan),
        }
    }
}
