use crate::{Color, Painter, Point2D, Rect, TextLayout};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PaintOp {
    FillRect(Rect, Color),
    StrokeRect(Rect, Color, f32),
    Text {
        content: String,
        origin: Point2D,
        color: jian_core::scene::Color,
    },
    ClipRect(Rect),
    StrokeLine(Point2D, Point2D, Color, f32),
    FillRoundRect(Rect, f32, Color),
    StrokeRoundRect(Rect, f32, Color, f32),
    StrokeSvgPath {
        d: String,
        top_left: Point2D,
        size: f32,
        color: Color,
        width: f32,
    },
    FillDropShadow(Rect, f32, f32, Color),
    FillOval(Rect, Color),
    Save,
    Restore,
    Translate(Point2D),
    Resize(u32, u32),
}

#[derive(Default)]
pub(crate) struct CapturePainter {
    pub(crate) ops: Vec<PaintOp>,
}

impl CapturePainter {
    pub(crate) fn fills_with(&self, color: Color) -> usize {
        self.ops
            .iter()
            .filter(|op| match op {
                PaintOp::FillRect(_, c) | PaintOp::FillRoundRect(_, _, c) => *c == color,
                _ => false,
            })
            .count()
    }

    pub(crate) fn texts(&self) -> impl Iterator<Item = (&str, Point2D, jian_core::scene::Color)> {
        self.ops.iter().filter_map(|op| match op {
            PaintOp::Text {
                content,
                origin,
                color,
            } => Some((content.as_str(), *origin, *color)),
            _ => None,
        })
    }
}

impl Painter for CapturePainter {
    fn begin_frame(&mut self) {}
    fn end_frame(&mut self) {}

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.ops.push(PaintOp::FillRect(rect, color));
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32) {
        self.ops.push(PaintOp::StrokeRect(rect, color, width));
    }

    fn draw_text(&mut self, layout: &TextLayout, origin: Point2D) {
        if let Some(run) = layout.runs().first() {
            self.ops.push(PaintOp::Text {
                content: run.content.clone(),
                origin,
                color: run.color,
            });
        }
    }

    fn clip_rect(&mut self, rect: Rect) {
        self.ops.push(PaintOp::ClipRect(rect));
    }

    fn stroke_line(&mut self, from: Point2D, to: Point2D, color: Color, width: f32) {
        self.ops.push(PaintOp::StrokeLine(from, to, color, width));
    }

    fn fill_round_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        self.ops.push(PaintOp::FillRoundRect(rect, radius, color));
    }

    fn stroke_round_rect(&mut self, rect: Rect, radius: f32, color: Color, width: f32) {
        self.ops
            .push(PaintOp::StrokeRoundRect(rect, radius, color, width));
    }

    fn stroke_svg_path(&mut self, d: &str, top_left: Point2D, size: f32, color: Color, width: f32) {
        self.ops.push(PaintOp::StrokeSvgPath {
            d: d.to_owned(),
            top_left,
            size,
            color,
            width,
        });
    }

    fn fill_drop_shadow(&mut self, rect: Rect, radius: f32, blur: f32, color: Color) {
        self.ops
            .push(PaintOp::FillDropShadow(rect, radius, blur, color));
    }

    fn fill_oval(&mut self, bounds: Rect, color: Color) {
        self.ops.push(PaintOp::FillOval(bounds, color));
    }

    fn save(&mut self) {
        self.ops.push(PaintOp::Save);
    }

    fn restore(&mut self) {
        self.ops.push(PaintOp::Restore);
    }

    fn translate(&mut self, offset: Point2D) {
        self.ops.push(PaintOp::Translate(offset));
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.ops.push(PaintOp::Resize(width, height));
    }

    fn dpi_scale(&self) -> f32 {
        1.0
    }
}
