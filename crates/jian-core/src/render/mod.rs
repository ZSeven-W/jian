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
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct DecodeError(pub String);
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for DecodeError {}

thread_local! { static REGISTERED_IMAGES: RefCell<BTreeMap<String, Vec<u8>>> = const { RefCell::new(BTreeMap::new()) }; }
pub fn registered_image_bytes(key: &str) -> Option<Vec<u8>> {
    REGISTERED_IMAGES.with(|images| images.borrow().get(key).cloned())
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
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
            let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
            let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
            let rgba = (width as u64)
                .saturating_mul(height as u64)
                .saturating_mul(4);
            if width > 16_384 || height > 16_384 || rgba > 128 * 1024 * 1024 {
                return Err(DecodeError("decoded image bounds exceeded".into()));
            }
        }
        REGISTERED_IMAGES.with(|images| {
            images
                .borrow_mut()
                .insert(url_key.to_owned(), bytes.to_vec())
        });
        Ok(())
    }
    fn release_image(&mut self, url_key: &str) {
        REGISTERED_IMAGES.with(|images| images.borrow_mut().remove(url_key));
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
