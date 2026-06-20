//! Input — a single-line text field: shadcn field chrome (surface, border,
//! focus ring, optional leading icon) composed over the glyph-painting
//! `TextInputView`. `TextInputView` paints only text / caret / selection; this
//! adds the box the bare view lacks, so the many hand-rolled "border + bg +
//! focus-stroke then draw glyphs" inputs collapse onto one primitive.

use crate::components::text_input::TextInputView;
use crate::{Painter, Point2D, Rect, Tokens};

const PAD_X: f32 = 8.0;
const ICON_GAP: f32 = 6.0;

pub struct Input<'a> {
    pub state: &'a jian_core::text_input::TextInputState,
    pub placeholder: &'a str,
    pub focused: bool,
    /// `<= 0` falls back to the density font size.
    pub font_size: f32,
    pub now_ms: u64,
    /// Optional leading lucide d-string (24x24 viewBox) stroked at the left.
    pub icon_d: Option<&'a str>,
}

impl Input<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let radius = t.radius;
        // Field surface + border; focus swaps the input border for the ring.
        p.fill_round_rect(rect, radius, t.background);
        let (border, width) = if self.focused {
            (t.ring, 1.5)
        } else {
            (t.input, 1.0)
        };
        p.stroke_round_rect(rect, radius, border, width);

        let font_size = if self.font_size > 0.0 {
            self.font_size
        } else {
            t.density.font_size()
        };

        // Leading icon (optional) shifts the text start so glyphs clear it.
        let mut pad_x = PAD_X;
        if let Some(d) = self.icon_d {
            let icon_size = font_size;
            let cy = rect.origin.y + (rect.size.y - icon_size) / 2.0;
            p.stroke_svg_path(
                d,
                Point2D::new(rect.origin.x + PAD_X, cy),
                icon_size,
                t.muted_foreground,
                1.5,
            );
            pad_x += icon_size + ICON_GAP;
        }

        TextInputView {
            state: self.state,
            placeholder: self.placeholder,
            focused: self.focused,
            font_size: self.font_size,
            now_ms: self.now_ms,
            pad_x,
            baseline_delta_y: 0.0,
        }
        .paint(p, rect, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    fn input<'a>(
        state: &'a jian_core::text_input::TextInputState,
        focused: bool,
        icon_d: Option<&'a str>,
    ) -> Input<'a> {
        Input {
            state,
            placeholder: "",
            focused,
            font_size: 13.0,
            now_ms: 0,
            icon_d,
        }
    }

    #[test]
    fn focused_input_fills_surface_and_strokes_ring() {
        let t = Tokens::dark();
        let state = jian_core::text_input::TextInputState::with_text("hi");
        let mut p = CapturePainter::default();
        input(&state, true, None).paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 28.0), &t);

        assert_eq!(p.fills_with(t.background), 1);
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, c, _) if *c == t.ring
        )));
    }

    #[test]
    fn unfocused_input_strokes_input_border() {
        let t = Tokens::dark();
        let state = jian_core::text_input::TextInputState::with_text("hi");
        let mut p = CapturePainter::default();
        input(&state, false, None).paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 28.0), &t);

        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, c, _) if *c == t.input
        )));
        assert!(!p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, c, _) if *c == t.ring
        )));
    }

    #[test]
    fn leading_icon_is_stroked() {
        let t = Tokens::dark();
        let state = jian_core::text_input::TextInputState::with_text("");
        let mut p = CapturePainter::default();
        input(&state, false, Some("M4 12h16")).paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 28.0), &t);

        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeSvgPath { d, .. } if *d == "M4 12h16"
        )));
    }
}
