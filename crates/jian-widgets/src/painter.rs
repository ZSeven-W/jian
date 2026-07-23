//! Platform-neutral painter contract for immediate-mode widgets.

use crate::geometry::{fold_alpha, Color, Point2D, Rect};
use jian_core::render::{TextAlign, TextRun};

/// Text layout passed through to a backend-owned text renderer.
#[derive(Debug, Clone)]
pub struct TextLayout {
    runs: Vec<TextRun>,
    italic: bool,
}

/// Inputs used to locate the first alphabetic baseline inside a text line.
///
/// `line_height` is the authored multiplier (`0` means backend default).
/// Including the actual line text lets a backend resolve the same fallback
/// face used for CJK/emoji paint instead of sampling an unrelated Latin glyph.
#[derive(Debug, Clone, Copy)]
pub struct TextBaselineRequest<'a> {
    pub text: &'a str,
    pub font_family: &'a str,
    pub font_size: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub line_height: f32,
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

/// Compositing mode applied to an isolated paint layer or raster image.
///
/// `Normal` preserves the historical source-over behaviour. The remaining
/// variants mirror the blend modes supported by the canonical `.op` schema
/// and map directly onto Skia / CanvasKit paint modes in host backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageBlendMode {
    #[default]
    Normal,
    Darken,
    Multiply,
    Screen,
    Overlay,
    Lighten,
    Difference,
    Hue,
    Saturation,
    Color,
    Luminosity,
    SoftLight,
    ColorDodge,
    ColorBurn,
    HardLight,
    Exclusion,
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

    fn clip_round_rect_per_corner(&mut self, rect: Rect, radii: [f32; 4]) {
        self.clip_round_rect(rect, radii[0]);
    }

    /// Intersect the current clip with an oval inscribed in `bounds`.
    /// Basic painters retain a bounded rounded-rect approximation; rich
    /// backends override this with their native oval/path clip.
    fn clip_oval(&mut self, bounds: Rect) {
        self.clip_round_rect(bounds, bounds.size.x.min(bounds.size.y) / 2.0);
    }

    /// Intersect the current clip with a closed polygon. Basic painters use
    /// its AABB so the API remains source-compatible; rendering hosts override
    /// this with an exact path clip.
    fn clip_polygon(&mut self, points: &[Point2D]) {
        if points.len() < 3 {
            self.clip_rect(Rect::ZERO);
            return;
        }
        let first = points[0];
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (first.x, first.y, first.x, first.y);
        for point in &points[1..] {
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
        self.clip_rect(Rect::xywh(min_x, min_y, max_x - min_x, max_y - min_y));
    }

    /// Intersect the current clip with an SVG path fitted into `rect`.
    /// Rich backends honor `even_odd`; the compatibility fallback stays
    /// bounded to the destination rectangle.
    fn clip_svg_path_in_rect(&mut self, d: &str, rect: Rect, even_odd: bool) {
        let _ = (d, even_odd);
        self.clip_rect(rect);
    }

    fn stroke_line(&mut self, from: Point2D, to: Point2D, color: Color, width: f32);
    fn fill_round_rect(&mut self, rect: Rect, radius: f32, color: Color);
    fn stroke_round_rect(&mut self, rect: Rect, radius: f32, color: Color, width: f32);
    fn fill_round_rect_per_corner(&mut self, rect: Rect, radii: [f32; 4], color: Color) {
        self.fill_round_rect(rect, radii[0], color);
    }
    fn stroke_round_rect_per_corner(
        &mut self,
        rect: Rect,
        radii: [f32; 4],
        color: Color,
        width: f32,
    ) {
        self.stroke_round_rect(rect, radii[0], color, width);
    }
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

    #[allow(clippy::too_many_arguments)]
    fn fill_svg_path_with_fill_rule(
        &mut self,
        d: &str,
        top_left: Point2D,
        size: f32,
        viewbox: f32,
        color: Color,
        even_odd: bool,
    ) {
        let _ = even_odd;
        self.fill_svg_path(d, top_left, size, viewbox, color);
    }

    fn fill_svg_path_in_rect(&mut self, d: &str, rect: Rect, color: Color) {
        self.fill_svg_path(d, rect.origin, rect.size.x.max(rect.size.y), 1.0, color);
    }

    fn fill_svg_path_in_rect_with_fill_rule(
        &mut self,
        d: &str,
        rect: Rect,
        color: Color,
        even_odd: bool,
    ) {
        let _ = even_odd;
        self.fill_svg_path_in_rect(d, rect, color);
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
    fn fill_svg_path_in_rect_linear_gradient_with_fill_rule(
        &mut self,
        d: &str,
        rect: Rect,
        stops: &[(f32, Color)],
        angle_deg: f32,
        opacity: f32,
        even_odd: bool,
    ) {
        let _ = even_odd;
        self.fill_svg_path_in_rect_linear_gradient(d, rect, stops, angle_deg, opacity);
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
    fn fill_inner_shadow_svg_path_with_fill_rule(
        &mut self,
        d: &str,
        rect: Rect,
        offset_x: f32,
        offset_y: f32,
        blur: f32,
        color: Color,
        even_odd: bool,
    ) {
        let _ = even_odd;
        self.fill_inner_shadow_svg_path(d, rect, offset_x, offset_y, blur, color);
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

    #[allow(clippy::too_many_arguments)]
    fn fill_svg_path_in_rect_radial_gradient_with_fill_rule(
        &mut self,
        d: &str,
        rect: Rect,
        stops: &[(f32, Color)],
        cx_frac: f32,
        cy_frac: f32,
        radius_frac: f32,
        opacity: f32,
        even_odd: bool,
    ) {
        let _ = even_odd;
        self.fill_svg_path_in_rect_radial_gradient(
            d,
            rect,
            stops,
            cx_frac,
            cy_frac,
            radius_frac,
            opacity,
        );
    }

    fn fill_drop_shadow(&mut self, rect: Rect, radius: f32, blur: f32, color: Color) {
        let _ = blur;
        self.fill_round_rect(rect, radius, color);
    }

    /// Begin a Gaussian-blur layer: subsequent draws are captured into
    /// an offscreen layer that is blurred by `sigma` (px) when the
    /// matching [`Painter::restore`] pops it. `sigma <= 0` is a plain
    /// [`Painter::save`]. Backends without layer-filter support fall
    /// back to `save` (no blur) so callers stay balanced.
    fn push_blur_layer(&mut self, sigma: f32) {
        let _ = sigma;
        self.save();
    }

    /// Begin a bounded offscreen compositing layer. Draws until the matching
    /// [`Painter::restore`] are first combined in isolation, then land on the
    /// backdrop once with `opacity` and `mode`. Rich backends must create a
    /// real save-layer even for fully opaque source-over; that case still
    /// provides the isolation required by background blend stacks.
    fn push_composite_layer(&mut self, bounds: Rect, opacity: f32, mode: ImageBlendMode) {
        let _ = (bounds, opacity, mode);
        self.save();
    }

    /// Whether this backend can assemble a deferred mask source and composite
    /// it into an isolated destination with Porter-Duff `DstIn`.
    fn supports_pixel_masks(&self) -> bool {
        false
    }

    /// Begin the mask-source save-layer. On matching [`Painter::restore`],
    /// the assembled source is applied to the current isolated content using
    /// `DstIn`; luminance mode first converts source luminance into alpha.
    /// Callers must guard this with [`Painter::supports_pixel_masks`].
    fn push_mask_source_layer(&mut self, luminance: bool) {
        let _ = luminance;
        self.save();
    }

    /// Begin an isolated compositing layer. Draws until the matching
    /// [`Painter::restore`] are blended with the backdrop using `mode`.
    /// Backends without save-layer blend support degrade to source-over while
    /// preserving the caller's save/restore balance.
    fn push_blend_layer(&mut self, mode: ImageBlendMode) {
        let _ = mode;
        self.save();
    }

    /// Begin a save-layer initialized from a blurred copy of the
    /// already-painted backdrop. The caller owns the silhouette clip
    /// and balances this layer with [`Painter::restore`].
    fn push_backdrop_blur_layer(&mut self, sigma: f32) {
        let _ = sigma;
        self.save();
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

    /// True when drawing this image would not synchronously decode it
    /// AND the cached raster is at least `max_edge_px` on its longest
    /// edge. Backends that raster per required size answer `false` for a
    /// too-coarse cache hit so paint can request a sharper decode while
    /// still drawing what it already has. Backends without an
    /// asynchronous decode path keep the existing behavior by accepting
    /// encoded bytes as immediately drawable.
    fn image_decoded(&mut self, id: u64, encoded: &[u8], max_edge_px: u32) -> bool {
        let _ = (id, encoded, max_edge_px);
        true
    }

    /// True when SOME raster for this image is resident, even one too
    /// coarse for the current zoom. Paint draws it while a sharper
    /// decode is in flight; without this a zoom-in would drop a
    /// perfectly good image back to placeholder art.
    fn image_resident(&mut self, id: u64) -> bool {
        let _ = id;
        true
    }

    /// Draw a small blur-up placeholder raster for a full image that is still
    /// decoding. Platform backends may synchronously decode this bounded JPEG
    /// into a dedicated thumbnail cache; the full-image cache is untouched.
    fn draw_image_thumb(&mut self, rect: Rect, image_id: u64, jpeg: &[u8]) {
        let _ = (rect, image_id, jpeg);
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

    /// Draw an image with an optional Figma affine fill transform.
    ///
    /// `transform` maps the destination node's normalized unit square into
    /// normalized image UV coordinates. Backends that have not implemented
    /// affine image sampling retain the existing placement-mode behaviour.
    #[allow(clippy::too_many_arguments)]
    fn draw_image_with_options_and_transform(
        &mut self,
        rect: Rect,
        image_id: u64,
        encoded: &[u8],
        mode: ImageDrawMode,
        adjustments: ImageAdjustments,
        opacity: f32,
        corner_radius: f32,
        transform: Option<[f32; 6]>,
    ) {
        let _ = transform;
        self.draw_image_with_options(
            rect,
            image_id,
            encoded,
            mode,
            adjustments,
            opacity,
            corner_radius,
        );
    }

    /// Draw an image with affine sampling and an explicit compositing mode.
    ///
    /// The default deliberately routes through the pre-existing transform
    /// method so third-party painters remain source-compatible and render new
    /// documents as `Normal` until they opt into blend support.
    #[allow(clippy::too_many_arguments)]
    fn draw_image_with_options_transform_and_blend(
        &mut self,
        rect: Rect,
        image_id: u64,
        encoded: &[u8],
        mode: ImageDrawMode,
        adjustments: ImageAdjustments,
        opacity: f32,
        corner_radius: f32,
        transform: Option<[f32; 6]>,
        blend_mode: ImageBlendMode,
    ) {
        let _ = blend_mode;
        self.draw_image_with_options_and_transform(
            rect,
            image_id,
            encoded,
            mode,
            adjustments,
            opacity,
            corner_radius,
            transform,
        );
    }

    /// Draw an image with a dedicated TILE scale. The additive entry point
    /// keeps existing painter implementations source-compatible; backends
    /// that do not opt in retain the historical neutral scale.
    #[allow(clippy::too_many_arguments)]
    fn draw_image_with_options_transform_blend_and_tile_scale(
        &mut self,
        rect: Rect,
        image_id: u64,
        encoded: &[u8],
        mode: ImageDrawMode,
        adjustments: ImageAdjustments,
        opacity: f32,
        corner_radius: f32,
        transform: Option<[f32; 6]>,
        blend_mode: ImageBlendMode,
        original_size: Option<[f32; 2]>,
        tile_scale: f32,
    ) {
        let _ = (original_size, tile_scale);
        self.draw_image_with_options_transform_and_blend(
            rect,
            image_id,
            encoded,
            mode,
            adjustments,
            opacity,
            corner_radius,
            transform,
            blend_mode,
        );
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
    fn fill_round_rect_linear_gradient_per_corner(
        &mut self,
        rect: Rect,
        radii: [f32; 4],
        stops: &[(f32, Color)],
        _angle_deg: f32,
        opacity: f32,
    ) {
        if let Some((_, c)) = stops.first() {
            self.fill_round_rect_per_corner(rect, radii, fold_alpha(*c, opacity));
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

    #[allow(clippy::too_many_arguments)]
    fn fill_round_rect_radial_gradient_per_corner(
        &mut self,
        rect: Rect,
        radii: [f32; 4],
        stops: &[(f32, Color)],
        _cx_frac: f32,
        _cy_frac: f32,
        _radius_frac: f32,
        opacity: f32,
    ) {
        if let Some((_, c)) = stops.first() {
            self.fill_round_rect_per_corner(rect, radii, fold_alpha(*c, opacity));
        }
    }

    /// Paint a uniform-grid mesh gradient. `colors` is a row-major
    /// `rows`×`cols` lattice. The default impl falls back to the
    /// first-vertex colour as a flat fill — backends that can Gouraud-
    /// interpolate (the native Skia host) override this. Keeping the
    /// solid fallback here lets the capture / CanvasKit / frame backends
    /// compile and render *something* without per-vertex support.
    fn fill_round_rect_mesh_gradient(
        &mut self,
        rect: Rect,
        radius: f32,
        _rows: u32,
        _cols: u32,
        colors: &[Color],
        opacity: f32,
    ) {
        if let Some(c) = colors.first() {
            self.fill_round_rect(rect, radius, fold_alpha(*c, opacity));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_round_rect_mesh_gradient_per_corner(
        &mut self,
        rect: Rect,
        radii: [f32; 4],
        _rows: u32,
        _cols: u32,
        colors: &[Color],
        opacity: f32,
    ) {
        if let Some(c) = colors.first() {
            self.fill_round_rect_per_corner(rect, radii, fold_alpha(*c, opacity));
        }
    }

    /// Paint a native SkSL shader fill. `sksl` is the RAW (untrusted)
    /// source (entrypoint `half4 main(float2 fragCoord)`); `uniforms`
    /// carries `(name, values)` bindings (length 1 = float, 2/3/4 =
    /// vec*); `fallback` is the visible solid colour to paint when the
    /// backend can't compile / run the program. The default impl is the
    /// solid fallback — only the native Skia host overrides this with a
    /// real cached `RuntimeEffect`. Web / capture / frame backends keep
    /// this fallback (documented parity gap, same as mesh gradients).
    fn fill_round_rect_shader(
        &mut self,
        rect: Rect,
        radius: f32,
        _sksl: &str,
        _uniforms: &[(&str, &[f32])],
        opacity: f32,
        fallback: Color,
    ) {
        self.fill_round_rect(rect, radius, fold_alpha(fallback, opacity));
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_round_rect_shader_per_corner(
        &mut self,
        rect: Rect,
        radii: [f32; 4],
        _sksl: &str,
        _uniforms: &[(&str, &[f32])],
        opacity: f32,
        fallback: Color,
    ) {
        self.fill_round_rect_per_corner(rect, radii, fold_alpha(fallback, opacity));
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

    /// Distance from a text run's top edge to its alphabetic baseline.
    /// Backends without font metrics preserve the historical approximation.
    fn text_ascent(&mut self, font_size: f32, _weight: u16) -> f32 {
        font_size * 0.8
    }

    /// Family-aware ascent for canvas text. The default keeps compatibility
    /// with backends that only implement [`Painter::text_ascent`].
    fn text_ascent_family(&mut self, font_size: f32, _family: &str, weight: u16) -> f32 {
        self.text_ascent(font_size, weight)
    }

    /// Distance from the top of the first line box to its alphabetic baseline.
    /// Rich backends shape `request.text` with the requested family/style and
    /// authored line height. The default deliberately preserves the historical
    /// ascent-only behavior for capture/estimate backends.
    fn text_first_baseline(&mut self, request: &TextBaselineRequest<'_>) -> f32 {
        let _ = (request.text, request.italic, request.line_height);
        self.text_ascent_family(request.font_size, request.font_family, request.font_weight)
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

    #[test]
    fn fill_rule_variant_defaults_to_existing_path_fill() {
        let mut p = CapturePainter::default();
        let rect = Rect::xywh(2.0, 3.0, 12.0, 18.0);

        p.fill_svg_path_in_rect_with_fill_rule("M0 0h1v1z", rect, Color::RED, true);

        assert!(matches!(p.ops.as_slice(), [PaintOp::FillSvgPath { .. }]));
    }

    #[test]
    fn image_thumb_hook_defaults_to_a_no_op() {
        let mut p = CapturePainter::default();

        p.draw_image_thumb(Rect::xywh(0.0, 0.0, 20.0, 10.0), 7, b"jpeg");

        assert!(p.ops.is_empty());
    }
}
