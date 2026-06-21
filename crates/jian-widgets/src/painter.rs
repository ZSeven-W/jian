//! Platform-neutral painter contract for immediate-mode widgets.

use crate::geometry::{fold_alpha, Color, Point2D, Rect};
use jian_core::render::{TextAlign, TextRun};

/// Text layout passed through to a backend-owned text renderer.
#[derive(Debug, Clone)]
pub struct TextLayout {
    runs: Vec<TextRun>,
    italic: bool,
}

/// Raster image placement mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageDrawMode {
    #[default]
    Fill,
    Fit,
    Crop,
    Tile,
    Stretch,
}

/// Per-image adjustment values in the UI slider range `[-100, 100]`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImageAdjustments {
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub temperature: f32,
    pub tint: f32,
    pub highlights: f32,
    pub shadows: f32,
}

impl ImageAdjustments {
    pub fn is_neutral(self) -> bool {
        self.exposure == 0.0
            && self.contrast == 0.0
            && self.saturation == 0.0
            && self.temperature == 0.0
            && self.tint == 0.0
            && self.highlights == 0.0
            && self.shadows == 0.0
    }
}

impl TextLayout {
    pub fn single_run(
        content: &str,
        font_family: &str,
        font_size: f32,
        color: jian_core::scene::Color,
        origin: Point2D,
    ) -> Self {
        let run = TextRun {
            content: content.to_string(),
            font_family: font_family.to_string(),
            font_size,
            font_weight: 400,
            color,
            origin: jian_core::geometry::Point::new(origin.x, origin.y),
            max_width: 0.0,
            align: TextAlign::Start,
            line_height: 0.0,
        };
        Self {
            runs: vec![run],
            italic: false,
        }
    }

    pub fn runs(&self) -> &[TextRun] {
        &self.runs
    }

    pub fn with_font_weight(mut self, weight: u16) -> Self {
        for r in &mut self.runs {
            r.font_weight = weight;
        }
        self
    }

    pub fn with_italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    pub fn italic(&self) -> bool {
        self.italic
    }

    pub fn translated(&self, offset: Point2D) -> Self {
        let runs = self
            .runs
            .iter()
            .map(|r| {
                let mut r2 = r.clone();
                r2.origin =
                    jian_core::geometry::Point::new(r.origin.x + offset.x, r.origin.y + offset.y);
                r2
            })
            .collect();
        Self {
            runs,
            italic: self.italic,
        }
    }
}

/// Backend abstraction used by cross-platform widgets.
pub trait Painter {
    fn begin_frame(&mut self);
    fn end_frame(&mut self);

    fn fill_rect(&mut self, rect: Rect, color: Color);
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32);
    fn draw_text(&mut self, layout: &TextLayout, origin: Point2D);
    fn clip_rect(&mut self, rect: Rect);

    fn clip_round_rect(&mut self, rect: Rect, radius: f32) {
        let _ = radius;
        self.clip_rect(rect);
    }

    fn stroke_line(&mut self, from: Point2D, to: Point2D, color: Color, width: f32);
    fn fill_round_rect(&mut self, rect: Rect, radius: f32, color: Color);
    fn stroke_round_rect(&mut self, rect: Rect, radius: f32, color: Color, width: f32);
    fn stroke_svg_path(&mut self, d: &str, top_left: Point2D, size: f32, color: Color, width: f32);

    fn fill_svg_path(
        &mut self,
        _d: &str,
        _top_left: Point2D,
        _size: f32,
        _viewbox: f32,
        _color: Color,
    ) {
    }

    fn fill_svg_path_in_rect(&mut self, d: &str, rect: Rect, color: Color) {
        self.fill_svg_path(d, rect.origin, rect.size.x.max(rect.size.y), 1.0, color);
    }

    fn stroke_svg_path_in_rect(&mut self, d: &str, rect: Rect, color: Color, width: f32) {
        self.stroke_svg_path(d, rect.origin, rect.size.x.max(rect.size.y), color, width);
    }

    fn fill_svg_path_in_rect_linear_gradient(
        &mut self,
        d: &str,
        rect: Rect,
        stops: &[(f32, Color)],
        _angle_deg: f32,
        opacity: f32,
    ) {
        if let Some((_, c)) = stops.first() {
            self.fill_svg_path_in_rect(d, rect, fold_alpha(*c, opacity));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_inner_shadow_svg_path(
        &mut self,
        _d: &str,
        _rect: Rect,
        _offset_x: f32,
        _offset_y: f32,
        _blur: f32,
        _color: Color,
    ) {
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_svg_path_in_rect_radial_gradient(
        &mut self,
        d: &str,
        rect: Rect,
        stops: &[(f32, Color)],
        _cx_frac: f32,
        _cy_frac: f32,
        _radius_frac: f32,
        opacity: f32,
    ) {
        if let Some((_, c)) = stops.first() {
            self.fill_svg_path_in_rect(d, rect, fold_alpha(*c, opacity));
        }
    }

    fn fill_drop_shadow(&mut self, rect: Rect, radius: f32, blur: f32, color: Color) {
        let _ = blur;
        self.fill_round_rect(rect, radius, color);
    }

    fn fill_oval(&mut self, bounds: Rect, color: Color) {
        let radius = bounds.size.x.min(bounds.size.y) / 2.0;
        self.fill_round_rect(bounds, radius, color);
    }

    fn stroke_oval(&mut self, bounds: Rect, color: Color, width: f32) {
        let radius = bounds.size.x.min(bounds.size.y) / 2.0;
        self.stroke_round_rect(bounds, radius, color, width);
    }

    fn fill_dots(&mut self, centers: &[Point2D], radius: f32, color: Color) {
        for c in centers {
            self.fill_oval(
                Rect {
                    origin: Point2D::new(c.x - radius, c.y - radius),
                    size: Point2D::new(radius * 2.0, radius * 2.0),
                },
                color,
            );
        }
    }

    fn fill_polygon(&mut self, _points: &[Point2D], _color: Color) {}

    fn stroke_polygon(&mut self, points: &[Point2D], color: Color, width: f32) {
        if points.len() < 2 {
            return;
        }
        for i in 0..points.len() {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            self.stroke_line(a, b, color, width);
        }
    }

    fn draw_image(&mut self, _rect: Rect, _image_id: u64, _encoded: &[u8]) {}

    fn draw_image_with_mode(
        &mut self,
        rect: Rect,
        image_id: u64,
        encoded: &[u8],
        mode: ImageDrawMode,
    ) {
        let _ = mode;
        self.draw_image(rect, image_id, encoded);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_image_with_options(
        &mut self,
        rect: Rect,
        image_id: u64,
        encoded: &[u8],
        mode: ImageDrawMode,
        adjustments: ImageAdjustments,
        opacity: f32,
        corner_radius: f32,
    ) {
        let _ = (adjustments, opacity, corner_radius);
        self.draw_image_with_mode(rect, image_id, encoded, mode);
    }

    fn fill_round_rect_linear_gradient(
        &mut self,
        rect: Rect,
        radius: f32,
        stops: &[(f32, Color)],
        _angle_deg: f32,
        opacity: f32,
    ) {
        if let Some((_, c)) = stops.first() {
            self.fill_round_rect(rect, radius, fold_alpha(*c, opacity));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_round_rect_radial_gradient(
        &mut self,
        rect: Rect,
        radius: f32,
        stops: &[(f32, Color)],
        _cx_frac: f32,
        _cy_frac: f32,
        _radius_frac: f32,
        opacity: f32,
    ) {
        if let Some((_, c)) = stops.first() {
            self.fill_round_rect(rect, radius, fold_alpha(*c, opacity));
        }
    }

    fn save(&mut self);
    fn restore(&mut self);
    fn translate(&mut self, offset: Point2D);
    fn scale(&mut self, _scale: Point2D, _pivot: Point2D) {}
    fn rotate(&mut self, _radians: f32, _pivot: Point2D) {}

    fn resize(&mut self, width: u32, height: u32);
    fn dpi_scale(&self) -> f32;

    fn measure_text(&mut self, text: &str, font_size: f32) -> f32 {
        self.measure_text_weighted(text, font_size, 400)
    }

    fn measure_text_weighted(&mut self, text: &str, font_size: f32, _weight: u16) -> f32 {
        let mut w = 0.0;
        for c in text.chars() {
            w += if c.is_ascii() {
                font_size * 0.55
            } else {
                font_size
            };
        }
        w
    }

    fn measure_text_styled(
        &mut self,
        text: &str,
        font_size: f32,
        weight: u16,
        italic: bool,
    ) -> f32 {
        let _ = italic;
        self.measure_text_weighted(text, font_size, weight)
    }

    /// Measure text width using a specific font `family`, so an editable
    /// field's caret / selection geometry lines up with the glyphs it
    /// actually paints. The family-blind [`Painter::measure_text`]
    /// resolves the backend's default font, which on native differs from
    /// a named draw family like "Inter" (named families go through the
    /// system `FontMgr`, the default uses bundled Roboto) — that gap is
    /// what makes a hand-positioned caret drift. The default impl
    /// forwards to `measure_text`; backends that resolve named families
    /// (native skia) override this to measure with `family`.
    fn measure_text_family(&mut self, text: &str, font_size: f32, _family: &str) -> f32 {
        self.measure_text(text, font_size)
    }

    fn measure_text_family_styled(
        &mut self,
        text: &str,
        font_size: f32,
        family: &str,
        weight: u16,
        italic: bool,
    ) -> f32 {
        let _ = family;
        self.measure_text_styled(text, font_size, weight, italic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn fill_svg_path_in_rect_forwards_destination_extent_to_scalar_fallback() {
        let mut p = CapturePainter::default();
        let rect = Rect::xywh(2.0, 3.0, 12.0, 18.0);

        p.fill_svg_path_in_rect("M0 0h1v1z", rect, Color::RED);

        assert!(matches!(
            p.ops.as_slice(),
            [PaintOp::FillSvgPath {
                top_left,
                size,
                viewbox,
                color,
                ..
            }] if *top_left == rect.origin
                && *size == rect.size.y
                && *viewbox == 1.0
                && *color == Color::RED
        ));
    }
}
