use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

pub const CLOSE_D: &str = "M18 6 6 18M6 6l12 12";
const FONT_FAMILY: &str = "Inter";
const CARD_RADIUS: f32 = 10.0;
const CLOSE_SIZE: f32 = 28.0;

#[derive(Debug, Clone, Copy)]
pub struct Dialog<'a> {
    pub title: &'a str,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogHit {
    Close,
    Inside,
    Scrim,
}

impl Dialog<'_> {
    pub fn paint(&self, p: &mut dyn Painter, viewport: Rect, t: &Tokens) {
        let card = self.card_rect(viewport);
        p.fill_rect(viewport, t.background.with_alpha(0.6));
        p.fill_drop_shadow(
            card,
            CARD_RADIUS,
            24.0,
            crate::Color::BLACK.with_alpha(0.35),
        );
        p.fill_round_rect(card, CARD_RADIUS, t.card);
        p.stroke_round_rect(card, CARD_RADIUS, t.border, 1.0);

        let title_origin = Point2D::new(card.origin.x + 20.0, card.origin.y + 18.0);
        let title = TextLayout::single_run(
            self.title,
            FONT_FAMILY,
            16.0,
            t.card_foreground.to_jian(),
            title_origin,
        )
        .with_font_weight(600);
        p.draw_text(&title, title_origin);

        let close = close_rect(card);
        p.stroke_svg_path(
            CLOSE_D,
            Point2D::new(close.origin.x + 5.0, close.origin.y + 5.0),
            18.0,
            t.muted_foreground,
            1.75,
        );
    }

    pub fn card_rect(&self, viewport: Rect) -> Rect {
        let width = self.width.min(viewport.size.x).max(0.0);
        let height = self.height.min(viewport.size.y).max(0.0);
        Rect::xywh(
            viewport.origin.x + (viewport.size.x - width) / 2.0,
            viewport.origin.y + (viewport.size.y - height) / 2.0,
            width,
            height,
        )
    }

    pub fn hit(&self, viewport: Rect, point: Point2D) -> DialogHit {
        let card = self.card_rect(viewport);
        if close_rect(card).contains(point) {
            return DialogHit::Close;
        }
        if card.contains(point) {
            DialogHit::Inside
        } else {
            DialogHit::Scrim
        }
    }
}

fn close_rect(card: Rect) -> Rect {
    Rect::xywh(
        card.origin.x + card.size.x - CLOSE_SIZE - 8.0,
        card.origin.y + 8.0,
        CLOSE_SIZE,
        CLOSE_SIZE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn card_rect_is_centered_in_viewport() {
        let dialog = Dialog {
            title: "Export",
            width: 200.0,
            height: 100.0,
        };
        let rect = dialog.card_rect(Rect::xywh(0.0, 0.0, 600.0, 400.0));

        assert_eq!(rect, Rect::xywh(200.0, 150.0, 200.0, 100.0));
    }

    #[test]
    fn hit_distinguishes_close_inside_and_scrim() {
        let dialog = Dialog {
            title: "Export",
            width: 200.0,
            height: 100.0,
        };
        let viewport = Rect::xywh(0.0, 0.0, 600.0, 400.0);
        let card = dialog.card_rect(viewport);

        assert_eq!(
            dialog.hit(
                viewport,
                Point2D::new(card.origin.x + card.size.x - 16.0, card.origin.y + 16.0)
            ),
            DialogHit::Close
        );
        assert_eq!(
            dialog.hit(
                viewport,
                Point2D::new(card.origin.x + 20.0, card.origin.y + 50.0)
            ),
            DialogHit::Inside
        );
        assert_eq!(
            dialog.hit(viewport, Point2D::new(5.0, 5.0)),
            DialogHit::Scrim
        );
    }

    #[test]
    fn paint_draws_scrim_shadow_card_title_and_close() {
        let dialog = Dialog {
            title: "Export",
            width: 200.0,
            height: 100.0,
        };
        let t = Tokens::dark();
        let mut p = CapturePainter::default();

        dialog.paint(&mut p, Rect::xywh(0.0, 0.0, 600.0, 400.0), &t);

        assert_eq!(p.fills_with(t.background.with_alpha(0.6)), 1);
        assert!(p
            .ops
            .iter()
            .any(|op| matches!(op, PaintOp::FillDropShadow(_, 10.0, _, _))));
        assert_eq!(p.texts().next().map(|(s, _, _)| s), Some("Export"));
        assert!(p
            .ops
            .iter()
            .any(|op| { matches!(op, PaintOp::StrokeSvgPath { d, .. } if d == CLOSE_D) }));
    }
}
