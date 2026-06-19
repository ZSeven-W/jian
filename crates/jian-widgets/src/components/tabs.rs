//! Tabs — a horizontal tab strip in the shadcn "muted container + active pill"
//! style. Paint-only: the active index and hover come from the caller. The
//! container is divided into N equal cells; the active cell paints a raised
//! pill, inactive cells paint muted text (hovered inactive brightens).

use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

const FONT_FAMILY: &str = "Inter";
const CONTAINER_RADIUS: f32 = 6.0;
const PILL_RADIUS: f32 = 4.0;
const PILL_INSET: f32 = 2.0;
const FONT_SIZE: f32 = 12.0;

#[derive(Debug, Clone, Copy)]
pub struct Tabs<'a> {
    pub labels: &'a [&'a str],
    /// Index of the active tab.
    pub active: usize,
    /// Index of the currently hovered tab, if any.
    pub hover: Option<usize>,
}

impl Tabs<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        p.fill_round_rect(rect, CONTAINER_RADIUS, t.muted);

        let count = self.labels.len();
        if count == 0 {
            return;
        }
        let cell_w = rect.size.x / count as f32;

        for (i, label) in self.labels.iter().enumerate() {
            let cell = Rect::xywh(
                rect.origin.x + cell_w * i as f32,
                rect.origin.y,
                cell_w,
                rect.size.y,
            );

            let text_color = if i == self.active {
                // Raised pill under the active tab.
                let pill = Rect::xywh(
                    cell.origin.x + PILL_INSET,
                    cell.origin.y + PILL_INSET,
                    (cell.size.x - PILL_INSET * 2.0).max(0.0),
                    (cell.size.y - PILL_INSET * 2.0).max(0.0),
                );
                p.fill_round_rect(pill, PILL_RADIUS, t.background);
                t.foreground
            } else if self.hover == Some(i) {
                t.foreground
            } else {
                t.muted_foreground
            };

            if label.is_empty() {
                continue;
            }
            let text_w = p.measure_text(label, FONT_SIZE);
            let origin = Point2D::new(
                cell.origin.x + (cell.size.x - text_w) / 2.0,
                cell.origin.y + (cell.size.y - FONT_SIZE) / 2.0,
            );
            let layout = TextLayout::single_run(
                label,
                FONT_FAMILY,
                FONT_SIZE,
                text_color.to_jian(),
                Point2D::new(0.0, 0.0),
            );
            p.draw_text(&layout, origin);
        }
    }

    /// Map an x position to a cell index, or `None` if the point is outside the
    /// strip. Pure geometry — mirrors the equal-cell layout used by `paint`.
    pub fn tab_at(rect: Rect, count: usize, point: Point2D) -> Option<usize> {
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

    fn tabs<'a>(labels: &'a [&'a str], active: usize, hover: Option<usize>) -> Tabs<'a> {
        Tabs {
            labels,
            active,
            hover,
        }
    }

    #[test]
    fn active_tab_paints_background_pill_once() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        let labels = ["One", "Two", "Three"];
        tabs(&labels, 1, None).paint(&mut p, Rect::xywh(0.0, 0.0, 240.0, 32.0), &t);

        // Container fills muted; exactly one pill fills background.
        assert_eq!(p.fills_with(t.muted), 1);
        assert_eq!(p.fills_with(t.background), 1);
    }

    #[test]
    fn active_label_uses_foreground_color() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        let labels = ["A", "B"];
        tabs(&labels, 0, None).paint(&mut p, Rect::xywh(0.0, 0.0, 160.0, 32.0), &t);

        let (text, _, color) = p.texts().next().expect("first label should paint");
        assert_eq!(text, "A");
        assert_eq!(color, t.foreground.to_jian());
    }

    #[test]
    fn tab_at_maps_mid_cell_point_to_index() {
        let rect = Rect::xywh(0.0, 0.0, 300.0, 32.0);
        // Cells are 100px wide: 0=[0,100), 1=[100,200), 2=[200,300).
        assert_eq!(Tabs::tab_at(rect, 3, Point2D::new(150.0, 16.0)), Some(1));
        assert_eq!(Tabs::tab_at(rect, 3, Point2D::new(50.0, 16.0)), Some(0));
        assert_eq!(Tabs::tab_at(rect, 3, Point2D::new(250.0, 16.0)), Some(2));
    }

    #[test]
    fn tab_at_returns_none_outside_rect() {
        let rect = Rect::xywh(10.0, 10.0, 200.0, 32.0);
        assert_eq!(Tabs::tab_at(rect, 4, Point2D::new(5.0, 20.0)), None);
        assert_eq!(Tabs::tab_at(rect, 0, Point2D::new(50.0, 20.0)), None);
    }
}
