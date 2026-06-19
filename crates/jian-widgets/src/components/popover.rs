use crate::{Painter, Point2D, Rect, Tokens};

/// Anchored floating container chrome plus a pure placement helper.
///
/// Generalizes the popup flip/clamp logic used by `Select`: position the
/// popover at the anchor's bottom-left, flip above when it would overflow the
/// viewport bottom, and clamp both axes within the viewport.
#[derive(Debug, Clone, Copy, Default)]
pub struct Popover;

impl Popover {
    /// Paint the popover container chrome (rounded fill + border).
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        p.fill_round_rect(rect, 6.0, t.popover);
        p.stroke_round_rect(rect, 6.0, t.border, 1.0);
    }

    /// Pure placement: anchor the popover at the anchor's bottom-left, flip
    /// above the anchor when the bottom would overflow the viewport, and clamp
    /// the result within the viewport on both axes.
    pub fn placement(anchor: Rect, size: Point2D, viewport: Rect) -> Rect {
        let width = size.x;
        let height = size.y;

        // Horizontal: prefer the anchor's left edge, clamp the right edge.
        let max_x = viewport.origin.x + viewport.size.x - width;
        let x = anchor
            .origin
            .x
            .clamp(viewport.origin.x, max_x.max(viewport.origin.x));

        // Vertical: prefer below the anchor, flip above on overflow, then clamp.
        let below = anchor.origin.y + anchor.size.y;
        let above = anchor.origin.y - height;
        let viewport_bottom = viewport.origin.y + viewport.size.y;
        let y = if below + height <= viewport_bottom {
            below
        } else if above >= viewport.origin.y {
            above
        } else {
            (viewport_bottom - height).max(viewport.origin.y)
        };

        Rect::xywh(x, y, width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn placement_sits_below_anchor_when_room() {
        let anchor = Rect::xywh(20.0, 40.0, 120.0, 24.0);
        let viewport = Rect::xywh(0.0, 0.0, 400.0, 400.0);

        let rect = Popover::placement(anchor, Point2D::new(120.0, 80.0), viewport);

        assert_eq!(rect.origin.x, 20.0);
        assert_eq!(rect.origin.y, anchor.origin.y + anchor.size.y);
        assert_eq!(rect.size.x, 120.0);
        assert_eq!(rect.size.y, 80.0);
    }

    #[test]
    fn placement_flips_above_when_bottom_overflows() {
        let anchor = Rect::xywh(20.0, 360.0, 120.0, 24.0);
        let viewport = Rect::xywh(0.0, 0.0, 400.0, 400.0);

        let rect = Popover::placement(anchor, Point2D::new(120.0, 80.0), viewport);

        // Flipped above: bottom edge no lower than the anchor's top edge.
        assert!(rect.origin.y + rect.size.y <= anchor.origin.y);
        assert!(rect.origin.y >= viewport.origin.y);
    }

    #[test]
    fn placement_clamps_right_edge_within_viewport() {
        let anchor = Rect::xywh(360.0, 40.0, 120.0, 24.0);
        let viewport = Rect::xywh(0.0, 0.0, 400.0, 400.0);

        let rect = Popover::placement(anchor, Point2D::new(120.0, 80.0), viewport);

        assert!(rect.origin.x + rect.size.x <= viewport.origin.x + viewport.size.x);
        assert_eq!(rect.origin.x, 400.0 - 120.0);
    }

    #[test]
    fn paint_draws_filled_and_stroked_container() {
        let t = Tokens::dark();
        let rect = Rect::xywh(10.0, 10.0, 160.0, 90.0);
        let mut p = CapturePainter::default();

        Popover.paint(&mut p, rect, &t);

        assert!(p
            .ops
            .iter()
            .any(|op| matches!(op, PaintOp::FillRoundRect(_, 6.0, _))));
        assert!(p
            .ops
            .iter()
            .any(|op| matches!(op, PaintOp::StrokeRoundRect(_, 6.0, _, 1.0))));
    }
}
