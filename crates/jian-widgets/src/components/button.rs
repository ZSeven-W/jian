use crate::{Color, Painter, Point2D, Rect, TextLayout, Tokens};

const RADIUS: f32 = 6.0;
const ICON_LABEL_GAP: f32 = 6.0;
const FONT_FAMILY: &str = "Inter";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    /// Transparent until hovered/pressed: toolbar and icon buttons.
    #[default]
    Ghost,
    Primary,
    Outline,
    Destructive,
    /// Muted neutral fill (shadcn `secondary`) — secondary actions.
    Secondary,
    /// Text-only, primary-colored, no fill/stroke (shadcn `link`).
    Link,
}

#[derive(Debug, Clone, Copy)]
pub struct Button<'a> {
    pub label: &'a str,
    /// Optional lucide d-string (24x24 viewBox) stroked left of label.
    pub icon_d: Option<&'a str>,
    pub variant: ButtonVariant,
    pub enabled: bool,
    pub hovered: bool,
    pub pressed: bool,
    pub font_size: f32,
}

impl Button<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let font_size = if self.font_size > 0.0 {
            self.font_size
        } else {
            t.density.font_size()
        };
        let (fill, stroke, mut text_color) = self.colors(t);

        if let Some(color) = fill {
            p.fill_round_rect(rect, RADIUS, color);
        }
        if let Some(color) = stroke {
            p.stroke_round_rect(rect, RADIUS, color, 1.0);
        }

        if !self.enabled {
            text_color = text_color.with_alpha(0.5);
        }

        let label_width = p.measure_text(self.label, font_size);
        let icon_size = font_size + 3.0;
        let has_icon = self.icon_d.is_some();
        let has_label = !self.label.is_empty();
        let icon_width = if has_icon { icon_size } else { 0.0 };
        let gap = if has_icon && has_label {
            ICON_LABEL_GAP
        } else {
            0.0
        };
        let total_width = icon_width + gap + label_width;
        let mut x = rect.origin.x + (rect.size.x - total_width).max(0.0) / 2.0;

        if let Some(d) = self.icon_d {
            let top_left = Point2D::new(x, rect.origin.y + (rect.size.y - icon_size) / 2.0);
            p.stroke_svg_path(d, top_left, icon_size, text_color, 1.75);
            x += icon_size + gap;
        }

        if has_label {
            let text_origin = Point2D::new(x, rect.origin.y + (rect.size.y - font_size) / 2.0);
            let layout = TextLayout::single_run(
                self.label,
                FONT_FAMILY,
                font_size,
                text_color.to_jian(),
                Point2D::new(0.0, 0.0),
            );
            p.draw_text(&layout, text_origin);
        }
    }

    pub fn hit(rect: Rect, point: Point2D) -> bool {
        rect.contains(point)
    }

    fn colors(&self, t: &Tokens) -> (Option<Color>, Option<Color>, Color) {
        let feedback = if self.enabled {
            if self.pressed {
                Some(t.button_hover.with_alpha(t.button_hover.a * 1.8))
            } else if self.hovered {
                Some(t.button_hover)
            } else {
                None
            }
        } else {
            None
        };

        let (fill, stroke, text) = match self.variant {
            ButtonVariant::Ghost => (feedback, None, t.foreground),
            ButtonVariant::Primary => (Some(t.primary), None, t.primary_foreground),
            ButtonVariant::Outline => (feedback, Some(t.border), t.foreground),
            ButtonVariant::Destructive => (Some(t.destructive), None, t.primary_foreground),
            ButtonVariant::Secondary => (Some(t.secondary), None, t.secondary_foreground),
            ButtonVariant::Link => (None, None, t.primary),
        };

        if self.enabled {
            (fill, stroke, text)
        } else {
            (
                fill.map(|c| c.with_alpha(0.5)),
                stroke.map(|c| c.with_alpha(0.5)),
                text,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn ghost_pressed_paints_touch_feedback() {
        let t = Tokens::dark();
        let b = Button {
            label: "Run",
            icon_d: None,
            variant: ButtonVariant::Ghost,
            enabled: true,
            hovered: true,
            pressed: true,
            font_size: 13.0,
        };
        let mut p = CapturePainter::default();

        b.paint(&mut p, Rect::xywh(0.0, 0.0, 80.0, 30.0), &t);

        assert_eq!(
            p.fills_with(t.button_hover.with_alpha(t.button_hover.a * 1.8)),
            1
        );
    }

    #[test]
    fn primary_disabled_dims_background_and_label() {
        let t = Tokens::dark();
        let b = Button {
            label: "Save",
            icon_d: None,
            variant: ButtonVariant::Primary,
            enabled: false,
            hovered: false,
            pressed: false,
            font_size: 13.0,
        };
        let mut p = CapturePainter::default();

        b.paint(&mut p, Rect::xywh(0.0, 0.0, 90.0, 30.0), &t);

        assert_eq!(p.fills_with(t.primary.with_alpha(0.5)), 1);
        let (_, _, color) = p.texts().next().expect("button label should be painted");
        assert_eq!(color, t.primary_foreground.with_alpha(0.5).to_jian());
    }

    #[test]
    fn disabled_ghost_suppresses_hover_feedback() {
        let t = Tokens::dark();
        let b = Button {
            label: "Save",
            icon_d: None,
            variant: ButtonVariant::Ghost,
            enabled: false,
            hovered: true,
            pressed: true,
            font_size: 13.0,
        };
        let mut p = CapturePainter::default();

        b.paint(&mut p, Rect::xywh(0.0, 0.0, 90.0, 30.0), &t);

        assert_eq!(p.fills_with(t.button_hover), 0);
        assert_eq!(p.fills_with(t.button_hover.with_alpha(0.5)), 0);
    }

    #[test]
    fn icon_and_label_are_centered_as_one_group() {
        let t = Tokens::dark();
        let icon = "M4 12h16";
        let b = Button {
            label: "Open",
            icon_d: Some(icon),
            variant: ButtonVariant::Outline,
            enabled: true,
            hovered: false,
            pressed: false,
            font_size: 13.0,
        };
        let mut p = CapturePainter::default();
        let rect = Rect::xywh(10.0, 20.0, 120.0, 30.0);

        b.paint(&mut p, rect, &t);

        assert!(p.ops.iter().any(|op| {
            matches!(
                op,
                PaintOp::StrokeSvgPath { d, size, .. } if d == icon && *size == 16.0
            )
        }));
        let (_, origin, _) = p.texts().next().expect("label should be painted");
        assert!(origin.x > rect.origin.x + 50.0);
        assert!(origin.x < rect.origin.x + 60.0);
    }

    #[test]
    fn hit_uses_button_rect() {
        let rect = Rect::xywh(10.0, 20.0, 80.0, 30.0);
        assert!(Button::hit(rect, Point2D::new(90.0, 50.0)));
        assert!(!Button::hit(rect, Point2D::new(91.0, 50.0)));
    }

    #[test]
    fn secondary_paints_secondary_fill_and_foreground() {
        let t = Tokens::dark();
        let b = Button {
            label: "Cancel",
            icon_d: None,
            variant: ButtonVariant::Secondary,
            enabled: true,
            hovered: false,
            pressed: false,
            font_size: 13.0,
        };
        let mut p = CapturePainter::default();

        b.paint(&mut p, Rect::xywh(0.0, 0.0, 80.0, 30.0), &t);

        assert_eq!(p.fills_with(t.secondary), 1);
        let (_, _, color) = p.texts().next().expect("label should be painted");
        assert_eq!(color, t.secondary_foreground.to_jian());
    }

    #[test]
    fn link_paints_no_fill_with_primary_text() {
        let t = Tokens::dark();
        let b = Button {
            label: "Learn more",
            icon_d: None,
            variant: ButtonVariant::Link,
            enabled: true,
            hovered: true,
            pressed: false,
            font_size: 13.0,
        };
        let mut p = CapturePainter::default();

        b.paint(&mut p, Rect::xywh(0.0, 0.0, 100.0, 30.0), &t);

        // Link has no fill even when hovered (no feedback wash).
        assert_eq!(p.fills_with(t.secondary), 0);
        assert_eq!(p.fills_with(t.button_hover), 0);
        let (_, _, color) = p.texts().next().expect("label should be painted");
        assert_eq!(color, t.primary.to_jian());
    }
}
