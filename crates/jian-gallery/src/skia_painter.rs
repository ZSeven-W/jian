use jian_skia::SkiaSurface;
use jian_widgets::{Color, Painter, Point2D, Rect, TextLayout, TextMetrics};
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::{
    Color as SkColor, Color4f, Font, FontMgr, FontStyle, Paint as SkPaint, PaintStyle,
    Point as SkPoint, RRect, Rect as SkRect, Typeface,
};

pub struct SkiaWidgetPainter<'a> {
    surface: &'a mut SkiaSurface,
    dpi_scale: f32,
}

impl<'a> SkiaWidgetPainter<'a> {
    pub fn new(surface: &'a mut SkiaSurface, dpi_scale: f32) -> Self {
        Self {
            surface,
            dpi_scale: dpi_scale.max(0.01),
        }
    }

    fn canvas(&mut self) -> &skia_safe::Canvas {
        self.surface.canvas()
    }
}

impl Painter for SkiaWidgetPainter<'_> {
    fn begin_frame(&mut self) {
        let scale = self.dpi_scale;
        self.canvas().clear(SkColor::TRANSPARENT);
        self.canvas().save();
        if (scale - 1.0).abs() > f32::EPSILON {
            self.canvas().scale((scale, scale));
        }
    }

    fn end_frame(&mut self) {
        self.canvas().restore_to_count(1);
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let mut paint = paint(color, PaintStyle::Fill);
        paint.set_anti_alias(true);
        self.canvas().draw_rect(to_sk_rect(rect), &paint);
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32) {
        let mut paint = paint(color, PaintStyle::Stroke);
        paint.set_stroke_width(width);
        paint.set_anti_alias(true);
        self.canvas().draw_rect(to_sk_rect(rect), &paint);
    }

    fn draw_text(&mut self, layout: &TextLayout, origin: Point2D) {
        for run in layout.runs() {
            let color = Color::rgba_u8(
                run.color.r(),
                run.color.g(),
                run.color.b(),
                run.color.a() as f32 / 255.0,
            );
            let mut paint = SkPaint::new(to_sk_color(color), None);
            paint.set_anti_alias(true);
            let x = origin.x + run.origin.x;
            let y = origin.y + run.origin.y + run.font_size;
            if run.content.is_ascii() {
                let font = font_for(
                    &run.font_family,
                    run.font_size,
                    run.font_weight,
                    layout.italic(),
                );
                self.canvas()
                    .draw_str(&run.content, SkPoint::new(x, y), &font, &paint);
            } else {
                draw_text_with_fallback(
                    self.canvas(),
                    &run.content,
                    SkPoint::new(x, y),
                    &run.font_family,
                    run.font_size,
                    run.font_weight,
                    layout.italic(),
                    &paint,
                );
            }
        }
    }

    fn clip_rect(&mut self, rect: Rect) {
        self.canvas().clip_rect(to_sk_rect(rect), None, true);
    }

    fn clip_round_rect(&mut self, rect: Rect, radius: f32) {
        self.canvas()
            .clip_rrect(to_sk_rrect(rect, radius), None, true);
    }

    fn stroke_line(&mut self, from: Point2D, to: Point2D, color: Color, width: f32) {
        let mut paint = paint(color, PaintStyle::Stroke);
        paint.set_stroke_width(width);
        paint.set_anti_alias(true);
        self.canvas().draw_line(
            SkPoint::new(from.x, from.y),
            SkPoint::new(to.x, to.y),
            &paint,
        );
    }

    fn fill_round_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        let mut paint = paint(color, PaintStyle::Fill);
        paint.set_anti_alias(true);
        self.canvas().draw_rrect(to_sk_rrect(rect, radius), &paint);
    }

    fn stroke_round_rect(&mut self, rect: Rect, radius: f32, color: Color, width: f32) {
        let mut paint = paint(color, PaintStyle::Stroke);
        paint.set_stroke_width(width);
        paint.set_anti_alias(true);
        self.canvas().draw_rrect(to_sk_rrect(rect, radius), &paint);
    }

    fn stroke_svg_path(
        &mut self,
        _d: &str,
        top_left: Point2D,
        size: f32,
        color: Color,
        width: f32,
    ) {
        let mut paint = paint(color, PaintStyle::Stroke);
        paint.set_stroke_width(width);
        paint.set_anti_alias(true);
        let a = SkPoint::new(top_left.x + size * 0.2, top_left.y + size * 0.5);
        let b = SkPoint::new(top_left.x + size * 0.8, top_left.y + size * 0.5);
        self.canvas().draw_line(a, b, &paint);
    }

    fn fill_svg_path(
        &mut self,
        _d: &str,
        top_left: Point2D,
        size: f32,
        _viewbox: f32,
        color: Color,
    ) {
        self.fill_round_rect(Rect::xywh(top_left.x, top_left.y, size, size), 2.0, color);
    }

    fn fill_drop_shadow(&mut self, rect: Rect, radius: f32, _blur: f32, color: Color) {
        let shadow = Rect::xywh(rect.origin.x, rect.origin.y + 4.0, rect.size.x, rect.size.y);
        self.fill_round_rect(shadow, radius, color);
    }

    fn fill_oval(&mut self, bounds: Rect, color: Color) {
        let mut paint = paint(color, PaintStyle::Fill);
        paint.set_anti_alias(true);
        self.canvas().draw_oval(to_sk_rect(bounds), &paint);
    }

    fn stroke_oval(&mut self, bounds: Rect, color: Color, width: f32) {
        let mut paint = paint(color, PaintStyle::Stroke);
        paint.set_stroke_width(width);
        paint.set_anti_alias(true);
        self.canvas().draw_oval(to_sk_rect(bounds), &paint);
    }

    fn save(&mut self) {
        self.canvas().save();
    }

    fn restore(&mut self) {
        self.canvas().restore();
    }

    fn translate(&mut self, offset: Point2D) {
        self.canvas().translate((offset.x, offset.y));
    }

    fn scale(&mut self, scale: Point2D, _pivot: Point2D) {
        self.canvas().scale((scale.x, scale.y));
    }

    fn rotate(&mut self, radians: f32, _pivot: Point2D) {
        self.canvas().rotate(radians.to_degrees(), None);
    }

    fn resize(&mut self, _width: u32, _height: u32) {}

    fn dpi_scale(&self) -> f32 {
        self.dpi_scale
    }

    fn measure_text_weighted(&mut self, text: &str, font_size: f32, weight: u16) -> f32 {
        measure_text_with_fallback(text, font_size, weight, false)
    }

    fn measure_text_styled(
        &mut self,
        text: &str,
        font_size: f32,
        weight: u16,
        italic: bool,
    ) -> f32 {
        measure_text_with_fallback(text, font_size, weight, italic)
    }

    fn measure_text_family_styled(
        &mut self,
        text: &str,
        font_size: f32,
        family: &str,
        weight: u16,
        italic: bool,
    ) -> f32 {
        measure_text_with_family_fallback(text, font_size, family, weight, italic)
    }

    fn measure_text_metrics_family_styled(
        &mut self,
        text: &str,
        font_size: f32,
        family: &str,
        weight: u16,
        italic: bool,
    ) -> TextMetrics {
        let width = measure_text_with_family_fallback(text, font_size, family, weight, italic);
        let Some((ink_top, ink_bottom)) =
            text_ink_bounds_with_family_fallback(text, font_size, family, weight, italic)
        else {
            return TextMetrics::line_box(width, font_size);
        };
        TextMetrics {
            width,
            line_height: font_size,
            baseline: font_size,
            ink_top: font_size + ink_top,
            ink_bottom: font_size + ink_bottom,
        }
    }
}

fn paint(color: Color, style: PaintStyle) -> SkPaint {
    let mut paint = SkPaint::new(to_sk_color(color), None);
    paint.set_style(style);
    paint
}

fn to_sk_color(color: Color) -> Color4f {
    Color4f {
        r: color.r,
        g: color.g,
        b: color.b,
        a: color.a,
    }
}

fn to_sk_rect(rect: Rect) -> SkRect {
    SkRect::from_xywh(rect.origin.x, rect.origin.y, rect.size.x, rect.size.y)
}

fn to_sk_rrect(rect: Rect, radius: f32) -> RRect {
    RRect::new_rect_xy(to_sk_rect(rect), radius, radius)
}

fn font_for(family: &str, size: f32, weight: u16, italic: bool) -> Font {
    Font::new(typeface_for(family, weight, italic), size)
}

fn font_for_char(family: &str, size: f32, weight: u16, italic: bool, ch: char) -> Font {
    let style = font_style(weight, italic);
    let manager = FontMgr::new();
    let typeface = manager
        .match_family_style_character(family, style, &["zh-Hans", "en"], ch as _)
        .or_else(|| manager.match_family_style_character("", style, &["zh-Hans", "en"], ch as _))
        .unwrap_or_else(|| typeface_for(family, weight, italic));
    Font::new(typeface, size)
}

fn typeface_for(family: &str, weight: u16, italic: bool) -> Typeface {
    let style = font_style(weight, italic);
    let manager = FontMgr::new();
    manager
        .match_family_style(family, style)
        .or_else(|| manager.legacy_make_typeface(None, style))
        .expect("default typeface")
}

fn font_style(weight: u16, italic: bool) -> FontStyle {
    FontStyle::new(
        Weight::from(weight as i32),
        Width::NORMAL,
        if italic {
            Slant::Italic
        } else {
            Slant::Upright
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_text_with_fallback(
    canvas: &skia_safe::Canvas,
    text: &str,
    origin: SkPoint,
    family: &str,
    size: f32,
    weight: u16,
    italic: bool,
    paint: &SkPaint,
) {
    let mut x = origin.x;
    for ch in text.chars() {
        let s = ch.to_string();
        let font = font_for_char(family, size, weight, italic, ch);
        canvas.draw_str(&s, SkPoint::new(x, origin.y), &font, paint);
        x += font.measure_str(&s, Some(paint)).0;
    }
}

fn measure_text_with_fallback(text: &str, size: f32, weight: u16, italic: bool) -> f32 {
    measure_text_with_family_fallback(text, size, "Inter", weight, italic)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TextMeasureStyle<'a> {
    family: &'a str,
    size: f32,
    weight: u16,
    italic: bool,
}

fn text_measure_style<'a>(
    family: &'a str,
    size: f32,
    weight: u16,
    italic: bool,
) -> TextMeasureStyle<'a> {
    TextMeasureStyle {
        family,
        size,
        weight,
        italic,
    }
}

fn measure_text_with_family_fallback(
    text: &str,
    size: f32,
    family: &str,
    weight: u16,
    italic: bool,
) -> f32 {
    let style = text_measure_style(family, size, weight, italic);
    let paint = SkPaint::default();
    if text.is_ascii() {
        return font_for(style.family, style.size, style.weight, style.italic)
            .measure_str(text, Some(&paint))
            .0;
    }
    text.chars()
        .map(|ch| {
            let s = ch.to_string();
            font_for_char(style.family, style.size, style.weight, style.italic, ch)
                .measure_str(&s, Some(&paint))
                .0
        })
        .sum()
}

fn text_ink_bounds_with_family_fallback(
    text: &str,
    size: f32,
    family: &str,
    weight: u16,
    italic: bool,
) -> Option<(f32, f32)> {
    let mut top = f32::INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for ch in text.chars().filter(|ch| !ch.is_whitespace()) {
        let font = font_for_char(family, size, weight, italic, ch);
        let glyphs = font.str_to_glyphs_vec(ch.to_string());
        let mut found_outline = false;
        for glyph in glyphs {
            let Some(path) = font.get_path(glyph) else {
                continue;
            };
            let bounds = path.compute_tight_bounds();
            if bounds.top.is_finite()
                && bounds.bottom.is_finite()
                && bounds.bottom >= bounds.top
                && bounds.height() > 0.0
            {
                top = top.min(bounds.top);
                bottom = bottom.max(bounds.bottom);
                found_outline = true;
            }
        }
        if !found_outline {
            let (_, bounds) = font.measure_str(ch.to_string(), None);
            top = top.min(bounds.top);
            bottom = bottom.max(bounds.bottom);
        }
    }
    (top.is_finite() && bottom.is_finite() && bottom >= top).then_some((top, bottom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_style_preserves_requested_family_size_and_weight() {
        assert_eq!(
            text_measure_style("Probe Sans", 17.0, 650, false),
            TextMeasureStyle {
                family: "Probe Sans",
                size: 17.0,
                weight: 650,
                italic: false,
            }
        );
    }

    #[test]
    fn text_metrics_follow_the_gallery_baseline_and_real_ink_bounds() {
        let mut surface = SkiaSurface::new_raster(100, 40);
        let mut painter = SkiaWidgetPainter::new(&mut surface, 1.0);
        let metrics =
            painter.measure_text_metrics_family_styled("Center", 13.0, "system-ui", 400, false);
        let (ink_top, ink_bottom) =
            text_ink_bounds_with_family_fallback("Center", 13.0, "system-ui", 400, false)
                .expect("ascii glyphs expose visible bounds");

        assert_eq!(metrics.baseline, 13.0);
        assert!((metrics.ink_top - (13.0 + ink_top)).abs() < 0.001);
        assert!((metrics.ink_bottom - (13.0 + ink_bottom)).abs() < 0.001);
    }
}
