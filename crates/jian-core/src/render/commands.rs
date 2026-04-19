//! RenderCommand — a recordable drawing instruction. Used by the `CaptureBackend`
//! in tests and by replay tools. For live rendering the backend issues calls
//! directly on the trait without constructing this enum.

use super::paint::{DrawOp, ShadowSpec};
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
    PopLayer,
    ApplyBlur { sigma: f32 },
    ApplyShadow(ShadowSpec),
}

pub fn affine_to_array(a: &Affine2) -> [f32; 6] {
    [a.m11, a.m12, a.m21, a.m22, a.m31, a.m32]
}
