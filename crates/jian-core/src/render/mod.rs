//! RenderBackend trait — how to draw the scene.
//!
//! Host crates implement this trait. `jian-skia` will be the MVP implementation.
//! For tests we provide [`CaptureBackend`], which records every call into a
//! `Vec<RenderCommand>` so integration tests can assert on the output.

pub mod commands;
pub mod image_store;
pub mod paint;
pub mod scene;
pub mod widget_style;

pub use commands::{affine_to_array, RenderCommand};
pub use paint::{
    BorderRadii, DrawOp, GradientStop, ImageSource, LinearGradient, MeshGradient, Paint,
    PathCommand, RadialGradient, ShaderSpec, ShaderUniform, ShadowSpec, StrokeOp, TextAlign,
    TextRun,
};
pub use scene::{collect_draws, collect_draws_with_state};

use crate::geometry::{Affine2, Rect, Size};

#[derive(Debug, Clone)]
pub struct DecodeError(pub String);
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for DecodeError {}

pub fn probe_image_bounds(bytes: &[u8]) -> Result<(), DecodeError> {
    let dimensions = image_dimensions(bytes);
    if let Some((width, height)) = dimensions {
        let rgba = u64::from(width)
            .saturating_mul(u64::from(height))
            .saturating_mul(4);
        if width > 16_384 || height > 16_384 || rgba > 128 * 1024 * 1024 {
            return Err(DecodeError("decoded image bounds exceeded".into()));
        }
    }
    Ok(())
}

fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        return Some((
            u32::from_be_bytes(bytes[16..20].try_into().ok()?),
            u32::from_be_bytes(bytes[20..24].try_into().ok()?),
        ));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some((
            u16::from_le_bytes(bytes.get(6..8)?.try_into().ok()?) as u32,
            u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?) as u32,
        ));
    }
    if bytes.starts_with(b"RIFF")
        && bytes.get(8..12) == Some(b"WEBP")
        && bytes.get(12..16) == Some(b"VP8X")
        && bytes.len() >= 30
    {
        let w = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
        let h = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
        return Some((w, h));
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut i = 2usize;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xff {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                return Some((
                    u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32,
                    u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32,
                ));
            }
            if marker == 0xd8 || marker == 0xd9 {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if len < 2 {
                return None;
            }
            i = i.saturating_add(2 + len);
        }
    }
    None
}

pub trait RenderBackend {
    type Surface;

    fn new_surface(&mut self, size: Size) -> Self::Surface;

    fn begin_frame(&mut self, surface: &mut Self::Surface, clear: u32);
    fn end_frame(&mut self, surface: &mut Self::Surface);

    fn push_clip(&mut self, rect: Rect);
    fn push_transform(&mut self, m: &Affine2);
    fn pop(&mut self);

    fn push_layer(&mut self, bounds: Rect);
    fn pop_layer(&mut self);
    fn apply_blur(&mut self, sigma: f32);
    fn apply_shadow(&mut self, shadow: &ShadowSpec);

    fn draw(&mut self, op: &DrawOp);
    fn register_image(&mut self, url_key: &str, bytes: &[u8]) -> Result<(), DecodeError> {
        let _ = url_key;
        probe_image_bounds(bytes)
    }
    fn release_image(&mut self, url_key: &str) {
        let _ = url_key;
    }
}

/// Test / replay backend that records every command.
pub struct CaptureBackend {
    pub commands: Vec<RenderCommand>,
}

impl CaptureBackend {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
    pub fn take(&mut self) -> Vec<RenderCommand> {
        std::mem::take(&mut self.commands)
    }
}

impl Default for CaptureBackend {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CaptureSurface(());

impl RenderBackend for CaptureBackend {
    type Surface = CaptureSurface;

    fn new_surface(&mut self, _size: Size) -> Self::Surface {
        CaptureSurface(())
    }

    fn begin_frame(&mut self, _surface: &mut Self::Surface, clear: u32) {
        self.commands.push(RenderCommand::BeginFrame { clear });
    }
    fn end_frame(&mut self, _surface: &mut Self::Surface) {
        self.commands.push(RenderCommand::EndFrame);
    }
    fn push_clip(&mut self, rect: Rect) {
        self.commands.push(RenderCommand::PushClip { rect });
    }
    fn push_transform(&mut self, m: &Affine2) {
        self.commands.push(RenderCommand::PushTransform {
            m: affine_to_array(m),
        });
    }
    fn pop(&mut self) {
        self.commands.push(RenderCommand::Pop);
    }
    fn push_layer(&mut self, bounds: Rect) {
        self.commands.push(RenderCommand::PushLayer { bounds });
    }
    fn pop_layer(&mut self) {
        self.commands.push(RenderCommand::PopLayer);
    }
    fn apply_blur(&mut self, sigma: f32) {
        self.commands.push(RenderCommand::ApplyBlur { sigma });
    }
    fn apply_shadow(&mut self, shadow: &ShadowSpec) {
        self.commands
            .push(RenderCommand::ApplyShadow(shadow.clone()));
    }
    fn draw(&mut self, op: &DrawOp) {
        self.commands.push(RenderCommand::Draw(op.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{rect, size};
    use crate::scene::Color;

    #[test]
    fn capture_records_begin_end() {
        let mut b = CaptureBackend::new();
        let mut s = b.new_surface(size(100.0, 100.0));
        b.begin_frame(&mut s, 0xffffffff);
        b.end_frame(&mut s);
        assert_eq!(b.commands.len(), 2);
    }

    #[test]
    fn capture_records_draw_rect() {
        let mut b = CaptureBackend::new();
        b.draw(&DrawOp::Rect {
            rect: rect(0.0, 0.0, 10.0, 10.0),
            paint: Paint::solid(Color::rgb(0xff, 0, 0)),
        });
        match &b.commands[0] {
            RenderCommand::Draw(DrawOp::Rect { .. }) => {}
            _ => panic!("wrong command"),
        }
    }
}
