//! AccordionHeader — a collapsible section header row with a chevron (shadcn
//! `AccordionTrigger`). Paint-only: an optional hover wash, a left-padded title
//! and a right-aligned chevron that points right when collapsed and down when
//! expanded. The host owns the expand/collapse toggle dispatch.

use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

/// lucide `chevron-right` d-string (collapsed state).
const CHEVRON_RIGHT_D: &str = "m9 18 6-6-6-6";
/// lucide `chevron-down` d-string (expanded state).
const CHEVRON_DOWN_D: &str = "m6 9 6 6 6-6";

const HOVER_RADIUS: f32 = 4.0;
const FONT_SIZE: f32 = 13.0;
const PAD_X: f32 = 8.0;
const CHEVRON_SIZE: f32 = 16.0;

#[derive(Debug, Clone, Copy)]
pub struct AccordionHeader<'a> {
    pub title: &'a str,
    pub expanded: bool,
    pub hovered: bool,
}

impl AccordionHeader<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        if self.hovered {
            p.fill_round_rect(rect, HOVER_RADIUS, t.button_hover);
        }

        // Title text, vertically centred and left-padded.
        let text_y = crate::centered_text_baseline_y(rect, FONT_SIZE);
        let layout = TextLayout::single_run(
            self.title,
            "Inter",
            FONT_SIZE,
            t.foreground.to_jian(),
            Point2D::new(0.0, 0.0),
        );
        p.draw_text(&layout, Point2D::new(rect.origin.x + PAD_X, text_y));

        // Right-aligned chevron, swapped by expand state.
        let d = if self.expanded {
            CHEVRON_DOWN_D
        } else {
            CHEVRON_RIGHT_D
        };
        let cx = rect.origin.x + rect.size.x - PAD_X - CHEVRON_SIZE;
        let cy = rect.origin.y + (rect.size.y - CHEVRON_SIZE) / 2.0;
        p.stroke_svg_path(
            d,
            Point2D::new(cx, cy),
            CHEVRON_SIZE,
            t.muted_foreground,
            2.0,
        );
    }

    /// Whether `point` falls within the header row.
    pub fn hit(rect: Rect, point: Point2D) -> bool {
        rect.contains(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn paints_title_text() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        AccordionHeader {
            title: "Section",
            expanded: false,
            hovered: false,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 200.0, 32.0), &t);

        assert!(p
            .texts()
            .any(|(text, _, color)| text == "Section" && color == t.foreground.to_jian()));
    }

    #[test]
    fn collapsed_uses_chevron_right() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        AccordionHeader {
            title: "Section",
            expanded: false,
            hovered: false,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 200.0, 32.0), &t);

        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeSvgPath { d, .. } if *d == CHEVRON_RIGHT_D
        )));
    }

    #[test]
    fn expanded_uses_chevron_down() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        AccordionHeader {
            title: "Section",
            expanded: true,
            hovered: false,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 200.0, 32.0), &t);

        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeSvgPath { d, .. } if *d == CHEVRON_DOWN_D
        )));
    }

    #[test]
    fn hovered_paints_wash() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        AccordionHeader {
            title: "Section",
            expanded: false,
            hovered: true,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 200.0, 32.0), &t);

        assert_eq!(p.fills_with(t.button_hover), 1);
    }
}
