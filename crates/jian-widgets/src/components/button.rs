use crate::{Color, HorizontalAlign, Painter, Point2D, Rect, TextBox, Tokens, VerticalAlign};

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
    /// Destructive border + text over a hover wash (shadcn `outline` in a
    /// destructive context) — non-primary destructive actions like
    /// Disconnect / Remove / Delete that shouldn't shout like a solid
    /// `Destructive` CTA.
    DestructiveOutline,
    /// Text-only, primary-colored, no fill/stroke (shadcn `link`).
    Link,
}

impl ButtonVariant {
    /// Whether the variant paints an opaque fill (so the hover/press wash must
    /// be overlaid on top rather than folded into the fill).
    fn is_solid(self) -> bool {
        matches!(self, Self::Primary | Self::Secondary | Self::Destructive)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Button<'a> {
    pub label: &'a str,
    /// Optional lucide icon path(s) (24x24 viewBox) stroked left of the label.
    /// A slice so multi-subpath icons (e.g. `braces`, `sparkles`) render in
    /// full — matching `IconButton` / `SelectTrigger`.
    pub icon_paths: Option<&'a [&'a str]>,
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
        let feedback = self.feedback_wash(t);
        let (fill, stroke, mut text_color) = self.colors(t, feedback);

        if let Some(color) = fill {
            p.fill_round_rect(rect, RADIUS, color);
        }
        // Solid variants (Primary / Secondary / Destructive) darken on
        // hover / press by overlaying the feedback wash on top of their fill —
        // shadcn primaries hover-darken. Ghost / Outline / DestructiveOutline
        // already fold the wash into their (otherwise transparent) fill, so the
        // overlay only applies where a solid fill would hide it.
        if self.variant.is_solid() {
            if let Some(wash) = feedback {
                p.fill_round_rect(rect, RADIUS, wash);
            }
        }
        if let Some(color) = stroke {
            p.stroke_round_rect(rect, RADIUS, color, 1.0);
        }

        if !self.enabled {
            text_color = text_color.with_alpha(0.5);
        }

        let label_width =
            p.measure_text_family_styled(self.label, font_size, FONT_FAMILY, 400, false);
        let icon_size = font_size + 3.0;
        let has_icon = self.icon_paths.is_some();
        let has_label = !self.label.is_empty();
        let icon_width = if has_icon { icon_size } else { 0.0 };
        let gap = if has_icon && has_label {
            ICON_LABEL_GAP
        } else {
            0.0
        };
        let available_width = rect.size.x.max(0.0);
        let label_box_width = label_width.min((available_width - icon_width - gap).max(0.0));
        let visible_gap = if has_icon && label_box_width > 0.0 {
            gap
        } else {
            0.0
        };
        let visible_width = icon_width + visible_gap + label_box_width;
        let mut x = rect.origin.x + (available_width - visible_width) / 2.0;

        p.save();
        p.clip_rect(rect);

        if let Some(paths) = self.icon_paths {
            let top_left = Point2D::new(x, rect.origin.y + (rect.size.y - icon_size) / 2.0);
            for d in paths {
                p.stroke_svg_path(d, top_left, icon_size, text_color, 1.75);
            }
            x += icon_size + visible_gap;
        }

        if label_box_width > 0.0 {
            let label_center = x + label_box_width / 2.0;
            let safe_left = if has_icon {
                x - visible_gap
            } else {
                rect.origin.x
            };
            let safe_right = rect.origin.x + available_width;
            let half_width = (label_center - safe_left)
                .min(safe_right - label_center)
                .max(0.0);
            let label_rect = Rect::xywh(
                label_center - half_width,
                rect.origin.y,
                half_width * 2.0,
                rect.size.y,
            );
            TextBox::new(self.label)
                .with_font_family(FONT_FAMILY)
                .with_font_size(font_size)
                .with_color(text_color)
                .with_horizontal_align(HorizontalAlign::Center)
                .with_vertical_align(VerticalAlign::Center)
                .paint(p, label_rect);
        }

        p.restore();
    }

    pub fn hit(rect: Rect, point: Point2D) -> bool {
        rect.contains(point)
    }

    /// The hover / press wash, or `None` at rest / when disabled. Solid
    /// variants overlay it; transparent variants fold it into their fill.
    fn feedback_wash(&self, t: &Tokens) -> Option<Color> {
        if !self.enabled {
            return None;
        }
        if self.pressed {
            Some(t.button_hover.with_alpha(t.button_hover.a * 1.8))
        } else if self.hovered {
            Some(t.button_hover)
        } else {
            None
        }
    }

    fn colors(&self, t: &Tokens, feedback: Option<Color>) -> (Option<Color>, Option<Color>, Color) {
        let (fill, stroke, text) = match self.variant {
            ButtonVariant::Ghost => (feedback, None, t.foreground),
            ButtonVariant::Primary => (Some(t.primary), None, t.primary_foreground),
            ButtonVariant::Outline => (feedback, Some(t.border), t.foreground),
            ButtonVariant::Destructive => (Some(t.destructive), None, t.primary_foreground),
            ButtonVariant::Secondary => (Some(t.secondary), None, t.secondary_foreground),
            ButtonVariant::DestructiveOutline => (feedback, Some(t.destructive), t.destructive),
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
            icon_paths: None,
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
            icon_paths: None,
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
            icon_paths: None,
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
        let icon_paths: &[&str] = &[icon];
        let b = Button {
            label: "Open",
            icon_paths: Some(icon_paths),
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
    fn label_uses_a_top_left_origin_centered_in_the_control() {
        let t = Tokens::dark();
        let rect = Rect::xywh(0.0, 0.0, 80.0, 30.0);
        let mut p = CapturePainter::default();
        Button {
            label: "Run",
            icon_paths: None,
            variant: ButtonVariant::Primary,
            enabled: true,
            hovered: false,
            pressed: false,
            font_size: 13.0,
        }
        .paint(&mut p, rect, &t);
        let (_, origin, _) = p.texts().next().expect("label should be painted");
        assert!((origin.y - (rect.origin.y + (rect.size.y - 13.0) / 2.0)).abs() < 0.01);
    }

    #[test]
    fn secondary_paints_secondary_fill_and_foreground() {
        let t = Tokens::dark();
        let b = Button {
            label: "Cancel",
            icon_paths: None,
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
    fn primary_hovered_overlays_feedback_wash_on_solid_fill() {
        let t = Tokens::dark();
        let b = Button {
            label: "Export",
            icon_paths: None,
            variant: ButtonVariant::Primary,
            enabled: true,
            hovered: true,
            pressed: false,
            font_size: 13.0,
        };
        let mut p = CapturePainter::default();

        b.paint(&mut p, Rect::xywh(0.0, 0.0, 90.0, 30.0), &t);

        // Solid primary fill + the hover wash overlaid on top.
        assert_eq!(p.fills_with(t.primary), 1);
        assert_eq!(p.fills_with(t.button_hover), 1);
    }

    #[test]
    fn destructive_outline_strokes_destructive_border_and_text() {
        let t = Tokens::dark();
        let b = Button {
            label: "Disconnect",
            icon_paths: None,
            variant: ButtonVariant::DestructiveOutline,
            enabled: true,
            hovered: false,
            pressed: false,
            font_size: 12.0,
        };
        let mut p = CapturePainter::default();

        b.paint(&mut p, Rect::xywh(0.0, 0.0, 96.0, 28.0), &t);

        // Destructive-colored border, no solid fill at rest.
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeRoundRect(_, _, c, _) if *c == t.destructive
        )));
        assert_eq!(p.fills_with(t.destructive), 0);
        let (_, _, color) = p.texts().next().expect("label should be painted");
        assert_eq!(color, t.destructive.to_jian());
    }

    #[test]
    fn link_paints_no_fill_with_primary_text() {
        let t = Tokens::dark();
        let b = Button {
            label: "Learn more",
            icon_paths: None,
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
