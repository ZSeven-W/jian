//! Card — shadcn card chrome: a rounded `card` surface with a 1px border. The
//! body content stays caller-side; this paints only the chrome so the many ad
//! hoc `fill_round_rect + stroke_round_rect` card backgrounds across panels and
//! settings collapse onto one primitive (radius from the `radius` token).

use crate::{Painter, Rect, Tokens};

#[derive(Debug, Clone, Copy, Default)]
pub struct Card {
    /// Paints a hover wash over the surface (selectable cards under the pointer).
    pub hovered: bool,
    /// Emphasises the border with the primary color (active / selected card).
    pub selected: bool,
}

impl Card {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let radius = t.radius;
        p.fill_round_rect(rect, radius, t.card);
        if self.hovered {
            p.fill_round_rect(rect, radius, t.button_hover);
        }
        let border = if self.selected { t.primary } else { t.border };
        p.stroke_round_rect(rect, radius, border, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn paints_card_surface_and_border() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Card::default().paint(&mut p, Rect::xywh(0.0, 0.0, 200.0, 80.0), &t);

        assert_eq!(p.fills_with(t.card), 1);
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, color, _) if *color == t.border
        )));
    }

    #[test]
    fn selected_uses_primary_border() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Card {
            hovered: false,
            selected: true,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 200.0, 80.0), &t);

        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, color, _) if *color == t.primary
        )));
    }

    #[test]
    fn hovered_adds_button_hover_wash() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        Card {
            hovered: true,
            selected: false,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 200.0, 80.0), &t);

        assert_eq!(p.fills_with(t.button_hover), 1);
    }
}
