use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

const MENU_WIDTH: f32 = 180.0;
const FONT_FAMILY: &str = "Inter";

pub struct MenuItem<'a> {
    pub label: &'a str,
    pub icon_d: Option<&'a str>,
    pub danger: bool,
    pub disabled: bool,
    pub separator_above: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MenuState {
    pub hover: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuHit {
    Row(usize),
    Inside,
    Outside,
}

pub struct Menu<'a> {
    pub state: &'a MenuState,
    pub items: &'a [MenuItem<'a>],
}

impl Menu<'_> {
    pub fn paint(&self, p: &mut dyn Painter, anchor: Point2D, viewport: Rect, t: &Tokens) {
        let rect = Self::menu_rect(anchor, viewport, self.items.len(), t);
        let row_h = t.density.row_height();
        let font_size = t.density.font_size();
        let visible = visible_row_capacity(self.items.len(), rect.size.y, row_h);

        p.fill_round_rect(rect, 6.0, t.popover);
        p.stroke_round_rect(rect, 6.0, t.border, 1.0);
        p.save();
        p.clip_rect(rect);

        for (index, item) in self.items.iter().take(visible).enumerate() {
            let row = Rect::xywh(
                rect.origin.x,
                rect.origin.y + index as f32 * row_h,
                rect.size.x,
                row_h,
            );
            if item.separator_above {
                p.stroke_line(
                    Point2D::new(row.origin.x + 8.0, row.origin.y),
                    Point2D::new(row.origin.x + row.size.x - 8.0, row.origin.y),
                    t.border,
                    1.0,
                );
            }
            if self.state.hover == Some(index) && !item.disabled {
                p.fill_rect(row, t.button_hover);
            }

            let mut color = if item.danger {
                t.destructive
            } else {
                t.popover_foreground
            };
            if item.disabled {
                color = t.muted_foreground.with_alpha(0.5);
            }

            let mut text_x = row.origin.x + 10.0;
            if let Some(d) = item.icon_d {
                p.stroke_svg_path(
                    d,
                    Point2D::new(row.origin.x + 10.0, row.origin.y + (row_h - 16.0) / 2.0),
                    16.0,
                    color,
                    1.75,
                );
                text_x += 26.0;
            }
            let origin = Point2D::new(text_x, row.origin.y + (row_h - font_size) / 2.0);
            let layout = TextLayout::single_run(
                item.label,
                FONT_FAMILY,
                font_size,
                color.to_jian(),
                Point2D::new(0.0, 0.0),
            );
            p.draw_text(&layout, origin);
        }

        p.restore();
    }

    pub fn menu_rect(anchor: Point2D, viewport: Rect, items: usize, t: &Tokens) -> Rect {
        let width = MENU_WIDTH.min(viewport.size.x);
        let height = (items as f32 * t.density.row_height()).min(viewport.size.y.max(0.0));
        let viewport_right = viewport.origin.x + viewport.size.x;
        let viewport_bottom = viewport.origin.y + viewport.size.y;
        let x = if anchor.x + width > viewport_right {
            anchor.x - width
        } else {
            anchor.x
        }
        .clamp(
            viewport.origin.x,
            (viewport_right - width).max(viewport.origin.x),
        );
        let y = if anchor.y + height > viewport_bottom {
            anchor.y - height
        } else {
            anchor.y
        }
        .clamp(
            viewport.origin.y,
            (viewport_bottom - height).max(viewport.origin.y),
        );
        Rect::xywh(x, y, width, height)
    }

    pub fn hit(
        anchor: Point2D,
        viewport: Rect,
        items: usize,
        point: Point2D,
        t: &Tokens,
    ) -> MenuHit {
        let rect = Self::menu_rect(anchor, viewport, items, t);
        if !rect.contains(point) {
            return MenuHit::Outside;
        }
        let row_h = t.density.row_height();
        if row_h <= 0.0 {
            return MenuHit::Inside;
        }
        let visible = visible_row_capacity(items, rect.size.y, row_h);
        let row = ((point.y - rect.origin.y) / row_h).floor().max(0.0) as usize;
        if row < visible {
            MenuHit::Row(row)
        } else {
            MenuHit::Inside
        }
    }
}

fn visible_row_capacity(row_count: usize, menu_height: f32, row_h: f32) -> usize {
    if row_count == 0 || row_h <= 0.0 || menu_height <= 0.0 {
        return 0;
    }
    ((menu_height / row_h).ceil() as usize).min(row_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::CapturePainter;

    #[test]
    fn menu_rect_flips_left_and_up_on_overflow() {
        let t = Tokens::dark();
        let viewport = Rect::xywh(0.0, 0.0, 240.0, 160.0);
        let rect = Menu::menu_rect(Point2D::new(230.0, 150.0), viewport, 3, &t);

        assert!(rect.origin.x < 230.0);
        assert!(rect.origin.y < 150.0);
        assert_eq!(rect.size.y, 3.0 * t.density.row_height());
    }

    #[test]
    fn menu_height_is_capped_to_viewport_and_paints_visible_rows() {
        let t = Tokens::dark();
        let viewport = Rect::xywh(0.0, 0.0, 240.0, t.density.row_height() * 2.0);
        let labels: Vec<_> = (0..6).map(|i| format!("Item {i}")).collect();
        let items: Vec<_> = labels
            .iter()
            .map(|label| MenuItem {
                label,
                icon_d: None,
                danger: false,
                disabled: false,
                separator_above: false,
            })
            .collect();
        let state = MenuState::default();
        let menu = Menu {
            state: &state,
            items: &items,
        };
        let mut p = CapturePainter::default();

        let rect = Menu::menu_rect(Point2D::new(20.0, 20.0), viewport, items.len(), &t);
        menu.paint(&mut p, Point2D::new(20.0, 20.0), viewport, &t);

        assert_eq!(rect.size.y, viewport.size.y);
        assert!(rect.origin.y + rect.size.y <= viewport.origin.y + viewport.size.y);
        let painted: Vec<_> = p
            .texts()
            .map(|(content, _, _)| content.to_owned())
            .collect();
        assert_eq!(painted, vec!["Item 0", "Item 1"]);
        assert_eq!(
            Menu::hit(
                Point2D::new(20.0, 20.0),
                viewport,
                items.len(),
                Point2D::new(rect.origin.x + 4.0, rect.origin.y + rect.size.y - 1.0),
                &t,
            ),
            MenuHit::Row(1)
        );
    }

    #[test]
    fn hit_returns_row_inside_or_outside() {
        let t = Tokens::dark();
        let viewport = Rect::xywh(0.0, 0.0, 240.0, 160.0);
        let anchor = Point2D::new(20.0, 20.0);
        let rect = Menu::menu_rect(anchor, viewport, 2, &t);

        assert_eq!(
            Menu::hit(
                anchor,
                viewport,
                2,
                Point2D::new(rect.origin.x + 5.0, rect.origin.y + 5.0),
                &t,
            ),
            MenuHit::Row(0)
        );
        assert_eq!(
            Menu::hit(
                anchor,
                viewport,
                2,
                Point2D::new(rect.origin.x + rect.size.x + 1.0, rect.origin.y + 5.0),
                &t,
            ),
            MenuHit::Outside
        );
    }

    #[test]
    fn danger_item_uses_destructive_text_color() {
        let t = Tokens::dark();
        let state = MenuState::default();
        let items = [MenuItem {
            label: "Delete",
            icon_d: None,
            danger: true,
            disabled: false,
            separator_above: false,
        }];
        let menu = Menu {
            state: &state,
            items: &items,
        };
        let mut p = CapturePainter::default();

        menu.paint(
            &mut p,
            Point2D::new(0.0, 0.0),
            Rect::xywh(0.0, 0.0, 200.0, 120.0),
            &t,
        );

        let (_, _, color) = p.texts().next().expect("item label should paint");
        assert_eq!(color, t.destructive.to_jian());
    }
}
