//! RenderBackend trait — how to draw the scene.
//!
//! Host crates implement this trait. `jian-skia` will be the MVP implementation.
//! For tests we provide [`CaptureBackend`], which records every call into a
//! `Vec<RenderCommand>` so integration tests can assert on the output.

pub mod commands;
pub mod image_store;
pub mod paint;
pub mod scene;
mod scene_commands;
pub mod text;
pub mod text_geometry;
pub mod widget_style;

pub use commands::{affine_to_array, RenderCommand, ScenePaintCommand};
pub use paint::{
    BorderRadii, DrawOp, GradientStop, ImageSource, LinearGradient, MeshGradient, Paint,
    PathCommand, RadialGradient, ShaderSpec, ShaderUniform, ShadowSpec, StrokeOp, TextAlign,
    TextRun,
};
pub use scene::WidgetRenderCtx;
pub use scene::{collect_draws, collect_draws_with_state, collect_rich_draws_with_state};
pub use scene_commands::{
    collect_scene_paint_commands_with_state, collect_scene_paint_commands_with_widgets,
};
pub use text::{RichDrawList, RichTextGrowth, RichTextPlan, TextSpan};
pub use text_geometry::{
    byte_to_utf16_offset, normalize_utf16_offset, utf16_len, utf16_to_byte_offset, FieldKey,
    Granularity, TextGeometry, TextGeometryError, TextRect, WritingDirection,
};
pub use widget_style::{
    resolve_authored_widget_visual, with_visual_opacity, AuthoredWidgetVisual, WidgetTheme,
};

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
    let (width, height) = image_dimensions(bytes)?;
    if width == 0 || height == 0 {
        return Err(DecodeError("decoded image has zero dimensions".into()));
    }
    let rgba = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| DecodeError("decoded image bounds exceeded".into()))?;
    if width > 16_384 || height > 16_384 || rgba > 128 * 1024 * 1024 {
        return Err(DecodeError("decoded image bounds exceeded".into()));
    }
    Ok(())
}

fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
    let malformed = || DecodeError("malformed or truncated image header".into());
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        if bytes.len() < 33
            || u32::from_be_bytes(bytes[8..12].try_into().map_err(|_| malformed())?) != 13
            || bytes.get(12..16) != Some(b"IHDR")
        {
            return Err(malformed());
        }
        return Ok((
            u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| malformed())?),
            u32::from_be_bytes(bytes[20..24].try_into().map_err(|_| malformed())?),
        ));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        if bytes.len() < 13 {
            return Err(malformed());
        }
        return Ok((
            u16::from_le_bytes(bytes[6..8].try_into().map_err(|_| malformed())?) as u32,
            u16::from_le_bytes(bytes[8..10].try_into().map_err(|_| malformed())?) as u32,
        ));
    }
    if bytes.starts_with(b"RIFF") {
        if bytes.len() < 20 || bytes.get(8..12) != Some(b"WEBP") {
            return Err(malformed());
        }
        let riff_size =
            u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| malformed())?) as usize;
        let chunk_size =
            u32::from_le_bytes(bytes[16..20].try_into().map_err(|_| malformed())?) as usize;
        let padded_chunk_size = chunk_size
            .checked_add(chunk_size & 1)
            .ok_or_else(malformed)?;
        let chunk_end = 20usize
            .checked_add(padded_chunk_size)
            .ok_or_else(malformed)?;
        let riff_end = 8usize.checked_add(riff_size).ok_or_else(malformed)?;
        if riff_end > bytes.len() || chunk_end > riff_end || chunk_end > bytes.len() {
            return Err(malformed());
        }
        match bytes.get(12..16) {
            Some(b"VP8X") => {
                if bytes.len() < 30 || chunk_size < 10 {
                    return Err(malformed());
                }
                let width = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
                let height = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
                return Ok((width, height));
            }
            Some(b"VP8 ") => {
                if bytes.len() < 30
                    || chunk_size < 10
                    || bytes.get(23..26) != Some(&[0x9d, 0x01, 0x2a])
                {
                    return Err(malformed());
                }
                let width = u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3fff;
                let height = u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3fff;
                return Ok((u32::from(width), u32::from(height)));
            }
            Some(b"VP8L") => {
                if bytes.len() < 25 || chunk_size < 5 || bytes[20] != 0x2f {
                    return Err(malformed());
                }
                let b0 = u32::from(bytes[21]);
                let b1 = u32::from(bytes[22]);
                let b2 = u32::from(bytes[23]);
                let b3 = u32::from(bytes[24]);
                let width = 1 + b0 + ((b1 & 0x3f) << 8);
                let height = 1 + ((b1 & 0xc0) >> 6) + (b2 << 2) + ((b3 & 0x0f) << 10);
                return Ok((width, height));
            }
            _ => return Err(DecodeError("unsupported WebP image header".into())),
        }
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut i = 2usize;
        while i < bytes.len() {
            if bytes[i] != 0xff {
                i += 1;
                continue;
            }
            while i < bytes.len() && bytes[i] == 0xff {
                i += 1;
            }
            if i >= bytes.len() {
                return Err(malformed());
            }
            let marker = bytes[i];
            if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                if i + 8 >= bytes.len() {
                    return Err(malformed());
                }
                let segment_len = u16::from_be_bytes([bytes[i + 1], bytes[i + 2]]) as usize;
                let component_count = usize::from(bytes[i + 8]);
                let expected_len = component_count
                    .checked_mul(3)
                    .and_then(|components| components.checked_add(8))
                    .ok_or_else(malformed)?;
                if component_count == 0
                    || segment_len != expected_len
                    || i.checked_add(1 + segment_len).ok_or_else(malformed)? > bytes.len()
                {
                    return Err(malformed());
                }
                return Ok((
                    u16::from_be_bytes([bytes[i + 6], bytes[i + 7]]) as u32,
                    u16::from_be_bytes([bytes[i + 4], bytes[i + 5]]) as u32,
                ));
            }
            if matches!(marker, 0x01 | 0xd0..=0xd9) {
                i += 1;
                continue;
            }
            if i + 2 >= bytes.len() {
                return Err(malformed());
            }
            let len = u16::from_be_bytes([bytes[i + 1], bytes[i + 2]]) as usize;
            if len < 2 {
                return Err(malformed());
            }
            i = i.checked_add(1 + len).ok_or_else(malformed)?;
            if i > bytes.len() {
                return Err(malformed());
            }
        }
        return Err(malformed());
    }
    Err(DecodeError("unsupported image format".into()))
}

#[cfg(test)]
mod image_probe_tests {
    use super::probe_image_bounds;

    fn webp_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        if payload.len() & 1 == 1 {
            bytes.push(0);
        }
        let riff_size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        bytes
    }

    #[test]
    fn supported_image_headers_are_probed() {
        let mut png = vec![0; 33];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[8..12].copy_from_slice(&13u32.to_be_bytes());
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&32u32.to_be_bytes());
        png[20..24].copy_from_slice(&24u32.to_be_bytes());
        assert!(probe_image_bounds(&png).is_ok());

        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&32u16.to_le_bytes());
        gif.extend_from_slice(&24u16.to_le_bytes());
        gif.extend_from_slice(&[0, 0, 0]);
        assert!(probe_image_bounds(&gif).is_ok());

        let jpeg = [
            0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 24, 0, 32, 1, 1, 0x11, 0,
        ];
        assert!(probe_image_bounds(&jpeg).is_ok());

        let mut vp8x = [0; 10];
        vp8x[4..7].copy_from_slice(&31u32.to_le_bytes()[..3]);
        vp8x[7..10].copy_from_slice(&23u32.to_le_bytes()[..3]);
        assert!(probe_image_bounds(&webp_chunk(b"VP8X", &vp8x)).is_ok());

        let mut vp8 = vec![0; 10];
        vp8[3..6].copy_from_slice(&[0x9d, 0x01, 0x2a]);
        vp8[6..8].copy_from_slice(&32u16.to_le_bytes());
        vp8[8..10].copy_from_slice(&24u16.to_le_bytes());
        assert!(probe_image_bounds(&webp_chunk(b"VP8 ", &vp8)).is_ok());

        // width = 32, height = 24 according to the VP8L 14-bit packing.
        let vp8l = [0x2f, 31, 0xc0, 5, 0];
        assert!(probe_image_bounds(&webp_chunk(b"VP8L", &vp8l)).is_ok());
    }

    #[test]
    fn unsupported_truncated_zero_and_over_budget_images_are_rejected() {
        assert!(probe_image_bounds(b"not an image").is_err());
        assert!(probe_image_bounds(b"\x89PNG\r\n\x1a\n").is_err());

        let mut zero_png = vec![0; 33];
        zero_png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        zero_png[8..12].copy_from_slice(&13u32.to_be_bytes());
        zero_png[12..16].copy_from_slice(b"IHDR");
        assert!(probe_image_bounds(&zero_png).is_err());

        let mut vp8 = vec![0; 10];
        vp8[3..6].copy_from_slice(&[0x9d, 0x01, 0x2a]);
        vp8[6..8].copy_from_slice(&0x3fffu16.to_le_bytes());
        vp8[8..10].copy_from_slice(&0x3fffu16.to_le_bytes());
        assert!(probe_image_bounds(&webp_chunk(b"VP8 ", &vp8)).is_err());

        let vp8l = [0x2f, 0xff, 0xff, 0xff, 0x0f];
        assert!(probe_image_bounds(&webp_chunk(b"VP8L", &vp8l)).is_err());
    }
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
    /// Draw a paragraph with exact authored run styles. Backends without a
    /// rich-text shaper retain the stable flat DrawOp behavior.
    fn draw_text_runs(&mut self, run: &TextRun, spans: &[TextSpan]) {
        let _ = spans;
        self.draw(&DrawOp::Text(run.clone()));
    }
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
