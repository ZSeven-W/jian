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
    /// Optional leading icon (lucide path(s)) — e.g. a globe trigger. `None` for
    /// a plain text dropdown; with an empty `label` it becomes an icon+chevron
    /// dropdown button (TopBar globe / file-menu style).
    pub icon_paths: Option<&'a [&'a str]>,
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
            icon_paths: None,
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
        let mut text_x = rect.origin.x + PAD_X;

        // Optional leading icon (e.g. a globe / file-menu trigger).
        if let Some(paths) = self.icon_paths {
            let isize = font_size + 1.0;
            let mut ic = t.foreground;
            if !self.enabled {
                ic = ic.with_alpha(0.5);
            }
            let iy = rect.origin.y + (rect.size.y - isize) / 2.0;
            for d in paths {
                p.stroke_svg_path(d, Point2D::new(text_x, iy), isize, ic, 1.5);
            }
            text_x += isize + 6.0;
        }

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
            let clip_right = rect.origin.x + rect.size.x - PAD_X - chevron - 4.0;
            let clip = Rect::xywh(
                text_x,
                rect.origin.y,
                (clip_right - text_x).max(0.0),
                rect.size.y,
            );
            p.save();
            p.clip_rect(clip);
            let origin = Point2D::new(text_x, rect.origin.y + (rect.size.y - font_size) / 2.0);
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
            icon_paths: None,
            label,
            placeholder: "Select…",
            hovered: false,
            pressed: false,
            enabled: true,
            font_size: 12.0,
        }
    }

    #[test]
    fn icon_trigger_strokes_leading_icon_and_chevron() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        let globe: &[&str] = &["M2 12h20"];
        SelectTrigger {
            icon_paths: Some(globe),
            label: "",
            ..trigger("")
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 44.0, 28.0), &t);

        // leading icon + chevron => two distinct stroked paths.
        assert!(p
            .ops
            .iter()
            .any(|op| matches!(op, PaintOp::StrokeSvgPath { d, .. } if *d == "M2 12h20")));
        assert!(p
            .ops
            .iter()
            .any(|op| matches!(op, PaintOp::StrokeSvgPath { d, .. } if *d == CHEVRON_D)));
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
