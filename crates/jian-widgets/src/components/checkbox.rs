//! Checkbox — shadcn checkbox: a small rounded square. Unchecked paints a
//! border (the `input` token); checked paints a primary fill + white check
//! glyph. Paint + pure hit; the host owns the toggle dispatch.

use crate::{Painter, Point2D, Rect, Tokens};

/// lucide `check` d-string (24x24 viewBox).
const CHECK_D: &str = "M20 6 9 17l-5-5";
const RADIUS: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkbox {
    pub checked: bool,
    pub enabled: bool,
}

impl Checkbox {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        if self.checked {
            let mut fill = t.primary;
            let mut glyph = t.primary_foreground;
            if !self.enabled {
                fill = fill.with_alpha(0.5);
                glyph = glyph.with_alpha(0.5);
            }
            p.fill_round_rect(rect, RADIUS, fill);
            // Inset the glyph box so the (non-full-viewBox) check reads centred.
            let inset = rect.size.x * 0.15;
            p.stroke_svg_path(
                CHECK_D,
                Point2D::new(rect.origin.x + inset, rect.origin.y + inset),
                rect.size.x - inset * 2.0,
                glyph,
                2.0,
            );
        } else {
            let mut border = t.input;
            if !self.enabled {
                border = border.with_alpha(0.5);
            }
            p.stroke_round_rect(rect, RADIUS, border, 1.0);
        }
    }

    /// Whether `point` falls within the checkbox's density-expanded hit target.
    pub fn hit(rect: Rect, point: Point2D, t: &Tokens) -> bool {
        let min = t.density.min_hit_target();
        let ex = ((min - rect.size.x).max(0.0)) / 2.0;
        let ey = ((min - rect.size.y).max(0.0)) / 2.0;
        Rect::xywh(
            rect.origin.x - ex,
            rect.origin.y - ey,
            rect.size.x + ex * 2.0,
            rect.size.y + ey * 2.0,
        )
        .contains(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_fills_primary_and_strokes_check_glyph() {
        let t = Tokens::dark();
        let mut p = crate::test_support::CapturePainter::default();
        Checkbox {
            checked: true,
            enabled: true,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 16.0, 16.0), &t);

        assert_eq!(p.fills_with(t.primary), 1);
        assert!(p.ops.iter().any(|op| matches!(
            op,
            crate::test_support::PaintOp::StrokeSvgPath { d, .. } if *d == CHECK_D
        )));
    }

    #[test]
    fn unchecked_strokes_input_border_without_fill() {
        let t = Tokens::dark();
        let mut p = crate::test_support::CapturePainter::default();
        Checkbox {
            checked: false,
            enabled: true,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 16.0, 16.0), &t);

        assert_eq!(p.fills_with(t.primary), 0);
        assert!(p.ops.iter().any(|op| matches!(
            op,
            crate::test_support::PaintOp::StrokeRoundRect(_, _, color, _) if *color == t.input
        )));
    }

    #[test]
    fn touch_density_expands_hit_target() {
        let mut t = Tokens::dark();
        t.density = crate::Density::Touch;
        let rect = Rect::xywh(10.0, 10.0, 16.0, 16.0);
        // Just outside the 16px box but inside the >=44px touch target.
        assert!(Checkbox::hit(rect, Point2D::new(2.0, 18.0), &t));
        assert!(!Checkbox::hit(rect, Point2D::new(200.0, 18.0), &t));
    }
}
