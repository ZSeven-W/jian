//! Progress — shadcn determinate progress bar. Paint-only: a rounded `muted`
//! track with a `primary` fill spanning `value` (0..=1) of the width. Both ends
//! are pill-rounded so a partial fill reads cleanly.

use crate::{Painter, Rect, Tokens};

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    /// Completion fraction, clamped to `0.0..=1.0`.
    pub value: f32,
}

impl Progress {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let radius = rect.size.y / 2.0;
        p.fill_round_rect(rect, radius, t.muted);

        let v = self.value.clamp(0.0, 1.0);
        if v > 0.0 {
            let filled = Rect::xywh(rect.origin.x, rect.origin.y, rect.size.x * v, rect.size.y);
            p.fill_round_rect(filled, radius, t.primary);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn paints_muted_track_and_primary_fill_at_value() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Progress { value: 0.5 }.paint(&mut p, Rect::xywh(0.0, 0.0, 100.0, 6.0), &t);

        assert_eq!(p.fills_with(t.muted), 1);
        // Primary fill spans half the width.
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::FillRoundRect(r, _, c) if *c == t.primary && (r.size.x - 50.0).abs() < 0.01
        )));
    }

    #[test]
    fn zero_value_paints_track_only() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Progress { value: 0.0 }.paint(&mut p, Rect::xywh(0.0, 0.0, 100.0, 6.0), &t);

        assert_eq!(p.fills_with(t.muted), 1);
        assert_eq!(p.fills_with(t.primary), 0);
    }

    #[test]
    fn over_unit_value_is_clamped() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Progress { value: 2.5 }.paint(&mut p, Rect::xywh(0.0, 0.0, 100.0, 6.0), &t);

        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::FillRoundRect(r, _, c) if *c == t.primary && (r.size.x - 100.0).abs() < 0.01
        )));
    }
}
