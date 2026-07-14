//! Jian `RenderBackend` implemented by a per-mount CanvasKit bridge.

use crate::canvaskit::{self, CkImage, CkRuntime, CkSurface};
use crate::viewport::resize_backing_store_preserving_css_box;
use crate::{CkMeasure, FontRegistry};
use jian_core::geometry::{Affine2, Rect, Size};
use jian_core::render::{
    probe_image_bounds, BorderRadii, DecodeError, DrawOp, GradientStop, ImageSource, Paint,
    PathCommand, RenderBackend, RichTextGrowth, RichTextPlan, ShadowSpec, StrokeOp, TextAlign,
    TextRun, TextSpan,
};
use jian_core::scene::Color;
use js_sys::Array;
use std::collections::HashMap;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::HtmlCanvasElement;

enum Filter {
    Blur(f32),
    Shadow(ShadowSpec),
}

enum Command {
    Clip(Rect),
    Transform(Affine2),
    Pop,
    Layer(Rect, Option<Filter>),
    PopLayer,
    Draw(DrawOp),
    RichText(TextRun, Vec<TextSpan>, Rect, RichTextGrowth),
}

pub struct CanvasKitSurface {
    inner: CkSurface,
}

impl CanvasKitSurface {
    pub fn read_pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let bytes = self.inner.read_pixel(x, y);
        [
            bytes.first().copied().unwrap_or(0),
            bytes.get(1).copied().unwrap_or(0),
            bytes.get(2).copied().unwrap_or(0),
            bytes.get(3).copied().unwrap_or(0),
        ]
    }

    pub fn region_has_ink(&self, x: u32, y: u32, width: u32, height: u32) -> bool {
        self.inner.region_has_ink(x, y, width, height)
    }

    pub fn last_text_width(&self) -> f32 {
        self.inner.last_text_width()
    }
}

impl Drop for CanvasKitSurface {
    fn drop(&mut self) {
        self.inner.dispose();
    }
}

pub struct CanvasKitBackend {
    runtime: CkRuntime,
    canvas: HtmlCanvasElement,
    commands: Vec<Command>,
    pending_filter: Option<Filter>,
    clear: u32,
    images: HashMap<String, CkImage>,
    dpr: f32,
    preserve_drawing_buffer: bool,
    #[cfg(all(test, target_arch = "wasm32"))]
    frame_trace: Vec<&'static str>,
    #[cfg(all(test, target_arch = "wasm32"))]
    last_frame_trace: Vec<&'static str>,
    #[cfg(all(test, target_arch = "wasm32"))]
    frame_layer_trace: Vec<String>,
    #[cfg(all(test, target_arch = "wasm32"))]
    last_frame_layer_trace: Vec<String>,
}

impl CanvasKitBackend {
    pub async fn load(canvas: HtmlCanvasElement, asset_base: &str) -> Result<Self, JsValue> {
        let runtime = canvaskit::load(asset_base).await?;
        if !runtime.register_font(
            "Roboto",
            "Roboto",
            include_bytes!("../assets/fonts/Roboto-Regular.ttf"),
        ) {
            runtime.dispose_runtime();
            return Err(JsValue::from_str(
                "CanvasKit rejected the bundled default font",
            ));
        }
        if !runtime.register_font(
            "Source Han Sans CN",
            "Source Han Sans CN",
            include_bytes!("../assets/fonts/SourceHanSansCN-Regular.otf"),
        ) {
            runtime.dispose_runtime();
            return Err(JsValue::from_str(
                "CanvasKit rejected the bundled CJK fallback font",
            ));
        }
        Ok(Self {
            runtime,
            canvas,
            commands: Vec::new(),
            pending_filter: None,
            clear: 0,
            images: HashMap::new(),
            dpr: 1.0,
            preserve_drawing_buffer: false,
            #[cfg(all(test, target_arch = "wasm32"))]
            frame_trace: Vec::new(),
            #[cfg(all(test, target_arch = "wasm32"))]
            last_frame_trace: Vec::new(),
            #[cfg(all(test, target_arch = "wasm32"))]
            frame_layer_trace: Vec::new(),
            #[cfg(all(test, target_arch = "wasm32"))]
            last_frame_layer_trace: Vec::new(),
        })
    }

    fn trace(&mut self, entry: &'static str) {
        #[cfg(all(test, target_arch = "wasm32"))]
        self.frame_trace.push(entry);
        #[cfg(not(all(test, target_arch = "wasm32")))]
        let _ = entry;
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn last_frame_trace(&self) -> String {
        self.last_frame_trace.join(" -> ")
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn last_frame_layer_trace(&self) -> String {
        self.last_frame_layer_trace.join(" -> ")
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    fn trace_layer(&mut self, boundary: &str, kind: &str, bounds: Rect) {
        self.frame_layer_trace.push(format!(
            "{boundary}.{kind}=[{:.1},{:.1},{:.1},{:.1}]",
            bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height
        ));
    }

    pub fn has_image(&self, key: &str) -> bool {
        self.images.contains_key(key)
    }

    pub(crate) fn invalidate_images(&mut self) {
        for (_, image) in self.images.drain() {
            self.runtime.delete_image(&image);
        }
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn registered_font_count(&self) -> u32 {
        self.runtime.registered_font_count()
    }

    pub fn font_registry(&self) -> FontRegistry {
        FontRegistry::new(canvaskit::clone_runtime(&self.runtime))
    }

    pub fn measure_backend(&self) -> CkMeasure {
        CkMeasure::new(canvaskit::clone_runtime(&self.runtime))
    }

    pub(crate) fn draw_text_plan(&mut self, run: &TextRun, plan: &RichTextPlan) {
        self.trace("backend.draw_text_plan");
        self.commands.push(Command::RichText(
            run.clone(),
            plan.spans.clone(),
            plan.bounds,
            plan.growth,
        ));
    }

    pub(crate) fn set_dpr(&mut self, dpr: f32) {
        self.dpr = dpr.max(1.0);
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn preserve_drawing_buffer_for_test(&mut self) {
        // The production canvas is presented on rAF and never needs CPU
        // readback. Browser pixel tests do read it after presentation, when a
        // default WebGL framebuffer may otherwise have been discarded.
        self.preserve_drawing_buffer = true;
    }

    pub(crate) fn try_new_surface(&mut self, size: Size) -> Result<CanvasKitSurface, JsValue> {
        let width = size.width.max(1.0).round() as u32;
        let height = size.height.max(1.0).round() as u32;
        resize_backing_store_preserving_css_box(&self.canvas, width, height)?;
        let inner =
            self.runtime
                .make_surface(&self.canvas, width, height, self.preserve_drawing_buffer)?;
        Ok(CanvasKitSurface { inner })
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn fail_next_surface_for_test(&self) {
        self.runtime.fail_next_surface_for_test();
    }

    fn image_for(&mut self, source: &ImageSource) -> Option<CkImage> {
        let key = source.cache_key();
        if let Some(image) = self.images.get(&key) {
            return Some(clone_image(image));
        }
        let bytes = match source {
            ImageSource::DataUrl(data) => {
                jian_core::render::image_store::decode_data_url(data).ok()?
            }
            ImageSource::Bytes(bytes) => bytes.as_ref().clone(),
            ImageSource::Url(_) => return None,
        };
        probe_image_bounds(&bytes).ok()?;
        let image = self.runtime.decode_image(&bytes).ok()?;
        self.images.insert(key, clone_image(&image));
        Some(image)
    }

    fn replay(&mut self, surface: &CkSurface, command: Command) {
        self.trace(match &command {
            Command::Clip(_) => "surface.push_clip",
            Command::Transform(_) => "surface.push_transform",
            Command::Pop => "surface.pop",
            Command::Layer(_, Some(Filter::Blur(_))) => "surface.push_blur_layer",
            Command::Layer(_, Some(Filter::Shadow(_))) => "surface.push_shadow_layer",
            Command::Layer(_, None) => "surface.push_layer",
            Command::PopLayer => "surface.pop_layer",
            Command::Draw(DrawOp::ShadowedRect { .. }) => "surface.draw_shadowed_rect",
            Command::Draw(_) => "surface.draw",
            Command::RichText(_, _, _, _) => "surface.draw_rich_text",
        });
        match command {
            Command::Clip(rect) => surface.push_clip(
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
            ),
            Command::Transform(m) => surface.push_transform(&affine(&m)),
            Command::Pop => surface.pop(),
            Command::Layer(bounds, filter) => {
                let (kind, values) = match filter {
                    None => (0, Vec::new()),
                    Some(Filter::Blur(sigma)) => (1, vec![sigma]),
                    Some(Filter::Shadow(shadow)) => (2, shadow_values(&shadow)),
                };
                #[cfg(all(test, target_arch = "wasm32"))]
                self.trace_layer("surface", layer_kind(kind), bounds);
                surface.push_layer(&rect_values(bounds), kind, &values);
            }
            Command::PopLayer => surface.pop(),
            Command::Draw(op) => self.replay_draw(surface, op),
            Command::RichText(run, spans, bounds, growth) => {
                draw_text_runs(surface, &run, &spans, bounds, growth);
            }
        }
    }

    fn replay_draw(&mut self, surface: &CkSurface, op: DrawOp) {
        match op {
            DrawOp::Rect { rect, paint } => draw_rect(surface, rect, BorderRadii::zero(), paint),
            DrawOp::RoundedRect { rect, radii, paint } => draw_rect(surface, rect, radii, paint),
            DrawOp::Path { commands, paint } => {
                let (fill, stroke, width) = paint_values(&paint);
                surface.draw_path(&path_values(&commands), fill, stroke, width, paint.opacity);
            }
            DrawOp::Image {
                source,
                dst,
                opacity,
            } => {
                if let Some(image) = self.image_for(&source) {
                    surface.draw_image(&image, &rect_values(dst), opacity);
                } else {
                    draw_rect(
                        surface,
                        dst,
                        BorderRadii::zero(),
                        Paint {
                            fill: Some(Color::rgb(128, 128, 128)),
                            stroke: None,
                            opacity,
                        },
                    );
                }
            }
            DrawOp::Text(run) => {
                let align = match run.align {
                    TextAlign::Start => 0,
                    TextAlign::Center => 1,
                    TextAlign::End => 2,
                };
                surface.draw_text(
                    &run.content,
                    &run.font_family,
                    &[run.origin.x, run.origin.y, run.max_width, 0.0],
                    run.font_size,
                    run.font_weight,
                    run.color.0,
                    align,
                    run.line_height,
                );
            }
            DrawOp::LinearGradientRect {
                rect,
                radii,
                gradient,
                stroke,
            } => {
                let (stroke_color, stroke_width) = stroke_values(stroke.as_ref());
                surface.draw_linear_gradient(
                    &rect_values(rect),
                    &radii_values(radii),
                    gradient.angle_deg,
                    &gradient_values(&gradient.stops),
                    gradient.opacity,
                    stroke_color,
                    stroke_width,
                );
            }
            DrawOp::RadialGradientRect {
                rect,
                radii,
                gradient,
                stroke,
            } => {
                let (stroke_color, stroke_width) = stroke_values(stroke.as_ref());
                surface.draw_radial_gradient(
                    &rect_values(rect),
                    &radii_values(radii),
                    gradient.cx,
                    gradient.cy,
                    gradient.radius,
                    &gradient_values(&gradient.stops),
                    gradient.opacity,
                    stroke_color,
                    stroke_width,
                );
            }
            DrawOp::MeshGradientRect {
                rect,
                radii,
                gradient,
                stroke,
            } => {
                let (stroke_color, stroke_width) = stroke_values(stroke.as_ref());
                let colors: Vec<u32> = gradient.colors.iter().map(|color| color.0).collect();
                surface.draw_mesh_gradient(
                    &rect_values(rect),
                    &radii_values(radii),
                    gradient.rows,
                    gradient.cols,
                    &colors,
                    gradient.opacity,
                    stroke_color,
                    stroke_width,
                );
            }
            DrawOp::ShaderRect {
                rect,
                radii,
                shader,
                stroke,
            } => {
                let (stroke_color, stroke_width) = stroke_values(stroke.as_ref());
                let names = Array::new();
                let mut uniforms = Vec::new();
                let mut arities = Vec::new();
                for uniform in &shader.uniforms {
                    names.push(&JsValue::from_str(&uniform.name));
                    arities.push(uniform.values.len() as u32);
                    uniforms.extend_from_slice(&uniform.values);
                }
                surface.draw_shader(
                    &rect_values(rect),
                    &radii_values(radii),
                    &shader.sksl,
                    &names,
                    &uniforms,
                    &arities,
                    shader.opacity,
                    shader.fallback.0,
                    stroke_color,
                    stroke_width,
                );
            }
            DrawOp::ShadowedRect {
                rect,
                radii,
                shadow,
            } => {
                surface.draw_shadow(
                    &rect_values(rect),
                    &radii_values(radii),
                    &shadow_values(&shadow),
                );
            }
            DrawOp::Icon {
                rect,
                name,
                family,
                color,
            } => {
                surface.draw_icon(
                    &rect_values(rect),
                    &name,
                    family.as_deref().unwrap_or(""),
                    color.0,
                );
            }
        }
    }
}

fn clone_image(image: &CkImage) -> CkImage {
    let value: &JsValue = image.as_ref();
    value.clone().unchecked_into()
}

impl Drop for CanvasKitBackend {
    fn drop(&mut self) {
        self.invalidate_images();
        self.runtime.dispose_runtime();
    }
}

impl RenderBackend for CanvasKitBackend {
    type Surface = CanvasKitSurface;

    fn new_surface(&mut self, size: Size) -> Self::Surface {
        self.try_new_surface(size).unwrap_or_else(|error| {
            panic!("CanvasKit failed to create a canvas surface: {error:?}")
        })
    }

    fn begin_frame(&mut self, _surface: &mut Self::Surface, clear: u32) {
        #[cfg(all(test, target_arch = "wasm32"))]
        self.frame_trace.clear();
        #[cfg(all(test, target_arch = "wasm32"))]
        self.frame_layer_trace.clear();
        self.trace("backend.begin_frame");
        self.commands.clear();
        self.pending_filter = None;
        self.clear = clear;
    }

    fn end_frame(&mut self, surface: &mut Self::Surface) {
        self.trace("surface.begin_frame");
        surface.inner.begin_frame(self.clear, self.dpr);
        for command in std::mem::take(&mut self.commands) {
            self.replay(&surface.inner, command);
        }
        surface.inner.end_frame();
        self.trace("surface.end_frame");
        #[cfg(all(test, target_arch = "wasm32"))]
        {
            self.last_frame_trace = std::mem::take(&mut self.frame_trace);
            self.last_frame_layer_trace = std::mem::take(&mut self.frame_layer_trace);
        }
    }

    fn push_clip(&mut self, rect: Rect) {
        self.trace("backend.push_clip");
        self.commands.push(Command::Clip(rect));
    }
    fn push_transform(&mut self, m: &Affine2) {
        self.trace("backend.push_transform");
        self.commands.push(Command::Transform(*m));
    }
    fn pop(&mut self) {
        self.trace("backend.pop");
        self.commands.push(Command::Pop);
    }
    fn push_layer(&mut self, bounds: Rect) {
        self.trace("backend.push_layer");
        #[cfg(all(test, target_arch = "wasm32"))]
        self.trace_layer(
            "backend",
            match self.pending_filter.as_ref() {
                Some(Filter::Blur(_)) => "blur",
                Some(Filter::Shadow(_)) => "shadow",
                None => "plain",
            },
            bounds,
        );
        self.commands
            .push(Command::Layer(bounds, self.pending_filter.take()));
    }
    fn pop_layer(&mut self) {
        self.trace("backend.pop_layer");
        self.commands.push(Command::PopLayer);
    }
    fn apply_blur(&mut self, sigma: f32) {
        self.trace("backend.apply_blur");
        self.pending_filter = Some(Filter::Blur(sigma));
    }
    fn apply_shadow(&mut self, shadow: &ShadowSpec) {
        self.trace("backend.apply_shadow");
        self.pending_filter = Some(Filter::Shadow(shadow.clone()));
    }
    fn draw(&mut self, op: &DrawOp) {
        self.trace("backend.draw");
        self.commands.push(Command::Draw(op.clone()));
    }

    fn draw_text_runs(&mut self, run: &TextRun, spans: &[TextSpan]) {
        self.trace("backend.draw_text_runs");
        let growth = if run.max_width > 0.0 {
            RichTextGrowth::FixedWidth
        } else {
            RichTextGrowth::Auto
        };
        self.commands.push(Command::RichText(
            run.clone(),
            spans.to_vec(),
            Rect::new(run.origin, Size::new(run.max_width.max(0.0), 0.0)),
            growth,
        ));
    }

    fn register_image(&mut self, key: &str, bytes: &[u8]) -> Result<(), DecodeError> {
        probe_image_bounds(bytes)?;
        let image = self
            .runtime
            .decode_image(bytes)
            .map_err(|error| DecodeError(format!("CanvasKit image decode failed: {error:?}")))?;
        if let Some(old) = self.images.insert(key.to_owned(), image) {
            self.runtime.delete_image(&old);
        }
        Ok(())
    }

    fn release_image(&mut self, key: &str) {
        if let Some(image) = self.images.remove(key) {
            self.runtime.delete_image(&image);
        }
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
fn layer_kind(kind: u8) -> &'static str {
    match kind {
        1 => "blur",
        2 => "shadow",
        _ => "plain",
    }
}

fn draw_text_runs(
    surface: &CkSurface,
    run: &TextRun,
    spans: &[TextSpan],
    bounds: Rect,
    growth: RichTextGrowth,
) {
    let texts = Array::new();
    let families = Array::new();
    let mut sizes = Vec::with_capacity(spans.len());
    let mut weights = Vec::with_capacity(spans.len());
    let mut italics = Vec::with_capacity(spans.len());
    let mut spacing = Vec::with_capacity(spans.len());
    let mut colors = Vec::with_capacity(spans.len());
    for span in spans {
        texts.push(&JsValue::from_str(&span.content));
        families.push(&JsValue::from_str(&span.font_family));
        sizes.push(span.font_size);
        weights.push(span.font_weight);
        italics.push(u8::from(span.italic));
        spacing.push(span.letter_spacing);
        colors.push(span.color.0);
    }
    let align = match run.align {
        TextAlign::Start => 0,
        TextAlign::Center => 1,
        TextAlign::End => 2,
    };
    surface.draw_rich_text(
        &texts,
        &families,
        &sizes,
        &weights,
        &italics,
        &spacing,
        &colors,
        &rect_values(bounds),
        match growth {
            RichTextGrowth::Legacy | RichTextGrowth::FixedWidth => 1,
            RichTextGrowth::Auto => 0,
            RichTextGrowth::FixedWidthHeight => 2,
        },
        align,
        run.line_height,
    );
}

fn rect_values(rect: Rect) -> [f32; 4] {
    [
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    ]
}
fn radii_values(radii: BorderRadii) -> [f32; 4] {
    [radii.tl, radii.tr, radii.br, radii.bl]
}
fn affine(m: &Affine2) -> [f32; 6] {
    [m.m11, m.m12, m.m21, m.m22, m.m31, m.m32]
}
fn color_value(color: Option<Color>) -> f64 {
    color.map_or(-1.0, |color| f64::from(color.0))
}
fn stroke_values(stroke: Option<&StrokeOp>) -> (f64, f32) {
    stroke.map_or((-1.0, 0.0), |stroke| {
        (f64::from(stroke.color.0), stroke.width)
    })
}
fn paint_values(paint: &Paint) -> (f64, f64, f32) {
    let (stroke, width) = stroke_values(paint.stroke.as_ref());
    (color_value(paint.fill), stroke, width)
}
fn gradient_values(stops: &[GradientStop]) -> Vec<f32> {
    let mut values = Vec::with_capacity(stops.len() * 5);
    for stop in stops {
        values.extend([
            stop.offset,
            f32::from(stop.color.r()) / 255.0,
            f32::from(stop.color.g()) / 255.0,
            f32::from(stop.color.b()) / 255.0,
            f32::from(stop.color.a()) / 255.0,
        ]);
    }
    values
}
fn shadow_values(shadow: &ShadowSpec) -> Vec<f32> {
    vec![
        shadow.dx,
        shadow.dy,
        shadow.blur,
        shadow.spread,
        f32::from(shadow.color.r()) / 255.0,
        f32::from(shadow.color.g()) / 255.0,
        f32::from(shadow.color.b()) / 255.0,
        f32::from(shadow.color.a()) / 255.0,
    ]
}
fn path_values(commands: &[PathCommand]) -> Vec<f32> {
    let mut values = Vec::new();
    for command in commands {
        match command {
            PathCommand::MoveTo(p) => values.extend([0.0, p.x, p.y]),
            PathCommand::LineTo(p) => values.extend([1.0, p.x, p.y]),
            PathCommand::QuadTo(p1, p2) => values.extend([2.0, p1.x, p1.y, p2.x, p2.y]),
            PathCommand::CubicTo(p1, p2, p3) => {
                values.extend([3.0, p1.x, p1.y, p2.x, p2.y, p3.x, p3.y])
            }
            PathCommand::Close => values.push(4.0),
        }
    }
    values
}

fn draw_rect(surface: &CkSurface, rect: Rect, radii: BorderRadii, paint: Paint) {
    let (fill, stroke, width) = paint_values(&paint);
    if radii == BorderRadii::zero() {
        surface.draw_rect(&rect_values(rect), fill, stroke, width, paint.opacity);
    } else {
        surface.draw_rounded_rect(
            &rect_values(rect),
            &radii_values(radii),
            fill,
            stroke,
            width,
            paint.opacity,
        );
    }
}
