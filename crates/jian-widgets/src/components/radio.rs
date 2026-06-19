//! Radio — shadcn radio button: a circle with a border; when selected the ring
//! turns primary and a filled primary dot fills the centre. Paint + pure hit;
//! the host owns the selection dispatch. (Group semantics stay host-side.)
//!
//! The ring is drawn as a max-radius round rect so it reads as a circle for a
//! square `rect` and records cleanly through the immediate-mode `Painter`.

use crate::{Painter, Point2D, Rect, Tokens};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Radio {
    pub selected: bool,
    pub enabled: bool,
}

impl Radio {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let radius = rect.size.x.min(rect.size.y) / 2.0;
        let mut ring = if self.selected { t.primary } else { t.input };
        if !self.enabled {
            ring = ring.with_alpha(0.5);
        }
        p.stroke_round_rect(rect, radius, ring, 1.0);

        if self.selected {
            let inset = rect.size.x * 0.3;
            let mut dot = t.primary;
            if !self.enabled {
                dot = dot.with_alpha(0.5);
            }
            p.fill_oval(
                Rect::xywh(
                    rect.origin.x + inset,
                    rect.origin.y + inset,
                    rect.size.x - inset * 2.0,
                    rect.size.y - inset * 2.0,
                ),
                dot,
            );
        }
    }

    /// Whether `point` falls within the radio's density-expanded hit target.
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
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn selected_strokes_primary_ring_and_fills_primary_dot() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Radio {
            selected: true,
            enabled: true,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 16.0, 16.0), &t);

        assert!(p
            .ops
            .iter()
            .any(|op| matches!(op, PaintOp::FillOval(_, c) if *c == t.primary))); // inner dot
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, color, _) if *color == t.primary
        )));
    }

    #[test]
    fn unselected_strokes_input_ring_without_dot() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Radio {
            selected: false,
            enabled: true,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 16.0, 16.0), &t);

        assert_eq!(p.fills_with(t.primary), 0);
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, color, _) if *color == t.input
        )));
    }
}
