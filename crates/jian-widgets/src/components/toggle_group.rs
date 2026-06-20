//! ToggleGroup — a segmented single-select control of bordered cells (shadcn
//! `ToggleGroup`, single mode). Distinct from Tabs: a button-group look where
//! the active cell is a solid `primary` fill rather than an underline.

use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

const FONT_FAMILY: &str = "Inter";
const RADIUS: f32 = 6.0;

#[derive(Debug, Clone, Copy)]
pub struct ToggleGroup<'a> {
    pub options: &'a [&'a str],
    /// Optional per-cell icon paths (lucide d-strings), parallel to `options`.
    /// A cell with a non-empty entry strokes its icon centered, with the label
    /// beside it when the label is also non-empty; `None` (or an empty entry)
    /// → label-only. Lets a segmented control be icon-only (flex direction),
    /// label-only (text growth), or icon+label without the caller hand-rolling
    /// the cell content.
    pub icons: Option<&'a [&'a [&'a str]]>,
    /// Index of the active (selected) option.
    pub active: usize,
    /// Index currently hovered, if any.
    pub hover: Option<usize>,
    /// `<= 0` derives from the density font size; set it for compact toggles.
    pub font_size: f32,
}

impl ToggleGroup<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        if self.options.is_empty() {
            return;
        }
        let count = self.options.len();
        let cell_w = rect.size.x / count as f32;
        let font_size = if self.font_size > 0.0 {
            self.font_size
        } else {
            t.density.font_size()
        };

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

            let content_color = if is_active {
                t.primary_foreground
            } else {
                t.foreground
            };
            // Center an (optional icon)+(optional label) group in the cell.
            let icon_paths = self
                .icons
                .and_then(|ic| ic.get(i))
                .copied()
                .filter(|paths| !paths.is_empty());
            let label_w = if label.is_empty() {
                0.0
            } else {
                p.measure_text(label, font_size)
            };
            let icon_size = if icon_paths.is_some() {
                font_size + 4.0
            } else {
                0.0
            };
            let inner_gap = if icon_paths.is_some() && !label.is_empty() {
                6.0
            } else {
                0.0
            };
            let total = icon_size + inner_gap + label_w;
            let mut content_x = cell.origin.x + (cell_w - total) / 2.0;
            if let Some(paths) = icon_paths {
                let top_left =
                    Point2D::new(content_x, cell.origin.y + (rect.size.y - icon_size) / 2.0);
                for d in paths {
                    p.stroke_svg_path(d, top_left, icon_size, content_color, 1.5);
                }
                content_x += icon_size + inner_gap;
            }
            if !label.is_empty() {
                let origin =
                    Point2D::new(content_x, crate::centered_text_baseline_y(cell, font_size));
                let layout = TextLayout::single_run(
                    label,
                    FONT_FAMILY,
                    font_size,
                    content_color.to_jian(),
                    Point2D::new(0.0, 0.0),
                );
                p.draw_text(&layout, origin);
            }
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
            icons: None,
            active,
            hover,
            font_size: 0.0,
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
    fn icon_only_cells_stroke_per_cell_icon() {
        use crate::test_support::PaintOp;
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        let icons: &[&[&str]] = &[&["M3 3h6v6H3z"], &["M4 12h16"], &["M12 4v16"]];
        ToggleGroup {
            options: &["", "", ""],
            icons: Some(icons),
            active: 0,
            hover: None,
            font_size: 0.0,
        }
        .paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 24.0), &t);

        // Each cell strokes its own icon path; the active cell's icon uses
        // primary_foreground, the others use foreground.
        for d in ["M3 3h6v6H3z", "M4 12h16", "M12 4v16"] {
            assert!(p
                .ops
                .iter()
                .any(|op| matches!(op, PaintOp::StrokeSvgPath { d: dd, .. } if dd == d)));
        }
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
