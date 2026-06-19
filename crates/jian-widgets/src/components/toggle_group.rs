//! ToggleGroup — a segmented single-select control of bordered cells (shadcn
//! `ToggleGroup`, single mode). Distinct from Tabs: a button-group look where
//! the active cell is a solid `primary` fill rather than an underline.

use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

const FONT_FAMILY: &str = "Inter";
const RADIUS: f32 = 6.0;

#[derive(Debug, Clone, Copy)]
pub struct ToggleGroup<'a> {
    pub options: &'a [&'a str],
    /// Index of the active (selected) option.
    pub active: usize,
    /// Index currently hovered, if any.
    pub hover: Option<usize>,
}

impl ToggleGroup<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        if self.options.is_empty() {
            return;
        }
        let count = self.options.len();
        let cell_w = rect.size.x / count as f32;
        let font_size = t.density.font_size();

        for (i, label) in self.options.iter().enumerate() {
            let x = rect.origin.x + cell_w * i as f32;
            let cell = Rect::xywh(x, rect.origin.y, cell_w, rect.size.y);
            let is_active = i == self.active;

            if is_active {
                p.fill_round_rect(cell, RADIUS, t.primary);
            } else if self.hover == Some(i) {
                p.fill_rect(cell, t.button_hover);
            }

            // 1px separator before every cell except the first one.
            if i > 0 {
                let top = Point2D::new(x, rect.origin.y);
                let bottom = Point2D::new(x, rect.origin.y + rect.size.y);
                p.stroke_line(top, bottom, t.border, 1.0);
            }

            let text_color = if is_active {
                t.primary_foreground
            } else {
                t.foreground
            };
            let text_w = p.measure_text(label, font_size);
            let origin = Point2D::new(
                cell.origin.x + (cell_w - text_w) / 2.0,
                cell.origin.y + (rect.size.y - font_size) / 2.0,
            );
            let layout = TextLayout::single_run(
                label,
                FONT_FAMILY,
                font_size,
                text_color.to_jian(),
                Point2D::new(0.0, 0.0),
            );
            p.draw_text(&layout, origin);
        }

        // Outer border drawn last so it frames the segments cleanly.
        p.stroke_round_rect(rect, RADIUS, t.border, 1.0);
    }

    /// Map a point inside `rect` to a segment index, given the segment `count`.
    pub fn segment_at(rect: Rect, count: usize, point: Point2D) -> Option<usize> {
        if count == 0 || !rect.contains(point) {
            return None;
        }
        let cell_w = rect.size.x / count as f32;
        let idx = ((point.x - rect.origin.x) / cell_w) as usize;
        Some(idx.min(count - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::CapturePainter;

    const OPTS: &[&str] = &["Left", "Center", "Right"];

    fn group(active: usize, hover: Option<usize>) -> ToggleGroup<'static> {
        ToggleGroup {
            options: OPTS,
            active,
            hover,
        }
    }

    #[test]
    fn active_cell_fills_primary_with_primary_foreground_text() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        group(1, None).paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 24.0), &t);

        // Exactly one cell fills the primary color.
        assert_eq!(p.fills_with(t.primary), 1);
        // The active label uses primary_foreground.
        let active = p
            .texts()
            .find(|(text, _, _)| *text == "Center")
            .expect("active label painted");
        assert_eq!(active.2, t.primary_foreground.to_jian());
    }

    #[test]
    fn inactive_labels_use_foreground() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        group(0, None).paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 24.0), &t);

        let inactive = p
            .texts()
            .find(|(text, _, _)| *text == "Right")
            .expect("inactive label painted");
        assert_eq!(inactive.2, t.foreground.to_jian());
    }

    #[test]
    fn hover_on_inactive_cell_washes_button_hover() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        group(0, Some(2)).paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 24.0), &t);

        assert_eq!(p.fills_with(t.button_hover), 1);
    }

    #[test]
    fn segment_at_maps_correctly() {
        let rect = Rect::xywh(0.0, 0.0, 120.0, 24.0);
        assert_eq!(
            ToggleGroup::segment_at(rect, 3, Point2D::new(10.0, 12.0)),
            Some(0)
        );
        assert_eq!(
            ToggleGroup::segment_at(rect, 3, Point2D::new(60.0, 12.0)),
            Some(1)
        );
        assert_eq!(
            ToggleGroup::segment_at(rect, 3, Point2D::new(115.0, 12.0)),
            Some(2)
        );
        // Outside the rect yields None.
        assert_eq!(
            ToggleGroup::segment_at(rect, 3, Point2D::new(200.0, 12.0)),
            None
        );
    }
}
