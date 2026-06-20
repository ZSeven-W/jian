//! IconButton — a self-contained icon button (think a React function component):
//! the hover / pressed / active / disabled feedback is the component's OWN
//! concern, painted automatically and centralised on [`Button`]. Callers pass
//! the lucide path(s) + state and call [`IconButton::paint`] — they never
//! hand-roll the background feedback or the icon color separately.

use crate::components::button::{Button, ButtonVariant};
use crate::{Color, Painter, Point2D, Rect, Tokens};

#[derive(Debug, Clone, Copy)]
pub struct IconButton<'a> {
    /// lucide d-string path(s) (24x24 viewBox); each is stroked, centred.
    pub icon_paths: &'a [&'a str],
    pub hovered: bool,
    pub pressed: bool,
    /// Solid primary highlight (e.g. a selected / active toggle button).
    pub active: bool,
    pub enabled: bool,
    /// Icon glyph size in px; `<= 0` derives from the density font size + 3.
    pub icon_size: f32,
    /// Icon stroke width; `<= 0` falls back to 1.5.
    pub stroke_width: f32,
}

impl IconButton<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let icon_color = self.paint_feedback(p, rect, t);
        let size = if self.icon_size > 0.0 {
            self.icon_size
        } else {
            t.density.font_size() + 3.0
        };
        let stroke = if self.stroke_width > 0.0 {
            self.stroke_width
        } else {
            1.5
        };
        let origin = Point2D::new(
            rect.origin.x + (rect.size.x - size) / 2.0,
            rect.origin.y + (rect.size.y - size) / 2.0,
        );
        for d in self.icon_paths {
            p.stroke_svg_path(d, origin, size, icon_color, stroke);
        }
    }

    /// Hit-test the button rect — the component owns this too, so a host routes
    /// clicks through `IconButton::hit` instead of an ad hoc `rect.contains`.
    pub fn hit(rect: Rect, point: Point2D) -> bool {
        rect.contains(point)
    }

    /// Paint the background feedback (active → solid primary via Button; else the
    /// ghost hover/press wash) and return the icon color it implies. Internal to
    /// the component — callers never see it.
    fn paint_feedback(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) -> Color {
        let variant = if self.active {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Ghost
        };
        Button {
            label: "",
            icon_d: None,
            variant,
            enabled: self.enabled,
            hovered: self.hovered,
            pressed: self.pressed,
            font_size: 0.0,
        }
        .paint(p, rect, t);

        let base = if self.active {
            t.primary_foreground
        } else if self.hovered || self.pressed {
            t.foreground
        } else {
            t.muted_foreground
        };
        if self.enabled {
            base
        } else {
            base.with_alpha(0.5)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    fn btn<'a>(paths: &'a [&'a str], active: bool, hovered: bool) -> IconButton<'a> {
        IconButton {
            icon_paths: paths,
            hovered,
            pressed: false,
            active,
            enabled: true,
            icon_size: 18.0,
            stroke_width: 1.6,
        }
    }

    #[test]
    fn active_fills_primary_and_strokes_icon() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        btn(&["M4 12h16"], true, false).paint(&mut p, Rect::xywh(0.0, 0.0, 32.0, 32.0), &t);

        assert_eq!(p.fills_with(t.primary), 1); // Button Primary background
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeSvgPath { d, color, .. }
                if *d == "M4 12h16" && *color == t.primary_foreground
        )));
    }

    #[test]
    fn inactive_hover_uses_ghost_wash_and_foreground_icon() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        btn(&["M4 12h16"], false, true).paint(&mut p, Rect::xywh(0.0, 0.0, 32.0, 32.0), &t);

        assert_eq!(p.fills_with(t.primary), 0);
        assert_eq!(p.fills_with(t.button_hover), 1); // ghost hover wash
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::StrokeSvgPath { color, .. } if *color == t.foreground
        )));
    }

    #[test]
    fn multi_path_icon_strokes_every_path() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        btn(&["M4 4h4", "M12 12h4"], false, false).paint(
            &mut p,
            Rect::xywh(0.0, 0.0, 32.0, 32.0),
            &t,
        );

        let strokes = p
            .ops
            .iter()
            .filter(|op| matches!(op, PaintOp::StrokeSvgPath { .. }))
            .count();
        assert_eq!(strokes, 2);
    }
}
