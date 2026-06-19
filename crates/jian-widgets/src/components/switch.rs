use crate::{Painter, Point2D, Rect, Tokens};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switch {
    pub on: bool,
    pub enabled: bool,
    pub hovered: bool,
    pub pressed: bool,
}

impl Switch {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let mut track = if self.on { t.primary } else { t.muted };
        if self.pressed {
            track = track.with_alpha((track.a * 0.85).max(0.0));
        }
        if !self.enabled {
            track = track.with_alpha(0.5);
        }

        let radius = rect.size.y / 2.0;
        p.fill_round_rect(rect, radius, track);

        if self.hovered && !self.pressed && self.enabled {
            p.stroke_round_rect(rect, radius, t.border, 1.0);
        }

        let knob = (rect.size.y - 4.0).max(0.0);
        let knob_x = if self.on {
            rect.origin.x + rect.size.x - knob - 2.0
        } else {
            rect.origin.x + 2.0
        };
        let knob_color = if self.enabled {
            crate::Color::WHITE
        } else {
            crate::Color::WHITE.with_alpha(0.5)
        };
        p.fill_oval(
            Rect::xywh(knob_x, rect.origin.y + 2.0, knob, knob),
            knob_color,
        );
    }

    pub fn hit_rect(rect: Rect, t: &Tokens) -> Rect {
        let min = t.density.min_hit_target();
        let extra_x = ((min - rect.size.x).max(0.0)) / 2.0;
        let extra_y = ((min - rect.size.y).max(0.0)) / 2.0;
        Rect::xywh(
            rect.origin.x - extra_x,
            rect.origin.y - extra_y,
            rect.size.x + extra_x * 2.0,
            rect.size.y + extra_y * 2.0,
        )
    }

    /// Whether `point` falls within the switch's density-expanded hit target.
    /// Mirrors the `hit` convention of `Button` / `Menu` / `Select` so hosts
    /// dispatch uniformly instead of special-casing `hit_rect`; folds in the
    /// touch-target expansion that `hit_rect` computes.
    pub fn hit(rect: Rect, point: Point2D, t: &Tokens) -> bool {
        Self::hit_rect(rect, t).contains(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn on_switch_paints_primary_track_and_right_knob() {
        let t = Tokens::dark();
        let s = Switch {
            on: true,
            enabled: true,
            hovered: false,
            pressed: false,
        };
        let mut p = CapturePainter::default();

        s.paint(&mut p, Rect::xywh(0.0, 0.0, 40.0, 22.0), &t);

        assert_eq!(p.fills_with(t.primary), 1);
        assert!(p
            .ops
            .iter()
            .any(|op| { matches!(op, PaintOp::FillOval(rect, _) if rect.origin.x > 18.0) }));
    }

    #[test]
    fn disabled_switch_dims_track() {
        let t = Tokens::dark();
        let s = Switch {
            on: false,
            enabled: false,
            hovered: false,
            pressed: false,
        };
        let mut p = CapturePainter::default();

        s.paint(&mut p, Rect::xywh(0.0, 0.0, 40.0, 22.0), &t);

        assert_eq!(p.fills_with(t.muted.with_alpha(0.5)), 1);
    }

    #[test]
    fn touch_density_expands_hit_rect_to_minimum_target() {
        let mut t = Tokens::dark();
        t.density = crate::Density::Touch;
        let hit = Switch::hit_rect(Rect::xywh(10.0, 10.0, 36.0, 20.0), &t);

        assert!(hit.size.x >= 44.0);
        assert!(hit.size.y >= 44.0);
        assert!(hit.origin.x < 10.0);
        assert!(hit.origin.y < 10.0);
    }

    #[test]
    fn hit_uses_density_expanded_target() {
        let mut t = Tokens::dark();
        t.density = crate::Density::Touch;
        let rect = Rect::xywh(10.0, 10.0, 36.0, 20.0);
        // Just above the visual rect but inside the expanded touch target.
        assert!(Switch::hit(rect, Point2D::new(28.0, 6.0), &t));
        // Far outside misses.
        assert!(!Switch::hit(rect, Point2D::new(200.0, 200.0), &t));
    }
}
