//! SelectTrigger — the closed-state trigger of a select / dropdown (shadcn
//! `SelectTrigger`): a bordered box showing the current value with a chevron,
//! with its own hover / press feedback. Pairs with [`Select`](super::select)
//! (the open popup list). Self-contained, React-FC style: pass the value +
//! state and call `paint` — the box, feedback, label/placeholder and chevron are
//! all the component's concern; callers never assemble them from pieces.

use crate::components::button::{Button, ButtonVariant};
use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

const FONT_FAMILY: &str = "Inter";
const PAD_X: f32 = 8.0;
/// lucide `chevron-down`.
const CHEVRON_D: &str = "m6 9 6 6 6-6";

pub struct SelectTrigger<'a> {
    /// The selected value; when empty the `placeholder` is shown muted.
    pub label: &'a str,
    pub placeholder: &'a str,
    pub hovered: bool,
    pub pressed: bool,
    pub enabled: bool,
    /// `<= 0` derives from the density font size.
    pub font_size: f32,
}

impl SelectTrigger<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        // Box = an Outline button: input/border outline + hover/press wash, all
        // owned by Button — the trigger never hand-rolls its feedback.
        Button {
            label: "",
            icon_d: None,
            variant: ButtonVariant::Outline,
            enabled: self.enabled,
            hovered: self.hovered,
            pressed: self.pressed,
            font_size: 0.0,
        }
        .paint(p, rect, t);

        let font_size = if self.font_size > 0.0 {
            self.font_size
        } else {
            t.density.font_size()
        };
        let chevron = 14.0;

        // Value / placeholder, clipped so a long value never runs under the
        // chevron.
        let (text, mut color) = if self.label.is_empty() {
            (self.placeholder, t.muted_foreground)
        } else {
            (self.label, t.foreground)
        };
        if !self.enabled {
            color = color.with_alpha(0.5);
        }
        if !text.is_empty() {
            let clip = Rect::xywh(
                rect.origin.x,
                rect.origin.y,
                (rect.size.x - PAD_X - chevron - 4.0).max(0.0),
                rect.size.y,
            );
            p.save();
            p.clip_rect(clip);
            let origin = Point2D::new(
                rect.origin.x + PAD_X,
                rect.origin.y + (rect.size.y - font_size) / 2.0,
            );
            let layout = TextLayout::single_run(
                text,
                FONT_FAMILY,
                font_size,
                color.to_jian(),
                Point2D::new(0.0, 0.0),
            );
            p.draw_text(&layout, origin);
            p.restore();
        }

        // Chevron on the right.
        let mut chev_color = t.muted_foreground;
        if !self.enabled {
            chev_color = chev_color.with_alpha(0.5);
        }
        let chev_origin = Point2D::new(
            rect.origin.x + rect.size.x - PAD_X - chevron,
            rect.origin.y + (rect.size.y - chevron) / 2.0,
        );
        p.stroke_svg_path(CHEVRON_D, chev_origin, chevron, chev_color, 1.5);
    }

    /// Hit-test the trigger box — the component owns this too.
    pub fn hit(rect: Rect, point: Point2D) -> bool {
        rect.contains(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    fn trigger<'a>(label: &'a str) -> SelectTrigger<'a> {
        SelectTrigger {
            label,
            placeholder: "Select…",
            hovered: false,
            pressed: false,
            enabled: true,
            font_size: 12.0,
        }
    }

    #[test]
    fn paints_outline_border_value_and_chevron() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        trigger("Kit A").paint(&mut p, Rect::xywh(0.0, 0.0, 140.0, 28.0), &t);

        // Outline box border.
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, c, _) if *c == t.border
        )));
        // The chevron-down glyph.
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeSvgPath { d, .. } if *d == CHEVRON_D
        )));
        // Value text in foreground.
        let (_, _, color) = p.texts().next().expect("value text");
        assert_eq!(color, t.foreground.to_jian());
    }

    #[test]
    fn empty_value_shows_muted_placeholder() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        trigger("").paint(&mut p, Rect::xywh(0.0, 0.0, 140.0, 28.0), &t);

        let (text, _, color) = p.texts().next().expect("placeholder text");
        assert_eq!(text, "Select…");
        assert_eq!(color, t.muted_foreground.to_jian());
    }

    #[test]
    fn hover_wash_comes_from_the_outline_button() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        SelectTrigger {
            hovered: true,
            ..trigger("Kit A")
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 140.0, 28.0), &t);

        assert_eq!(p.fills_with(t.button_hover), 1);
    }
}
