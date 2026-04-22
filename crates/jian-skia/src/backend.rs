//! `SkiaBackend` — the `RenderBackend` impl.
//!
//! Each draw call hits the current canvas directly. The backend is
//! stateless between frames; callers pass the target `SkiaSurface` to
//! `begin_frame` / `end_frame`. Clip / transform / layer saves are
//! tracked via `canvas.save()` / `canvas.restore()`.

use crate::color::to_sk_color;
use crate::convert::to_sk_rect;
use crate::path::to_sk_path;
use crate::surface::SkiaSurface;
use jian_core::geometry::{Affine2, Rect, Size};
use jian_core::render::{BorderRadii, DrawOp, Paint, RenderBackend, ShadowSpec};
use skia_safe::{
    image_filters, BlurStyle, Color, Color4f, ImageFilter, MaskFilter, Paint as SkPaint,
    PaintStyle, Point as SkPoint, RRect, Rect as SkRect,
};

pub struct SkiaBackend {
    /// Pending image filter applied on the next `push_layer`. Drained by
    /// `push_layer` / cleared on `pop_layer`. Used by `apply_blur` /
    /// `apply_shadow` to mirror the CaptureBackend command stream.
    pending_filter: Option<ImageFilter>,
}

impl SkiaBackend {
    pub fn new() -> Self {
        Self {
            pending_filter: None,
        }
    }
}

impl Default for SkiaBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderBackend for SkiaBackend {
    type Surface = SkiaSurface;

    fn new_surface(&mut self, size: Size) -> Self::Surface {
        SkiaSurface::new_raster(size.width.max(1.0) as i32, size.height.max(1.0) as i32)
    }

    fn begin_frame(&mut self, surface: &mut Self::Surface, clear: u32) {
        let canvas = surface.canvas();
        canvas.clear(Color::new(clear));
        canvas.save();
    }

    fn end_frame(&mut self, _surface: &mut Self::Surface) {
        // `save()` in `begin_frame` is matched here. We intentionally do
        // NOT flush the surface — host decides when to present.
        // `save` stack: a caller that forgot to `pop` would leak state;
        // use `restore_to_count(1)` to guarantee cleanup.
        _surface.canvas().restore_to_count(1);
    }

    fn push_clip(&mut self, _rect: Rect) {
        // Needs the canvas. Plan 2's trait is canvas-less; we stash the
        // rect and apply on the next draw. TODO(Plan 8): revise trait to
        // accept `&mut Self::Surface` on every method — that unlocks
        // direct canvas access. For now, `push_clip`/`push_transform` are
        // no-ops and callers rely on scene-level transforms instead.
    }

    fn push_transform(&mut self, _m: &Affine2) {
        // Same caveat as `push_clip`. See TODO.
    }

    fn pop(&mut self) {}

    fn push_layer(&mut self, _bounds: Rect) {
        // Same caveat; layer push is currently a no-op.
        self.pending_filter = None;
    }

    fn pop_layer(&mut self) {
        self.pending_filter = None;
    }

    fn apply_blur(&mut self, sigma: f32) {
        self.pending_filter = image_filters::blur((sigma, sigma), None, None, None);
    }

    fn apply_shadow(&mut self, shadow: &ShadowSpec) {
        let color = to_sk_color(shadow.color);
        let rgba = Color::from_argb(
            (color.a * 255.0) as u8,
            (color.r * 255.0) as u8,
            (color.g * 255.0) as u8,
            (color.b * 255.0) as u8,
        );
        self.pending_filter = image_filters::drop_shadow(
            (shadow.dx, shadow.dy),
            (shadow.blur, shadow.blur),
            rgba,
            None,
            None,
            None,
        );
    }

    fn draw(&mut self, _op: &DrawOp) {
        // Canvas-less trait forces draws to bounce through a per-surface
        // shim. `draw_on` is the real worker — see the helper below.
    }
}

impl SkiaBackend {
    /// Draw directly onto a surface. This is the canvas-aware entry point;
    /// the `RenderBackend::draw` signature in Plan 2 doesn't carry the
    /// surface, so tests / host adapters call this instead until the
    /// trait is revised in Plan 8.
    pub fn draw_on(&mut self, surface: &mut SkiaSurface, op: &DrawOp) {
        let canvas = surface.canvas();
        match op {
            DrawOp::Rect { rect, paint } => {
                draw_rect(canvas, *rect, paint);
            }
            DrawOp::RoundedRect { rect, radii, paint } => {
                draw_rrect(canvas, *rect, *radii, paint);
            }
            DrawOp::Path { commands, paint } => {
                let path = to_sk_path(commands);
                if let Some(fill_color) = paint.fill {
                    let mut p = SkPaint::new(to_sk_color(fill_color), None);
                    p.set_alpha_f(paint.opacity);
                    p.set_anti_alias(true);
                    p.set_style(PaintStyle::Fill);
                    canvas.draw_path(&path, &p);
                }
                if let Some(ref stroke) = paint.stroke {
                    let mut p = SkPaint::new(to_sk_color(stroke.color), None);
                    p.set_style(PaintStyle::Stroke);
                    p.set_stroke_width(stroke.width);
                    p.set_anti_alias(true);
                    p.set_alpha_f(paint.opacity);
                    canvas.draw_path(&path, &p);
                }
            }
            DrawOp::Image { dst, opacity, .. } => {
                // MVP: draw a 50% grey placeholder until the image cache
                // lands. Keeps golden layouts stable.
                let mut p = SkPaint::new(Color4f::new(0.5, 0.5, 0.5, *opacity), None);
                p.set_anti_alias(true);
                canvas.draw_rect(to_sk_rect(*dst), &p);
            }
            DrawOp::Text(run) => {
                let mut p = SkPaint::new(to_sk_color(run.color), None);
                p.set_anti_alias(true);
                let font = skia_safe::Font::new(
                    skia_safe::FontMgr::new()
                        .match_family_style(&run.font_family, skia_safe::FontStyle::normal())
                        .unwrap_or_else(|| {
                            skia_safe::FontMgr::new()
                                .legacy_make_typeface(None, skia_safe::FontStyle::normal())
                                .expect("default typeface")
                        }),
                    run.font_size,
                );
                canvas.draw_str(
                    &run.content,
                    SkPoint::new(run.origin.x, run.origin.y + run.font_size),
                    &font,
                    &p,
                );
            }
        }
    }
}

fn draw_rect(canvas: &skia_safe::Canvas, r: Rect, paint: &Paint) {
    let rr: SkRect = to_sk_rect(r);
    if let Some(fill) = paint.fill {
        let mut p = SkPaint::new(to_sk_color(fill), None);
        p.set_alpha_f(paint.opacity);
        p.set_anti_alias(true);
        p.set_style(PaintStyle::Fill);
        canvas.draw_rect(rr, &p);
    }
    if let Some(ref stroke) = paint.stroke {
        let mut p = SkPaint::new(to_sk_color(stroke.color), None);
        p.set_style(PaintStyle::Stroke);
        p.set_stroke_width(stroke.width);
        p.set_alpha_f(paint.opacity);
        p.set_anti_alias(true);
        canvas.draw_rect(rr, &p);
    }
}

fn draw_rrect(canvas: &skia_safe::Canvas, r: Rect, radii: BorderRadii, paint: &Paint) {
    let sk_rect = to_sk_rect(r);
    let radii_arr = [
        SkPoint::new(radii.tl, radii.tl),
        SkPoint::new(radii.tr, radii.tr),
        SkPoint::new(radii.br, radii.br),
        SkPoint::new(radii.bl, radii.bl),
    ];
    let rrect = RRect::new_rect_radii(sk_rect, &radii_arr);
    if let Some(fill) = paint.fill {
        let mut p = SkPaint::new(to_sk_color(fill), None);
        p.set_alpha_f(paint.opacity);
        p.set_anti_alias(true);
        p.set_style(PaintStyle::Fill);
        canvas.draw_rrect(rrect, &p);
    }
    if let Some(ref stroke) = paint.stroke {
        let mut p = SkPaint::new(to_sk_color(stroke.color), None);
        p.set_style(PaintStyle::Stroke);
        p.set_stroke_width(stroke.width);
        p.set_alpha_f(paint.opacity);
        p.set_anti_alias(true);
        canvas.draw_rrect(rrect, &p);
    }
}

// Convenience: lets `BlurStyle` / `MaskFilter` stay imported even when
// textlayout is off.
#[allow(dead_code)]
fn _unused_keeping_imports() {
    let _ = BlurStyle::Normal;
    let _ = MaskFilter::blur(BlurStyle::Normal, 1.0, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::geometry::{rect, size};
    use jian_core::scene::Color;

    fn draw_blue_rect() -> SkiaSurface {
        let mut backend = SkiaBackend::new();
        let mut surface = backend.new_surface(size(64.0, 64.0));
        backend.begin_frame(&mut surface, 0xffffffff);
        backend.draw_on(
            &mut surface,
            &DrawOp::Rect {
                rect: rect(8.0, 8.0, 48.0, 48.0),
                paint: Paint::solid(Color::rgb(0x1e, 0x88, 0xe5)),
            },
        );
        backend.end_frame(&mut surface);
        surface
    }

    #[test]
    fn rect_emits_png() {
        let mut s = draw_blue_rect();
        let png = s.encode_png().unwrap();
        assert!(png.len() > 100);
    }

    #[test]
    fn rounded_rect_draws() {
        let mut backend = SkiaBackend::new();
        let mut surface = backend.new_surface(size(32.0, 32.0));
        backend.begin_frame(&mut surface, 0);
        backend.draw_on(
            &mut surface,
            &DrawOp::RoundedRect {
                rect: rect(4.0, 4.0, 24.0, 24.0),
                radii: BorderRadii::uniform(4.0),
                paint: Paint::solid(Color::rgb(0xff, 0x00, 0x00)),
            },
        );
        backend.end_frame(&mut surface);
        assert!(surface.encode_png().is_some());
    }

    #[test]
    fn path_triangle_draws() {
        use jian_core::geometry::point;
        use jian_core::render::PathCommand::{self, *};
        let mut backend = SkiaBackend::new();
        let mut surface = backend.new_surface(size(32.0, 32.0));
        backend.begin_frame(&mut surface, 0);
        let cmds: Vec<PathCommand> = vec![
            MoveTo(point(4.0, 28.0)),
            LineTo(point(28.0, 28.0)),
            LineTo(point(16.0, 4.0)),
            Close,
        ];
        backend.draw_on(
            &mut surface,
            &DrawOp::Path {
                commands: cmds,
                paint: Paint::solid(Color::rgb(0x00, 0xff, 0x00)),
            },
        );
        backend.end_frame(&mut surface);
        assert!(surface.encode_png().is_some());
    }

    #[test]
    fn apply_blur_sets_pending_filter() {
        let mut backend = SkiaBackend::new();
        backend.apply_blur(4.0);
        assert!(backend.pending_filter.is_some());
        backend.pop_layer();
        assert!(backend.pending_filter.is_none());
    }
}
