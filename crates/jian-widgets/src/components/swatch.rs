//! Swatch — a color chip that reveals transparency via an alpha checkerboard
//! painted behind translucent colors. Paint-only: no hit / state.

use crate::{Color, Painter, Rect, Tokens};

/// Size of each checkerboard cell in px.
const CHECKER: f32 = 5.0;
const DEFAULT_RADIUS: f32 = 4.0;
/// Transparency-checker shades. THEME-INDEPENDENT light greys (not `muted`/
/// `card` tokens) so the pattern stays visible on any surface — a dark-theme
/// card behind a translucent swatch must not swallow the checker.
const CHECKER_LIGHT: Color = Color {
    r: 0.95,
    g: 0.95,
    b: 0.95,
    a: 1.0,
};
const CHECKER_DARK: Color = Color {
    r: 0.78,
    g: 0.78,
    b: 0.78,
    a: 1.0,
};

#[derive(Debug, Clone, Copy)]
pub struct Swatch {
    /// The (possibly translucent) chip color.
    pub color: Color,
    /// Corner radius in px. `<= 0` falls back to a shadcn `rounded` (4px).
    pub radius: f32,
    /// Whether to stroke a 1px border around the chip.
    pub border: bool,
}

impl Swatch {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let radius = if self.radius > 0.0 {
            self.radius
        } else {
            DEFAULT_RADIUS
        };

        // Only reveal a checkerboard when the color is translucent.
        if self.color.a < 1.0 {
            self.paint_checker(p, rect);
        }

        // The (possibly translucent) color tints whatever sits behind it.
        p.fill_round_rect(rect, radius, self.color);

        if self.border {
            p.stroke_round_rect(rect, radius, t.border, 1.0);
        }
    }

    /// Tile alternating light/dark cells across `rect`, clipped to its bounds.
    fn paint_checker(&self, p: &mut dyn Painter, rect: Rect) {
        p.save();
        p.clip_rect(rect);
        let light = CHECKER_LIGHT;
        let dark = CHECKER_DARK;
        let cols = (rect.size.x / CHECKER).ceil() as usize;
        let rows = (rect.size.y / CHECKER).ceil() as usize;
        for row in 0..rows {
            for col in 0..cols {
                let cell = if (row + col) % 2 == 0 { light } else { dark };
                let x = rect.origin.x + col as f32 * CHECKER;
                let y = rect.origin.y + row as f32 * CHECKER;
                p.fill_rect(Rect::xywh(x, y, CHECKER, CHECKER), cell);
            }
        }
        p.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::CapturePainter;

    #[test]
    fn opaque_color_paints_a_single_fill_no_checker() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        let swatch = Swatch {
            color: Color::rgb_u8(0x33, 0x66, 0x99),
            radius: 0.0,
            border: false,
        };
        swatch.paint(&mut p, Rect::xywh(0.0, 0.0, 20.0, 20.0), &t);

        // Opaque -> no checker cells, just the one color fill.
        assert_eq!(p.fills_with(swatch.color), 1);
        assert_eq!(p.fills_with(CHECKER_LIGHT), 0);
        assert_eq!(p.fills_with(CHECKER_DARK), 0);
    }

    #[test]
    fn translucent_color_paints_checker_then_color() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        let color = Color::rgba_u8(0xff, 0x00, 0x00, 0.5);
        let swatch = Swatch {
            color,
            radius: 4.0,
            border: false,
        };
        swatch.paint(&mut p, Rect::xywh(0.0, 0.0, 20.0, 20.0), &t);

        // Multiple checker cells (both shades) plus the tinting color fill.
        assert!(p.fills_with(CHECKER_LIGHT) > 1);
        assert!(p.fills_with(CHECKER_DARK) > 1);
        assert_eq!(p.fills_with(color), 1);
    }

    #[test]
    fn border_flag_strokes_a_rounded_rect() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        let swatch = Swatch {
            color: Color::WHITE,
            radius: 0.0,
            border: true,
        };
        swatch.paint(&mut p, Rect::xywh(0.0, 0.0, 16.0, 16.0), &t);

        assert!(p.ops.iter().any(|op| matches!(
            op,
            crate::test_support::PaintOp::StrokeRoundRect(_, _, c, _) if *c == t.border
        )));
    }

    #[test]
    fn zero_radius_falls_back_to_default() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        let swatch = Swatch {
            color: Color::BLACK,
            radius: 0.0,
            border: false,
        };
        swatch.paint(&mut p, Rect::xywh(0.0, 0.0, 16.0, 16.0), &t);

        assert!(p.ops.iter().any(|op| matches!(
            op,
            crate::test_support::PaintOp::FillRoundRect(_, r, _) if (*r - DEFAULT_RADIUS).abs() < 0.01
        )));
    }
}
