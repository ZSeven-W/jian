use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

pub const MAX_VISIBLE_ROWS: usize = 8;
const FONT_FAMILY: &str = "Inter";
const CHECK_D: &str = "M20 6 9 17l-5-5";

#[derive(Debug, Clone, Default)]
pub struct SelectState {
    pub open: bool,
    pub hover: Option<usize>,
    pub pressed: Option<usize>,
    pub scroll: jian_core::scroll::ScrollState,
}

pub struct SelectItem<'a> {
    pub label: &'a str,
    pub selected: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectHit {
    Row(usize),
    Inside,
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectKeyCode {
    ArrowUp,
    ArrowDown,
    Home,
    End,
    Enter,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectKey {
    pub code: SelectKeyCode,
    pub is_composing: bool,
}

impl SelectState {
    pub fn apply_key(&mut self, key: SelectKey, item_count: usize) -> Option<usize> {
        if item_count == 0 {
            self.hover = None;
            self.scroll.offset = 0.0;
            return None;
        }

        if key.is_composing
            && matches!(
                key.code,
                SelectKeyCode::ArrowUp
                    | SelectKeyCode::ArrowDown
                    | SelectKeyCode::Home
                    | SelectKeyCode::End
            )
        {
            return None;
        }

        let last = item_count - 1;
        match key.code {
            SelectKeyCode::ArrowUp => {
                let next = self.hover.unwrap_or(0).saturating_sub(1);
                self.set_hover(next);
            }
            SelectKeyCode::ArrowDown => {
                let next = self.hover.map_or(0, |i| (i + 1).min(last));
                self.set_hover(next);
            }
            SelectKeyCode::Home => self.set_hover(0),
            SelectKeyCode::End => self.set_hover(last),
            SelectKeyCode::Enter => {
                return self.hover.filter(|i| *i < item_count);
            }
            SelectKeyCode::Escape => {
                self.open = false;
            }
        }
        None
    }

    fn set_hover(&mut self, index: usize) {
        self.hover = Some(index);
        self.scroll
            .reveal(index as f32, index as f32 + 1.0, MAX_VISIBLE_ROWS as f32);
    }
}

pub struct Select<'a> {
    pub state: &'a SelectState,
    pub items: &'a [SelectItem<'a>],
}

impl Select<'_> {
    pub fn paint(&self, p: &mut dyn Painter, anchor: Rect, viewport: Rect, t: &Tokens) {
        if !self.state.open {
            return;
        }

        let popup = Self::popup_rect(anchor, viewport, self.items.len(), t);
        let row_h = t.density.row_height();
        let font_size = t.density.font_size();
        let first = self.state.scroll.offset.floor().max(0.0) as usize;
        let visible = self.items.len().saturating_sub(first).min(MAX_VISIBLE_ROWS);

        p.fill_round_rect(popup, 6.0, t.popover);
        p.stroke_round_rect(popup, 6.0, t.border, 1.0);
        p.save();
        p.clip_rect(popup);

        for row in 0..visible {
            let index = first + row;
            let item = &self.items[index];
            let row_rect = Rect::xywh(
                popup.origin.x,
                popup.origin.y + row as f32 * row_h,
                popup.size.x,
                row_h,
            );
            if self.state.pressed == Some(index) {
                p.fill_rect(row_rect, t.button_hover.with_alpha(t.button_hover.a * 1.8));
            } else if self.state.hover == Some(index) {
                p.fill_rect(row_rect, t.button_hover);
            } else if item.selected {
                p.fill_rect(row_rect, t.row_selected_primary);
            }

            let color = if item.disabled {
                t.muted_foreground.with_alpha(0.5)
            } else {
                t.popover_foreground
            };
            let origin = Point2D::new(
                row_rect.origin.x + 10.0,
                row_rect.origin.y + (row_h - font_size) / 2.0,
            );
            let layout =
                TextLayout::single_run(item.label, FONT_FAMILY, font_size, color.to_jian(), origin);
            p.draw_text(&layout, origin);

            if item.selected {
                p.stroke_svg_path(
                    CHECK_D,
                    Point2D::new(
                        row_rect.origin.x + row_rect.size.x - 24.0,
                        row_rect.origin.y + 7.0,
                    ),
                    14.0,
                    t.primary,
                    1.75,
                );
            }
        }

        p.restore();
    }

    pub fn popup_rect(anchor: Rect, viewport: Rect, row_count: usize, t: &Tokens) -> Rect {
        let row_h = t.density.row_height();
        let rows = row_count.min(MAX_VISIBLE_ROWS);
        let height = rows as f32 * row_h;
        let width = anchor.size.x.max(160.0).min(viewport.size.x);
        let max_x = viewport.origin.x + viewport.size.x - width;
        let x = anchor
            .origin
            .x
            .clamp(viewport.origin.x, max_x.max(viewport.origin.x));
        let below = anchor.origin.y + anchor.size.y;
        let viewport_bottom = viewport.origin.y + viewport.size.y;
        let y = if below + height <= viewport_bottom {
            below
        } else if anchor.origin.y - height >= viewport.origin.y {
            anchor.origin.y - height
        } else {
            (viewport_bottom - height).max(viewport.origin.y)
        };
        Rect::xywh(x, y, width, height)
    }

    pub fn hit(
        state: &SelectState,
        anchor: Rect,
        viewport: Rect,
        row_count: usize,
        point: Point2D,
        t: &Tokens,
    ) -> SelectHit {
        if !state.open {
            return SelectHit::Outside;
        }
        let popup = Self::popup_rect(anchor, viewport, row_count, t);
        if !popup.contains(point) {
            return SelectHit::Outside;
        }
        let row_h = t.density.row_height();
        if row_h <= 0.0 {
            return SelectHit::Inside;
        }
        let local_row = ((point.y - popup.origin.y) / row_h).floor().max(0.0) as usize;
        let row = state.scroll.offset.floor().max(0.0) as usize + local_row;
        if row < row_count {
            SelectHit::Row(row)
        } else {
            SelectHit::Inside
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_flips_above_anchor_when_bottom_would_overflow() {
        let t = Tokens::dark();
        let anchor = Rect::xywh(20.0, 180.0, 100.0, 20.0);
        let viewport = Rect::xywh(0.0, 0.0, 300.0, 220.0);

        let popup = Select::popup_rect(anchor, viewport, 4, &t);

        assert!(popup.origin.y + popup.size.y <= anchor.origin.y);
        assert_eq!(popup.size.y, 4.0 * t.density.row_height());
    }

    #[test]
    fn hit_returns_row_inside_or_outside() {
        let t = Tokens::dark();
        let anchor = Rect::xywh(20.0, 20.0, 100.0, 20.0);
        let viewport = Rect::xywh(0.0, 0.0, 300.0, 220.0);
        let state = SelectState {
            open: true,
            ..SelectState::default()
        };
        let popup = Select::popup_rect(anchor, viewport, 2, &t);

        assert_eq!(
            Select::hit(
                &state,
                anchor,
                viewport,
                2,
                Point2D::new(popup.origin.x + 4.0, popup.origin.y + 5.0),
                &t,
            ),
            SelectHit::Row(0)
        );
        assert_eq!(
            Select::hit(
                &state,
                anchor,
                viewport,
                2,
                Point2D::new(popup.origin.x + 4.0, popup.origin.y + popup.size.y + 1.0),
                &t,
            ),
            SelectHit::Outside
        );
    }

    #[test]
    fn keyboard_reveals_hover_past_visible_window() {
        let mut state = SelectState {
            open: true,
            hover: Some(7),
            ..SelectState::default()
        };

        state.apply_key(
            SelectKey {
                code: SelectKeyCode::ArrowDown,
                is_composing: false,
            },
            20,
        );

        assert_eq!(state.hover, Some(8));
        assert!(state.scroll.offset > 0.0);
    }

    #[test]
    fn composing_arrow_keys_do_not_move_hover() {
        let mut state = SelectState {
            open: true,
            hover: Some(3),
            ..SelectState::default()
        };

        state.apply_key(
            SelectKey {
                code: SelectKeyCode::ArrowDown,
                is_composing: true,
            },
            10,
        );

        assert_eq!(state.hover, Some(3));
        assert_eq!(state.scroll.offset, 0.0);
    }

    #[test]
    fn enter_returns_hovered_row() {
        let mut state = SelectState {
            open: true,
            hover: Some(2),
            ..SelectState::default()
        };

        assert_eq!(
            state.apply_key(
                SelectKey {
                    code: SelectKeyCode::Enter,
                    is_composing: false,
                },
                5,
            ),
            Some(2)
        );
    }
}
