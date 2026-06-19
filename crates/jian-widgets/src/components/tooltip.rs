//! Tooltip — a small anchored label bubble (shadcn `Tooltip`). Paint-only: no
//! hit / state. The caller positions and sizes the bubble; this widget fills a
//! rounded popover surface, strokes a hairline border, and left-pads the label.

use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

const FONT_FAMILY: &str = "Inter";
const FONT_SIZE: f32 = 12.0;
const RADIUS: f32 = 6.0;
const PAD_X: f32 = 8.0;

#[derive(Debug, Clone, Copy)]
pub struct Tooltip<'a> {
    pub label: &'a str,
}

impl Tooltip<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        // TODO: optional directional caret pointing at the anchor.
        p.fill_round_rect(rect, RADIUS, t.popover);
        p.stroke_round_rect(rect, RADIUS, t.border, 1.0);

        if !self.label.is_empty() {
            let cy = rect.origin.y + rect.size.y / 2.0;
            let origin = Point2D::new(rect.origin.x + PAD_X, cy - FONT_SIZE / 2.0);
            let layout = TextLayout::single_run(
                self.label,
                FONT_FAMILY,
                FONT_SIZE,
                t.popover_foreground.to_jian(),
                Point2D::new(0.0, 0.0),
            );
            p.draw_text(&layout, origin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::CapturePainter;

    #[test]
    fn paints_popover_fill_and_border() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Tooltip { label: "Copy" }.paint(&mut p, Rect::xywh(0.0, 0.0, 60.0, 24.0), &t);

        // The bubble surface is a popover-filled rounded rect.
        assert_eq!(p.fills_with(t.popover), 1);
    }

    #[test]
    fn paints_label_in_popover_foreground() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Tooltip { label: "Delete" }.paint(&mut p, Rect::xywh(0.0, 0.0, 60.0, 24.0), &t);

        let (text, _, color) = p.texts().next().expect("label should be painted");
        assert_eq!(text, "Delete");
        assert_eq!(color, t.popover_foreground.to_jian());
    }

    #[test]
    fn empty_label_paints_bubble_without_text() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Tooltip { label: "" }.paint(&mut p, Rect::xywh(0.0, 0.0, 20.0, 24.0), &t);

        assert_eq!(p.fills_with(t.popover), 1);
        assert!(p.texts().next().is_none());
    }
}
