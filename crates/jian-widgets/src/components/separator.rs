//! Separator — a hairline divider (shadcn `Separator`). Paint-only: fills a
//! `thickness`-px line centred along the orientation axis of `rect`, so the ad
//! hoc 1px `fill_rect` dividers across the chrome (toolbars, panels, top bar)
//! share one primitive. The color is caller-supplied (border, or border*0.6).

use crate::{Color, Painter, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy)]
pub struct Separator {
    pub orientation: Orientation,
    /// Line thickness in px; `<= 0` falls back to a 1px hairline.
    pub thickness: f32,
}

impl Separator {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, color: Color) {
        let th = if self.thickness > 0.0 {
            self.thickness
        } else {
            1.0
        };
        let line = match self.orientation {
            Orientation::Horizontal => Rect::xywh(
                rect.origin.x,
                rect.origin.y + (rect.size.y - th) / 2.0,
                rect.size.x,
                th,
            ),
            Orientation::Vertical => Rect::xywh(
                rect.origin.x + (rect.size.x - th) / 2.0,
                rect.origin.y,
                th,
                rect.size.y,
            ),
        };
        p.fill_rect(line, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn horizontal_fills_centred_hairline() {
        let color = Color::rgb_u8(0x26, 0x26, 0x26);
        let mut p = CapturePainter::default();
        Separator {
            orientation: Orientation::Horizontal,
            thickness: 0.0,
        }
        .paint(&mut p, Rect::xywh(10.0, 10.0, 100.0, 20.0), color);

        // A 1px line spanning the full width, vertically centred in the 20px band.
        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::FillRect(r, c)
                if *c == color && r.size.y == 1.0 && r.size.x == 100.0
                    && (r.origin.y - 19.5).abs() < 0.01
        )));
    }

    #[test]
    fn vertical_uses_thickness_and_full_height() {
        let color = Color::rgb_u8(0x26, 0x26, 0x26);
        let mut p = CapturePainter::default();
        Separator {
            orientation: Orientation::Vertical,
            thickness: 2.0,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 10.0, 40.0), color);

        assert!(p.ops.iter().any(|op| matches!(
            op,
            PaintOp::FillRect(r, _) if r.size.x == 2.0 && r.size.y == 40.0
        )));
    }
}
