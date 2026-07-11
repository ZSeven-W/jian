//! A small, painter-neutral primitive for positioning one non-italic line of
//! text. Text that exceeds its box is clipped when painted.

use crate::{Color, Painter, Point2D, Rect, TextLayout};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HorizontalAlign {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

#[derive(Debug, Clone, Copy)]
pub struct TextBox<'a> {
    content: &'a str,
    font_family: &'a str,
    font_size: f32,
    font_weight: u16,
    color: Color,
    horizontal_align: HorizontalAlign,
    vertical_align: VerticalAlign,
}

impl<'a> TextBox<'a> {
    pub const fn new(content: &'a str) -> Self {
        Self {
            content,
            font_family: "",
            font_size: 14.0,
            font_weight: 400,
            color: Color::BLACK,
            horizontal_align: HorizontalAlign::Start,
            vertical_align: VerticalAlign::Top,
        }
    }

    pub const fn with_font_family(mut self, font_family: &'a str) -> Self {
        self.font_family = font_family;
        self
    }

    pub const fn with_font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    pub const fn with_font_weight(mut self, font_weight: u16) -> Self {
        self.font_weight = font_weight;
        self
    }

    pub const fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub const fn with_horizontal_align(mut self, align: HorizontalAlign) -> Self {
        self.horizontal_align = align;
        self
    }

    pub const fn with_vertical_align(mut self, align: VerticalAlign) -> Self {
        self.vertical_align = align;
        self
    }

    /// Returns the top-left text origin within `rect`.
    pub fn origin(&self, painter: &mut dyn Painter, rect: Rect) -> Point2D {
        let rect_x = finite_or_zero(rect.origin.x);
        let rect_y = finite_or_zero(rect.origin.y);
        let rect_width = finite_non_negative(rect.size.x);
        let rect_height = finite_non_negative(rect.size.y);
        let font_size = finite_non_negative(self.font_size);
        let measured_width = painter.measure_text_family_styled(
            self.content,
            font_size,
            self.font_family,
            self.font_weight,
            false,
        );
        let measured_width = if measured_width.is_finite() {
            measured_width.max(0.0)
        } else {
            rect_width
        };

        let x = rect_x
            + alignment_offset(
                rect_width - measured_width,
                match self.horizontal_align {
                    HorizontalAlign::Start => 0.0,
                    HorizontalAlign::Center => 0.5,
                    HorizontalAlign::End => 1.0,
                },
            );
        let y = rect_y
            + alignment_offset(
                rect_height - font_size,
                match self.vertical_align {
                    VerticalAlign::Top => 0.0,
                    VerticalAlign::Center => 0.5,
                    VerticalAlign::Bottom => 1.0,
                },
            );

        Point2D::new(finite_or(x, rect_x), finite_or(y, rect_y))
    }

    /// Paints one styled run using a top-left origin derived from `rect`.
    pub fn paint(&self, painter: &mut dyn Painter, rect: Rect) {
        if !valid_paint_rect(rect) || !self.font_size.is_finite() || self.font_size <= 0.0 {
            return;
        }

        let origin = self.origin(painter, rect);
        let layout = TextLayout::single_run(
            self.content,
            self.font_family,
            finite_non_negative(self.font_size),
            self.color.to_jian(),
            Point2D::ZERO,
        )
        .with_font_weight(self.font_weight);
        painter.save();
        painter.clip_rect(rect);
        painter.draw_text(&layout, origin);
        painter.restore();
    }
}

fn alignment_offset(remaining: f32, factor: f32) -> f32 {
    finite_or_zero(remaining) * factor
}

fn valid_paint_rect(rect: Rect) -> bool {
    rect.origin.is_finite() && rect.size.is_finite() && rect.size.x > 0.0 && rect.size.y > 0.0
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn finite_or_zero(value: f32) -> f32 {
    finite_or(value, 0.0)
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Debug, Clone, PartialEq)]
    struct StyleCall {
        content: String,
        font_family: String,
        font_size: f32,
        font_weight: u16,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct DrawCall {
        origin: Point2D,
        style: StyleCall,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum PaintEvent {
        Save,
        Clip(Rect),
        Draw(Point2D),
        Restore,
    }

    #[derive(Default)]
    struct ProbePainter {
        measured_width: f32,
        measurement: Option<StyleCall>,
        draw: Option<DrawCall>,
        events: Vec<PaintEvent>,
    }

    impl ProbePainter {
        fn with_measured_width(measured_width: f32) -> Self {
            Self {
                measured_width,
                ..Self::default()
            }
        }
    }

    impl Painter for ProbePainter {
        fn begin_frame(&mut self) {}

        fn end_frame(&mut self) {}

        fn fill_rect(&mut self, _rect: Rect, _color: Color) {}

        fn stroke_rect(&mut self, _rect: Rect, _color: Color, _width: f32) {}

        fn draw_text(&mut self, layout: &TextLayout, origin: Point2D) {
            let run = layout.runs().first().expect("a text box has one run");
            self.events.push(PaintEvent::Draw(origin));
            self.draw = Some(DrawCall {
                origin,
                style: StyleCall {
                    content: run.content.clone(),
                    font_family: run.font_family.clone(),
                    font_size: run.font_size,
                    font_weight: run.font_weight,
                },
            });
        }

        fn clip_rect(&mut self, rect: Rect) {
            self.events.push(PaintEvent::Clip(rect));
        }

        fn stroke_line(&mut self, _from: Point2D, _to: Point2D, _color: Color, _width: f32) {}

        fn fill_round_rect(&mut self, _rect: Rect, _radius: f32, _color: Color) {}

        fn stroke_round_rect(&mut self, _rect: Rect, _radius: f32, _color: Color, _width: f32) {}

        fn stroke_svg_path(
            &mut self,
            _d: &str,
            _top_left: Point2D,
            _size: f32,
            _color: Color,
            _width: f32,
        ) {
        }

        fn save(&mut self) {
            self.events.push(PaintEvent::Save);
        }

        fn restore(&mut self) {
            self.events.push(PaintEvent::Restore);
        }

        fn translate(&mut self, _offset: Point2D) {}

        fn resize(&mut self, _width: u32, _height: u32) {}

        fn dpi_scale(&self) -> f32 {
            1.0
        }

        fn measure_text_family_styled(
            &mut self,
            text: &str,
            font_size: f32,
            family: &str,
            weight: u16,
            italic: bool,
        ) -> f32 {
            self.measurement = Some(StyleCall {
                content: text.to_owned(),
                font_family: family.to_owned(),
                font_size,
                font_weight: weight,
            });
            let _ = italic;
            self.measured_width
        }
    }

    #[test]
    fn center_vertical_alignment_returns_top_left_origin() {
        let mut painter = ProbePainter::with_measured_width(24.0);
        let rect = Rect::xywh(10.0, 20.0, 100.0, 60.0);
        let origin = TextBox::new("centered")
            .with_font_size(20.0)
            .with_vertical_align(VerticalAlign::Center)
            .origin(&mut painter, rect);

        assert_eq!(origin, Point2D::new(10.0, 40.0));
    }

    #[test]
    fn center_horizontal_alignment_uses_family_aware_width() {
        let mut painter = ProbePainter::with_measured_width(36.0);
        let rect = Rect::xywh(10.0, 20.0, 100.0, 60.0);
        let text_box = TextBox::new("family-aware")
            .with_font_family("Probe Sans")
            .with_font_size(16.0)
            .with_font_weight(650)
            .with_color(Color::BLUE)
            .with_horizontal_align(HorizontalAlign::Center);

        text_box.paint(&mut painter, rect);

        let expected_style = StyleCall {
            content: "family-aware".to_owned(),
            font_family: "Probe Sans".to_owned(),
            font_size: 16.0,
            font_weight: 650,
        };
        assert_eq!(painter.measurement, Some(expected_style.clone()));
        assert_eq!(
            painter.draw,
            Some(DrawCall {
                origin: Point2D::new(42.0, 20.0),
                style: expected_style,
            })
        );
        assert_eq!(
            painter.events,
            vec![
                PaintEvent::Save,
                PaintEvent::Clip(rect),
                PaintEvent::Draw(Point2D::new(42.0, 20.0)),
                PaintEvent::Restore,
            ]
        );
    }

    #[test]
    fn overwide_text_preserves_start_center_and_end_offsets() {
        let rect = Rect::xywh(10.0, 20.0, 100.0, 24.0);
        let text_box = TextBox::new("overwide").with_font_size(12.0);
        let mut start_painter = ProbePainter::with_measured_width(130.0);
        let start = text_box.origin(&mut start_painter, rect);
        let mut center_painter = ProbePainter::with_measured_width(130.0);
        let center = text_box
            .with_horizontal_align(HorizontalAlign::Center)
            .origin(&mut center_painter, rect);
        let mut end_painter = ProbePainter::with_measured_width(130.0);
        let end = text_box
            .with_horizontal_align(HorizontalAlign::End)
            .origin(&mut end_painter, rect);

        assert_eq!(start.x, 10.0);
        assert_eq!(center.x, -5.0);
        assert_eq!(end.x, -20.0);
    }

    #[test]
    fn tiny_rect_never_returns_non_finite_origin() {
        let text_box = TextBox::new("too wide")
            .with_font_size(40.0)
            .with_horizontal_align(HorizontalAlign::Center)
            .with_vertical_align(VerticalAlign::Center);
        let mut painter = ProbePainter::with_measured_width(f32::INFINITY);
        let tiny = Rect::xywh(5.0, 7.0, 1.0, 2.0);

        let origin = text_box.origin(&mut painter, tiny);

        assert!(origin.is_finite());

        text_box.paint(&mut painter, tiny);
        let draw_origin = painter
            .draw
            .as_ref()
            .map(|draw| draw.origin)
            .expect("a valid tiny rect still paints its visible text portion");
        assert!(draw_origin.is_finite());
        assert_eq!(
            painter.events,
            vec![
                PaintEvent::Save,
                PaintEvent::Clip(tiny),
                PaintEvent::Draw(draw_origin),
                PaintEvent::Restore,
            ]
        );

        let invalid = Rect::xywh(f32::NAN, f32::INFINITY, -1.0, f32::NAN);
        let invalid_origin = text_box.origin(&mut painter, invalid);
        assert!(invalid_origin.is_finite());

        let mut invalid_painter = ProbePainter::with_measured_width(24.0);
        TextBox::new("invalid").paint(&mut invalid_painter, Rect::xywh(f32::NAN, 0.0, 20.0, 20.0));
        assert!(invalid_painter.draw.is_none());
        assert!(invalid_painter.events.is_empty());

        let mut empty_painter = ProbePainter::with_measured_width(24.0);
        TextBox::new("empty").paint(&mut empty_painter, Rect::ZERO);
        assert!(empty_painter.draw.is_none());
        assert!(empty_painter.events.is_empty());

        let mut nonpositive_painter = ProbePainter::with_measured_width(24.0);
        TextBox::new("no size")
            .with_font_size(0.0)
            .paint(&mut nonpositive_painter, Rect::xywh(0.0, 0.0, 20.0, 20.0));
        assert!(nonpositive_painter.draw.is_none());
        assert!(nonpositive_painter.events.is_empty());
    }
}
