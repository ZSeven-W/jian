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
/// lucide `chevron-down` — for a tab that doubles as a dropdown trigger.
const CHEVRON_D: &str = "m6 9 6 6 6-6";

/// How the active content-width tab is emphasized. The even-cell [`Tabs::paint`]
/// always uses the shadcn muted-track + background pill; content-width strips
/// (variable-width, scrollable) pick one of these instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveStyle {
    /// Active tab fills a solid `primary` pill (a segmented chip strip).
    #[default]
    PrimaryPill,
    /// Active tab is just `foreground`-colored text (an underline-less text tab).
    Text,
}

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

    /// Pure-geometry layout for a CONTENT-WIDTH strip: one rect per tab, each
    /// sized by the caller-supplied (deterministic) `widths`, laid left→right
    /// with `gap`, the whole row shifted by `-scroll`. The caller owns the width
    /// function so the SAME rects drive paint (`paint_content`) and hit
    /// (`content_at`) without a `Painter` in the hit path — the standard
    /// immediate-mode escape from "hit-test needs to measure text".
    pub fn content_rects(
        origin: Point2D,
        widths: &[f32],
        height: f32,
        gap: f32,
        scroll: f32,
    ) -> Vec<Rect> {
        let mut x = origin.x - scroll;
        let mut out = Vec::with_capacity(widths.len());
        for &w in widths {
            out.push(Rect::xywh(x, origin.y, w, height));
            x += w + gap;
        }
        out
    }

    /// Paint a content-width strip at precomputed `rects` (from `content_rects`).
    /// The active tab is emphasized per `style`; an inactive hovered tab gets a
    /// `button_hover` wash. The label sits at `leading_pad` from each rect's
    /// left; when `chevron_on_active` the active tab gets a trailing chevron-down
    /// (a tab that is also a dropdown trigger). `font_size <= 0` → density-free
    /// default. The caller clips/culls (e.g. a scrolling band) around this call.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_content(
        &self,
        p: &mut dyn Painter,
        rects: &[Rect],
        style: ActiveStyle,
        chevron_on_active: bool,
        leading_pad: f32,
        font_size: f32,
        t: &Tokens,
    ) {
        let fs = if font_size > 0.0 {
            font_size
        } else {
            FONT_SIZE
        };
        for (i, r) in rects.iter().enumerate() {
            let is_active = i == self.active;
            if is_active && style == ActiveStyle::PrimaryPill {
                p.fill_round_rect(*r, PILL_RADIUS, t.primary);
            } else if self.hover == Some(i) {
                p.fill_round_rect(*r, PILL_RADIUS, t.button_hover);
            }
            let color = if is_active {
                match style {
                    ActiveStyle::PrimaryPill => t.primary_foreground,
                    ActiveStyle::Text => t.foreground,
                }
            } else {
                t.muted_foreground
            };
            if let Some(label) = self.labels.get(i) {
                if !label.is_empty() {
                    let origin =
                        Point2D::new(r.origin.x + leading_pad, r.origin.y + (r.size.y - fs) / 2.0);
                    let layout = TextLayout::single_run(
                        label,
                        FONT_FAMILY,
                        fs,
                        color.to_jian(),
                        Point2D::new(0.0, 0.0),
                    );
                    p.draw_text(&layout, origin);
                }
            }
            if is_active && chevron_on_active {
                let chev = 11.0;
                let origin = Point2D::new(
                    r.origin.x + r.size.x - chev - 2.0,
                    r.origin.y + (r.size.y - chev) / 2.0,
                );
                p.stroke_svg_path(CHEVRON_D, origin, chev, t.muted_foreground, 1.5);
            }
        }
    }

    /// Hit-test a content-width strip — the first `rect` containing `point`.
    pub fn content_at(rects: &[Rect], point: Point2D) -> Option<usize> {
        rects.iter().position(|r| r.contains(point))
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

    #[test]
    fn content_rects_lays_out_variable_widths_minus_scroll() {
        let rects =
            Tabs::content_rects(Point2D::new(10.0, 0.0), &[40.0, 60.0, 30.0], 24.0, 6.0, 5.0);
        assert_eq!(rects.len(), 3);
        // First tab starts at origin.x - scroll = 5.0; each advances by width+gap.
        assert!((rects[0].origin.x - 5.0).abs() < 0.01 && (rects[0].size.x - 40.0).abs() < 0.01);
        assert!((rects[1].origin.x - (5.0 + 40.0 + 6.0)).abs() < 0.01);
        assert!((rects[2].origin.x - (5.0 + 40.0 + 6.0 + 60.0 + 6.0)).abs() < 0.01);
    }

    #[test]
    fn content_at_maps_point_to_variable_tab() {
        let rects = Tabs::content_rects(Point2D::new(0.0, 0.0), &[40.0, 60.0], 24.0, 6.0, 0.0);
        assert_eq!(Tabs::content_at(&rects, Point2D::new(20.0, 12.0)), Some(0));
        assert_eq!(Tabs::content_at(&rects, Point2D::new(70.0, 12.0)), Some(1));
        assert_eq!(Tabs::content_at(&rects, Point2D::new(200.0, 12.0)), None);
    }

    #[test]
    fn paint_content_primary_pill_fills_active_with_primary() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        let labels = ["React", "Vue", "Svelte"];
        let rects =
            Tabs::content_rects(Point2D::new(0.0, 0.0), &[50.0, 40.0, 56.0], 24.0, 6.0, 0.0);
        tabs(&labels, 1, None).paint_content(
            &mut p,
            &rects,
            ActiveStyle::PrimaryPill,
            false,
            8.0,
            12.0,
            &t,
        );
        assert_eq!(p.fills_with(t.primary), 1);
        let active = p
            .texts()
            .find(|(s, _, _)| *s == "Vue")
            .expect("active label");
        assert_eq!(active.2, t.primary_foreground.to_jian());
    }

    #[test]
    fn paint_content_text_style_active_is_foreground_with_chevron() {
        use crate::test_support::PaintOp;
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        let labels = ["Light", "Dark"];
        let rects = Tabs::content_rects(Point2D::new(0.0, 0.0), &[44.0, 40.0], 28.0, 4.0, 0.0);
        tabs(&labels, 0, None).paint_content(
            &mut p,
            &rects,
            ActiveStyle::Text,
            true,
            0.0,
            13.0,
            &t,
        );
        // Text style paints no primary pill.
        assert_eq!(p.fills_with(t.primary), 0);
        let active = p
            .texts()
            .find(|(s, _, _)| *s == "Light")
            .expect("active label");
        assert_eq!(active.2, t.foreground.to_jian());
        // Trailing chevron on the active tab.
        assert!(p
            .ops
            .iter()
            .any(|op| matches!(op, PaintOp::StrokeSvgPath { d, .. } if *d == CHEVRON_D)));
    }
}
