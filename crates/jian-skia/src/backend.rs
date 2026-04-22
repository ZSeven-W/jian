//! `SkiaBackend` — the `RenderBackend` impl.
//!
//! The `RenderBackend` trait from Plan 2 is canvas-less: `draw`,
//! `push_clip`, `push_transform`, `push_layer`, `apply_blur`, and
//! `apply_shadow` don't carry the surface. To honour the full contract,
//! `SkiaBackend` accumulates a `Vec<Cmd>` between `begin_frame` and
//! `end_frame`, then replays it onto the surface's canvas. This means a
//! caller written against the generic trait gets correct output even
//! though the trait itself never sees the canvas.
//!
//! The canvas-aware entry point `SkiaBackend::draw_on(surface, op)`
//! remains available for host adapters that want to draw without
//! going through the frame-scoped command buffer.

use crate::color::to_sk_color;
use crate::convert::{to_sk_matrix, to_sk_rect};
use crate::path::to_sk_path;
use crate::surface::SkiaSurface;
use jian_core::geometry::{Affine2, Rect, Size};
use jian_core::render::{BorderRadii, DrawOp, Paint, RenderBackend, ShadowSpec};
use skia_safe::{
    canvas::SaveLayerRec, image_filters, BlurStyle, Color, Color4f, ImageFilter, MaskFilter,
    Paint as SkPaint, PaintStyle, Point as SkPoint, RRect, Rect as SkRect,
};

/// Buffered command — one-to-one with a trait-level `RenderBackend` call.
#[derive(Clone)]
enum Cmd {
    PushClip(Rect),
    PushTransform(Affine2),
    Pop,
    PushLayer {
        bounds: Rect,
        filter: Option<ImageFilter>,
    },
    PopLayer,
    Draw(DrawOp),
}

pub struct SkiaBackend {
    /// Pending image filter accumulated by `apply_blur` / `apply_shadow`.
    /// Consumed by the next `push_layer` so the layer is drawn with the
    /// filter applied.
    pending_filter: Option<ImageFilter>,
    /// Recorded commands for the current frame. Drained by `end_frame`.
    cmds: Vec<Cmd>,
}

impl SkiaBackend {
    pub fn new() -> Self {
        Self {
            pending_filter: None,
            cmds: Vec::new(),
        }
    }

    fn record(&mut self, cmd: Cmd) {
        self.cmds.push(cmd);
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
        self.cmds.clear();
        self.pending_filter = None;
    }

    fn end_frame(&mut self, surface: &mut Self::Surface) {
        let cmds = std::mem::take(&mut self.cmds);
        let canvas = surface.canvas();
        for cmd in cmds {
            match cmd {
                Cmd::PushClip(r) => {
                    canvas.save();
                    canvas.clip_rect(to_sk_rect(r), None, true);
                }
                Cmd::PushTransform(m) => {
                    canvas.save();
                    canvas.concat(&to_sk_matrix(&m));
                }
                Cmd::Pop => {
                    canvas.restore();
                }
                Cmd::PushLayer { bounds, filter } => {
                    let sk_bounds = to_sk_rect(bounds);
                    let paint = filter.map(|f| {
                        let mut p = SkPaint::default();
                        p.set_image_filter(f);
                        p
                    });
                    let mut rec = SaveLayerRec::default().bounds(&sk_bounds);
                    if let Some(ref p) = paint {
                        rec = rec.paint(p);
                    }
                    canvas.save_layer(&rec);
                }
                Cmd::PopLayer => {
                    canvas.restore();
                }
                Cmd::Draw(op) => {
                    draw_canvas(canvas, &op);
                }
            }
        }
        // Matches the `save()` in begin_frame; anything left on the stack
        // (a caller that forgot to pop) is cleaned up by restore_to_count.
        canvas.restore_to_count(1);
    }

    fn push_clip(&mut self, rect: Rect) {
        self.record(Cmd::PushClip(rect));
    }

    fn push_transform(&mut self, m: &Affine2) {
        self.record(Cmd::PushTransform(*m));
    }

    fn pop(&mut self) {
        self.record(Cmd::Pop);
    }

    fn push_layer(&mut self, bounds: Rect) {
        let filter = self.pending_filter.take();
        self.record(Cmd::PushLayer { bounds, filter });
    }

    fn pop_layer(&mut self) {
        self.record(Cmd::PopLayer);
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

    fn draw(&mut self, op: &DrawOp) {
        self.record(Cmd::Draw(op.clone()));
    }
}

impl SkiaBackend {
    /// Canvas-aware entry point — bypasses the command buffer and draws
    /// directly. Host adapters that already have the surface in hand can
    /// use this to avoid the buffer-and-replay round-trip.
    pub fn draw_on(&mut self, surface: &mut SkiaSurface, op: &DrawOp) {
        draw_canvas(surface.canvas(), op);
    }
}

fn draw_canvas(canvas: &skia_safe::Canvas, op: &DrawOp) {
    match op {
        DrawOp::Rect { rect, paint } => draw_rect(canvas, *rect, paint),
        DrawOp::RoundedRect { rect, radii, paint } => draw_rrect(canvas, *rect, *radii, paint),
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
            // MVP: grey placeholder until image cache lands (Plan 12).
            let mut p = SkPaint::new(Color4f::new(0.5, 0.5, 0.5, *opacity), None);
            p.set_anti_alias(true);
            canvas.draw_rect(to_sk_rect(*dst), &p);
        }
        DrawOp::Text(run) => draw_text(canvas, run),
        DrawOp::LinearGradientRect {
            rect,
            radii,
            gradient,
            stroke,
        } => draw_linear_gradient_rect(canvas, *rect, *radii, gradient, stroke.as_ref()),
        DrawOp::ShadowedRect { rect, radii, shadow } => {
            draw_shadowed_rect(canvas, *rect, *radii, shadow);
        }
        DrawOp::Icon {
            rect,
            name,
            family: _,
            color,
        } => draw_icon(canvas, *rect, name, *color),
    }
}

fn draw_text(canvas: &skia_safe::Canvas, run: &jian_core::render::TextRun) {
    use jian_core::render::TextAlign;
    use skia_safe::font_style::Weight;
    use skia_safe::FontStyle;

    let mut p = SkPaint::new(to_sk_color(run.color), None);
    p.set_anti_alias(true);

    let weight = Weight::from(run.font_weight as i32);
    let style = FontStyle::new(
        weight,
        skia_safe::font_style::Width::NORMAL,
        skia_safe::font_style::Slant::Upright,
    );
    let mgr = skia_safe::FontMgr::new();
    let typeface = if !run.font_family.is_empty() {
        mgr.match_family_style(&run.font_family, style)
            .or_else(|| mgr.legacy_make_typeface(None, style))
    } else {
        mgr.legacy_make_typeface(None, style)
    }
    .expect("default typeface");
    let font = skia_safe::Font::new(typeface, run.font_size);

    // Line-height in pixels (defaults to 1.3× font_size if unset).
    let line_mult = if run.line_height > 0.0 {
        run.line_height
    } else {
        1.3
    };
    let line_h = run.font_size * line_mult;

    // Simple word-wrap: split by spaces and accumulate runs that fit
    // within `max_width`. When `max_width == 0` draw on a single line.
    let lines = if run.max_width > 0.0 {
        wrap_to_lines(&run.content, &font, &p, run.max_width)
    } else {
        vec![run.content.clone()]
    };

    for (i, line) in lines.iter().enumerate() {
        let (line_w, _) = font.measure_str(line, Some(&p));
        let x = match run.align {
            TextAlign::Start => run.origin.x,
            TextAlign::Center => run.origin.x + (run.max_width - line_w) * 0.5,
            TextAlign::End => run.origin.x + (run.max_width - line_w),
        };
        // Baseline-y: font_size baseline is roughly 80% of font size.
        // Using `origin.y + font_size` matches CSS-ish placement.
        let y = run.origin.y + run.font_size + (i as f32) * line_h;
        canvas.draw_str(line, SkPoint::new(x, y), &font, &p);
    }
}

fn wrap_to_lines(
    content: &str,
    font: &skia_safe::Font,
    paint: &SkPaint,
    max_width: f32,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for paragraph in content.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        // Measure the whole paragraph first — if it fits, no wrapping.
        let (w_all, _) = font.measure_str(paragraph, Some(paint));
        if w_all <= max_width {
            lines.push(paragraph.to_owned());
            continue;
        }
        let mut cur = String::new();
        for word in paragraph.split_whitespace() {
            let trial = if cur.is_empty() {
                word.to_owned()
            } else {
                format!("{} {}", cur, word)
            };
            let (tw, _) = font.measure_str(&trial, Some(paint));
            if tw <= max_width || cur.is_empty() {
                cur = trial;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_owned();
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    lines
}

fn draw_icon(canvas: &skia_safe::Canvas, r: Rect, name: &str, color: jian_core::scene::Color) {
    use skia_safe::{utils::parse_path, Matrix};
    let Some(d) = crate::icons::lookup(name) else {
        // Unknown glyph — fall back to a small filled square so the
        // missing icon is visible for debugging.
        let mut p = SkPaint::new(to_sk_color(color), None);
        p.set_anti_alias(true);
        p.set_style(PaintStyle::Fill);
        canvas.draw_rect(to_sk_rect(r), &p);
        return;
    };
    let Some(mut path) = parse_path::from_svg(d) else {
        return;
    };
    // Lucide icons are authored in a 24×24 viewBox. Scale + translate
    // into the target rect, preserving aspect ratio.
    let scale_x = r.size.width / 24.0;
    let scale_y = r.size.height / 24.0;
    let mut m = Matrix::new_identity();
    m.pre_translate((r.min_x(), r.min_y()));
    m.pre_scale((scale_x, scale_y), None);
    path.transform(&m);

    let mut sp = SkPaint::new(to_sk_color(color), None);
    sp.set_anti_alias(true);
    sp.set_style(PaintStyle::Stroke);
    // Lucide's native stroke-width is 2 at 24px. Normalise so the
    // visual weight is consistent across icon sizes.
    let stroke_w = 2.0 * scale_x.min(scale_y).max(0.1);
    sp.set_stroke_width(stroke_w);
    sp.set_stroke_cap(skia_safe::paint::Cap::Round);
    sp.set_stroke_join(skia_safe::paint::Join::Round);
    canvas.draw_path(&path, &sp);
}

fn gradient_endpoints(rect: Rect, angle_deg: f32) -> ((f32, f32), (f32, f32)) {
    // Angle convention matches the .op schema: 0° flows left-to-right,
    // 90° top-to-bottom (clockwise from +x). We pick the two points on
    // the rect's edges where the axis-aligned gradient line meets.
    let cx = rect.min_x() + rect.size.width * 0.5;
    let cy = rect.min_y() + rect.size.height * 0.5;
    let rad = angle_deg.to_radians();
    let dx = rad.cos();
    let dy = rad.sin();
    let half_w = rect.size.width * 0.5;
    let half_h = rect.size.height * 0.5;
    // Project half-diagonal onto the axis so the gradient spans the
    // whole rect along the requested direction.
    let t = (dx.abs() * half_w) + (dy.abs() * half_h);
    let x0 = cx - dx * t;
    let y0 = cy - dy * t;
    let x1 = cx + dx * t;
    let y1 = cy + dy * t;
    ((x0, y0), (x1, y1))
}

fn draw_linear_gradient_rect(
    canvas: &skia_safe::Canvas,
    rect: Rect,
    radii: BorderRadii,
    g: &jian_core::render::LinearGradient,
    stroke: Option<&jian_core::render::StrokeOp>,
) {
    use skia_safe::{gradient_shader, Shader, TileMode};
    let (p0, p1) = gradient_endpoints(rect, g.angle_deg);
    let colors: Vec<Color4f> = g.stops.iter().map(|s| to_sk_color(s.color)).collect();
    let offsets: Vec<f32> = g.stops.iter().map(|s| s.offset.clamp(0.0, 1.0)).collect();
    let shader = gradient_shader::linear(
        (SkPoint::new(p0.0, p0.1), SkPoint::new(p1.0, p1.1)),
        skia_safe::gradient_shader::GradientShaderColors::ColorsInSpace(&colors, None),
        offsets.as_slice(),
        TileMode::Clamp,
        None,
        None,
    );
    let mut paint = SkPaint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);
    paint.set_alpha_f(g.opacity);
    if let Some(s) = shader {
        paint.set_shader(s as Shader);
    }

    let is_rounded = radii != BorderRadii::zero();
    if is_rounded {
        let sk_rect = to_sk_rect(rect);
        let radii_arr = [
            SkPoint::new(radii.tl, radii.tl),
            SkPoint::new(radii.tr, radii.tr),
            SkPoint::new(radii.br, radii.br),
            SkPoint::new(radii.bl, radii.bl),
        ];
        let rrect = RRect::new_rect_radii(sk_rect, &radii_arr);
        canvas.draw_rrect(rrect, &paint);
    } else {
        canvas.draw_rect(to_sk_rect(rect), &paint);
    }

    if let Some(stroke) = stroke {
        let mut p = SkPaint::new(to_sk_color(stroke.color), None);
        p.set_style(PaintStyle::Stroke);
        p.set_stroke_width(stroke.width);
        p.set_anti_alias(true);
        if is_rounded {
            let sk_rect = to_sk_rect(rect);
            let radii_arr = [
                SkPoint::new(radii.tl, radii.tl),
                SkPoint::new(radii.tr, radii.tr),
                SkPoint::new(radii.br, radii.br),
                SkPoint::new(radii.bl, radii.bl),
            ];
            let rrect = RRect::new_rect_radii(sk_rect, &radii_arr);
            canvas.draw_rrect(rrect, &p);
        } else {
            canvas.draw_rect(to_sk_rect(rect), &p);
        }
    }
}

fn draw_shadowed_rect(
    canvas: &skia_safe::Canvas,
    rect: Rect,
    radii: BorderRadii,
    shadow: &jian_core::render::ShadowSpec,
) {
    // Draw a blurred copy of the shape in `shadow.color` offset by
    // (dx, dy). The foreground fill lands via a subsequent draw-op.
    let c4 = to_sk_color(shadow.color);
    let mut paint = SkPaint::new(c4, None);
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);
    // Skia's blur filter uses sigma; CSS-ish `blur` is ~= sigma * 2.
    let sigma = (shadow.blur * 0.5).max(0.0);
    if sigma > 0.0 {
        paint.set_mask_filter(skia_safe::MaskFilter::blur(
            BlurStyle::Normal,
            sigma,
            None,
        ));
    }
    let offset_rect = Rect::new(
        jian_core::geometry::point(rect.min_x() + shadow.dx, rect.min_y() + shadow.dy),
        rect.size,
    );
    let is_rounded = radii != BorderRadii::zero();
    if is_rounded {
        let sk_rect = to_sk_rect(offset_rect);
        let radii_arr = [
            SkPoint::new(radii.tl, radii.tl),
            SkPoint::new(radii.tr, radii.tr),
            SkPoint::new(radii.br, radii.br),
            SkPoint::new(radii.bl, radii.bl),
        ];
        let rrect = RRect::new_rect_radii(sk_rect, &radii_arr);
        canvas.draw_rrect(rrect, &paint);
    } else {
        canvas.draw_rect(to_sk_rect(offset_rect), &paint);
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

// Convenience: keeps `BlurStyle` / `MaskFilter` imported for future
// stroke-expansion work without triggering an unused-import warning.
#[allow(dead_code)]
fn _unused_keeping_imports() {
    let _ = BlurStyle::Normal;
    let _ = MaskFilter::blur(BlurStyle::Normal, 1.0, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::geometry::{point, rect, size};
    use jian_core::render::PathCommand;
    use jian_core::scene::Color;

    fn draw_blue_rect() -> SkiaSurface {
        let mut backend = SkiaBackend::new();
        let mut surface = backend.new_surface(size(64.0, 64.0));
        backend.begin_frame(&mut surface, 0xffffffff);
        backend.draw(&DrawOp::Rect {
            rect: rect(8.0, 8.0, 48.0, 48.0),
            paint: Paint::solid(Color::rgb(0x1e, 0x88, 0xe5)),
        });
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
    fn rounded_rect_draws_through_trait() {
        let mut backend = SkiaBackend::new();
        let mut surface = backend.new_surface(size(32.0, 32.0));
        backend.begin_frame(&mut surface, 0);
        backend.draw(&DrawOp::RoundedRect {
            rect: rect(4.0, 4.0, 24.0, 24.0),
            radii: BorderRadii::uniform(4.0),
            paint: Paint::solid(Color::rgb(0xff, 0x00, 0x00)),
        });
        backend.end_frame(&mut surface);
        assert!(surface.encode_png().is_some());
    }

    #[test]
    fn path_triangle_draws_through_trait() {
        let mut backend = SkiaBackend::new();
        let mut surface = backend.new_surface(size(32.0, 32.0));
        backend.begin_frame(&mut surface, 0);
        let cmds: Vec<PathCommand> = vec![
            PathCommand::MoveTo(point(4.0, 28.0)),
            PathCommand::LineTo(point(28.0, 28.0)),
            PathCommand::LineTo(point(16.0, 4.0)),
            PathCommand::Close,
        ];
        backend.draw(&DrawOp::Path {
            commands: cmds,
            paint: Paint::solid(Color::rgb(0x00, 0xff, 0x00)),
        });
        backend.end_frame(&mut surface);
        assert!(surface.encode_png().is_some());
    }

    #[test]
    fn clip_transform_save_pop_cycle() {
        // A clip inside a transform inside a save should not panic on
        // end_frame — and the restore-to-count should clean up a
        // missing pop.
        let mut backend = SkiaBackend::new();
        let mut surface = backend.new_surface(size(32.0, 32.0));
        backend.begin_frame(&mut surface, 0xffffffff);
        backend.push_transform(&Affine2::translation(4.0, 4.0));
        backend.push_clip(rect(0.0, 0.0, 16.0, 16.0));
        backend.draw(&DrawOp::Rect {
            rect: rect(0.0, 0.0, 100.0, 100.0),
            paint: Paint::solid(Color::rgb(0, 0, 0)),
        });
        backend.pop();
        backend.pop();
        backend.end_frame(&mut surface);
        assert!(surface.encode_png().is_some());
    }

    #[test]
    fn apply_blur_then_push_layer_consumes_filter() {
        let mut backend = SkiaBackend::new();
        let mut surface = backend.new_surface(size(32.0, 32.0));
        backend.begin_frame(&mut surface, 0xffffffff);
        backend.apply_blur(4.0);
        assert!(backend.pending_filter.is_some());
        backend.push_layer(rect(0.0, 0.0, 32.0, 32.0));
        assert!(backend.pending_filter.is_none());
        backend.draw(&DrawOp::Rect {
            rect: rect(4.0, 4.0, 24.0, 24.0),
            paint: Paint::solid(Color::rgb(0xff, 0, 0)),
        });
        backend.pop_layer();
        backend.end_frame(&mut surface);
        assert!(surface.encode_png().is_some());
    }

    #[test]
    fn apply_shadow_then_push_layer_consumes_filter() {
        let mut backend = SkiaBackend::new();
        let mut surface = backend.new_surface(size(32.0, 32.0));
        backend.begin_frame(&mut surface, 0xffffffff);
        backend.apply_shadow(&ShadowSpec {
            color: Color::rgba(0, 0, 0, 128),
            dx: 2.0,
            dy: 2.0,
            blur: 3.0,
            spread: 0.0,
        });
        assert!(backend.pending_filter.is_some());
        backend.push_layer(rect(0.0, 0.0, 32.0, 32.0));
        assert!(backend.pending_filter.is_none());
        backend.draw(&DrawOp::Rect {
            rect: rect(4.0, 4.0, 24.0, 24.0),
            paint: Paint::solid(Color::rgb(0, 0, 0xff)),
        });
        backend.pop_layer();
        backend.end_frame(&mut surface);
        assert!(surface.encode_png().is_some());
    }
}
