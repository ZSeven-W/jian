//! Badge — a small rounded label pill (shadcn `Badge`). Paint-only: no hit /
//! state. Content (icon + label) is centered as one group within horizontal
//! padding so caller-sized pills retain balanced whitespace.

use crate::{Color, HorizontalAlign, Painter, Point2D, Rect, TextBox, Tokens, VerticalAlign};

const FONT_FAMILY: &str = "Inter";
const PAD_X: f32 = 8.0;
const ICON_LABEL_GAP: f32 = 4.0;
const DEFAULT_RADIUS: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeVariant {
    /// Primary fill (shadcn default).
    #[default]
    Default,
    /// Muted neutral fill (shadcn `secondary`).
    Secondary,
    /// Destructive (red) fill.
    Destructive,
    /// Border only, transparent fill (shadcn `outline`).
    Outline,
    /// Translucent primary tint (no border) — the common in-app "source" chip.
    Tinted,
}

#[derive(Debug, Clone, Copy)]
pub struct Badge<'a> {
    pub label: &'a str,
    /// Optional lucide d-string (24x24 viewBox) stroked left of the label.
    pub icon_d: Option<&'a str>,
    pub variant: BadgeVariant,
    /// Corner radius in px. `<= 0` falls back to a shadcn `rounded-md` (6px).
    pub radius: f32,
    /// Label font size in px. `<= 0` falls back to 11px (shadcn `text-xs`-ish).
    pub font_size: f32,
}

impl Badge<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let radius = if self.radius > 0.0 {
            self.radius
        } else {
            DEFAULT_RADIUS
        };
        let font_size = if self.font_size > 0.0 {
            self.font_size
        } else {
            11.0
        };
        let (fill, stroke, text_color) = self.colors(t);
        if let Some(c) = fill {
            p.fill_round_rect(rect, radius, c);
        }
        if let Some(c) = stroke {
            p.stroke_round_rect(rect, radius, c, 1.0);
        }

        let cy = rect.origin.y + rect.size.y / 2.0;
        let icon_size = font_size + 1.0;
        let label_width = if self.label.is_empty() {
            0.0
        } else {
            p.measure_text_family_styled(self.label, font_size, FONT_FAMILY, 400, false)
        };
        let gap = if self.icon_d.is_some() && !self.label.is_empty() {
            ICON_LABEL_GAP
        } else {
            0.0
        };
        let icon_width = if self.icon_d.is_some() {
            icon_size
        } else {
            0.0
        };
        let control_width = rect.size.x.max(0.0);
        let inset = PAD_X.min(control_width / 2.0);
        let content_width = (control_width - inset * 2.0).max(0.0);
        let label_box_width = label_width.min((content_width - icon_width - gap).max(0.0));
        let visible_gap = if self.icon_d.is_some() && label_box_width > 0.0 {
            gap
        } else {
            0.0
        };
        let visible_width = icon_width + visible_gap + label_box_width;
        let mut x = rect.origin.x + inset + (content_width - visible_width) / 2.0;

        p.save();
        p.clip_rect(rect);

        if let Some(d) = self.icon_d {
            let top_left = Point2D::new(x, cy - icon_size / 2.0);
            p.stroke_svg_path(d, top_left, icon_size, text_color, 1.5);
            x += icon_size + visible_gap;
        }
        if label_box_width > 0.0 {
            let label_center = x + label_box_width / 2.0;
            let safe_left = if self.icon_d.is_some() {
                x - visible_gap
            } else {
                rect.origin.x + inset
            };
            let safe_right = rect.origin.x + control_width - inset;
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

    fn colors(&self, t: &Tokens) -> (Option<Color>, Option<Color>, Color) {
        match self.variant {
            BadgeVariant::Default => (Some(t.primary), None, t.primary_foreground),
            BadgeVariant::Secondary => (Some(t.secondary), None, t.secondary_foreground),
            BadgeVariant::Destructive => (Some(t.destructive), None, t.primary_foreground),
            BadgeVariant::Outline => (None, Some(t.border), t.foreground),
            BadgeVariant::Tinted => (Some(t.primary.with_alpha(0.12)), None, t.foreground),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::CapturePainter;

    fn badge<'a>(label: &'a str, variant: BadgeVariant) -> Badge<'a> {
        Badge {
            label,
            icon_d: None,
            variant,
            radius: 0.0,
            font_size: 0.0,
        }
    }

    #[test]
    fn default_badge_fills_primary_with_primary_foreground_text() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        badge("New", BadgeVariant::Default).paint(&mut p, Rect::xywh(0.0, 0.0, 40.0, 16.0), &t);

        assert_eq!(p.fills_with(t.primary), 1);
        let (_, _, color) = p.texts().next().expect("label should be painted");
        assert_eq!(color, t.primary_foreground.to_jian());
    }

    #[test]
    fn outline_badge_strokes_border_without_fill() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        badge("Tag", BadgeVariant::Outline).paint(&mut p, Rect::xywh(0.0, 0.0, 40.0, 16.0), &t);

        assert_eq!(p.fills_with(t.primary), 0);
        let (_, _, color) = p.texts().next().expect("label should be painted");
        assert_eq!(color, t.foreground.to_jian());
    }

    #[test]
    fn tinted_badge_uses_translucent_primary() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        badge("mcp", BadgeVariant::Tinted).paint(&mut p, Rect::xywh(0.0, 0.0, 40.0, 12.0), &t);

        assert_eq!(p.fills_with(t.primary.with_alpha(0.12)), 1);
    }

    #[test]
    fn radius_and_font_fall_back_to_defaults_when_zero() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        // A 16px-tall badge with radius 0 should still render a rounded fill.
        badge("x", BadgeVariant::Secondary).paint(&mut p, Rect::xywh(0.0, 0.0, 20.0, 16.0), &t);
        assert_eq!(p.fills_with(t.secondary), 1);
    }
}
