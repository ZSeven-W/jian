//! Card — a rounded chrome primitive: an optional fill + optional border at a
//! radius. The card's body content AND its state→color logic (hover wash,
//! selected emphasis, disabled dim, outlined variants) stay caller-side — the
//! caller passes the already-resolved colors — so the many ad hoc
//! `fill_round_rect + stroke_round_rect` card backgrounds across panels and
//! settings collapse onto one primitive without forcing a single opinionated
//! look.

use crate::{Color, Painter, Rect, Tokens};

#[derive(Debug, Clone, Copy, Default)]
pub struct Card {
    /// Surface fill; `None` = transparent (border-only / unfilled card).
    pub fill: Option<Color>,
    /// Border stroke; `None` = no border.
    pub border: Option<Color>,
    /// Corner radius in px; `<= 0` falls back to the `radius` token.
    pub radius: f32,
}

impl Card {
    /// The default opaque card — `card` surface + `border` outline at the
    /// `radius` token. Callers layer hover / selected state on top by overriding
    /// `fill` / `border`.
    pub fn surface(t: &Tokens) -> Self {
        Self {
            fill: Some(t.card),
            border: Some(t.border),
            radius: 0.0,
        }
    }

    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let radius = if self.radius > 0.0 {
            self.radius
        } else {
            t.radius
        };
        if let Some(fill) = self.fill {
            p.fill_round_rect(rect, radius, fill);
        }
        if let Some(border) = self.border {
            p.stroke_round_rect(rect, radius, border, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn surface_fills_card_and_strokes_border() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Card::surface(&t).paint(&mut p, Rect::xywh(0.0, 0.0, 200.0, 80.0), &t);

        assert_eq!(p.fills_with(t.card), 1);
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, color, _) if *color == t.border
        )));
    }

    #[test]
    fn caller_picks_fill_and_border_colors_and_radius() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        // e.g. a hovered + selected card: accent wash + primary border at r=10.
        Card {
            fill: Some(t.accent),
            border: Some(t.primary),
            radius: 10.0,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 200.0, 80.0), &t);

        assert_eq!(p.fills_with(t.accent), 1);
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, r, color, _)
                if *color == t.primary && (*r - 10.0).abs() < 0.01
        )));
    }

    #[test]
    fn none_fill_is_transparent_border_only() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Card {
            fill: None,
            border: Some(t.border),
            radius: 0.0,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 200.0, 80.0), &t);

        assert_eq!(p.fills_with(t.card), 0);
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, color, _) if *color == t.border
        )));
    }
}
