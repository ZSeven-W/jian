use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

pub const MAX_VISIBLE_ROWS: usize = 8;
const FONT_FAMILY: &str = "Inter";
const CHECK_D: &str = "M20 6 9 17l-5-5";

#[derive(Debug, Clone, Default, PartialEq)]
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
    pub fn apply_key(&mut self, key: SelectKey, items: &[SelectItem<'_>]) -> Option<usize> {
        self.apply_key_with_row_height(key, items, crate::Density::Desktop.row_height())
    }

    pub fn apply_key_with_row_height(
        &mut self,
        key: SelectKey,
        items: &[SelectItem<'_>],
        row_height: f32,
    ) -> Option<usize> {
        if key.is_composing {
            return None;
        }

        let row_h = resolved_row_height(row_height);
        let item_count = items.len();
        if item_count == 0 {
            self.hover = None;
            self.scroll.offset = 0.0;
            if key.code == SelectKeyCode::Escape {
                self.open = false;
            }
            return None;
        }

        let last = item_count - 1;
        match key.code {
            SelectKeyCode::ArrowUp => {
                let current = self.hover.map_or(0, |i| i.min(last));
                let start = current.saturating_sub(1);
                if let Some(next) =
                    previous_enabled(items, start).or_else(|| next_enabled(items, current))
                {
                    self.set_hover(next, row_h);
                } else {
                    self.clear_hover();
                }
            }
            SelectKeyCode::ArrowDown => {
                let start = self.hover.map_or(0, |i| i.saturating_add(1).min(last));
                if let Some(next) =
                    next_enabled(items, start).or_else(|| previous_enabled(items, start))
                {
                    self.set_hover(next, row_h);
                } else {
                    self.clear_hover();
                }
            }
            SelectKeyCode::Home => {
                if let Some(next) = next_enabled(items, 0) {
                    self.set_hover(next, row_h);
                } else {
                    self.clear_hover();
                }
            }
            SelectKeyCode::End => {
                if let Some(next) = previous_enabled(items, last) {
                    self.set_hover(next, row_h);
                } else {
                    self.clear_hover();
                }
            }
            SelectKeyCode::Enter => {
                return self
                    .hover
                    .filter(|i| items.get(*i).is_some_and(|item| !item.disabled));
            }
            SelectKeyCode::Escape => {
                self.open = false;
            }
        }
        None
    }

    fn set_hover(&mut self, index: usize, row_h: f32) {
        self.hover = Some(index);
        self.scroll.reveal(
            index as f32 * row_h,
            (index as f32 + 1.0) * row_h,
            MAX_VISIBLE_ROWS as f32 * row_h,
        );
    }

    fn clear_hover(&mut self) {
        self.hover = None;
        self.scroll.offset = 0.0;
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
        let visible_capacity = visible_row_capacity(self.items.len(), popup.size.y, row_h);
        let first = clamped_first_row(self.state, self.items.len(), visible_capacity, row_h);
        let visible = self.items.len().saturating_sub(first).min(visible_capacity);

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
            if !item.disabled && self.state.pressed == Some(index) {
                p.fill_rect(row_rect, t.button_hover.with_alpha(t.button_hover.a * 1.8));
            } else if !item.disabled && self.state.hover == Some(index) {
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
            let layout = TextLayout::single_run(
                item.label,
                FONT_FAMILY,
                font_size,
                color.to_jian(),
                Point2D::new(0.0, 0.0),
            );
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
        let height = (rows as f32 * row_h).min(viewport.size.y.max(0.0));
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
        let visible_capacity = visible_row_capacity(row_count, popup.size.y, row_h);
        if visible_capacity == 0 {
            return SelectHit::Inside;
        }
        let local_row = ((point.y - popup.origin.y) / row_h).floor().max(0.0) as usize;
        if local_row >= visible_capacity {
            return SelectHit::Inside;
        }
        let row = clamped_first_row(state, row_count, visible_capacity, row_h) + local_row;
        if row < row_count {
            SelectHit::Row(row)
        } else {
            SelectHit::Inside
        }
    }
}

fn visible_row_capacity(row_count: usize, popup_height: f32, row_h: f32) -> usize {
    if row_count == 0 || row_h <= 0.0 || popup_height <= 0.0 {
        return 0;
    }
    ((popup_height / row_h).ceil() as usize).clamp(1, row_count.min(MAX_VISIBLE_ROWS))
}

fn resolved_row_height(row_height: f32) -> f32 {
    if row_height.is_finite() && row_height > 0.0 {
        row_height
    } else {
        crate::Density::Desktop.row_height()
    }
}

fn clamped_first_row(
    state: &SelectState,
    row_count: usize,
    visible_capacity: usize,
    row_h: f32,
) -> usize {
    if row_count == 0 || visible_capacity == 0 || row_h <= 0.0 {
        return 0;
    }

    let max_first = row_count.saturating_sub(visible_capacity);
    let mut first = ((state.scroll.offset / row_h).floor().max(0.0) as usize).min(max_first);
    if let Some(hover) = state.hover.filter(|i| *i < row_count) {
        if hover < first {
            first = hover;
        } else if hover >= first + visible_capacity {
            first = hover + 1 - visible_capacity;
        }
    }
    first.min(max_first)
}

fn next_enabled(items: &[SelectItem<'_>], start: usize) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(i, item)| (!item.disabled).then_some(i))
}

fn previous_enabled(items: &[SelectItem<'_>], start: usize) -> Option<usize> {
    let end = start.min(items.len().saturating_sub(1));
    items
        .iter()
        .enumerate()
        .take(end + 1)
        .rev()
        .find_map(|(i, item)| (!item.disabled).then_some(i))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_items(count: usize) -> Vec<SelectItem<'static>> {
        (0..count)
            .map(|_| SelectItem {
                label: "",
                selected: false,
                disabled: false,
            })
            .collect()
    }

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
    fn popup_height_is_capped_to_viewport() {
        let t = Tokens::dark();
        let anchor = Rect::xywh(20.0, 20.0, 100.0, 20.0);
        let viewport = Rect::xywh(0.0, 0.0, 300.0, 60.0);

        let popup = Select::popup_rect(anchor, viewport, 20, &t);

        assert_eq!(popup.size.y, viewport.size.y);
        assert!(popup.origin.y >= viewport.origin.y);
        assert!(popup.origin.y + popup.size.y <= viewport.origin.y + viewport.size.y);
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
    fn hit_bottom_edge_does_not_return_offscreen_row() {
        let t = Tokens::dark();
        let anchor = Rect::xywh(20.0, 20.0, 100.0, 20.0);
        let viewport = Rect::xywh(0.0, 0.0, 300.0, 400.0);
        let state = SelectState {
            open: true,
            ..SelectState::default()
        };
        let row_count = MAX_VISIBLE_ROWS + 2;
        let popup = Select::popup_rect(anchor, viewport, row_count, &t);

        assert_eq!(
            Select::hit(
                &state,
                anchor,
                viewport,
                row_count,
                Point2D::new(popup.origin.x + 4.0, popup.origin.y + popup.size.y),
                &t,
            ),
            SelectHit::Inside
        );
    }

    #[test]
    fn paint_interprets_scroll_offset_as_pixels() {
        let t = Tokens::dark();
        let anchor = Rect::xywh(20.0, 20.0, 100.0, 20.0);
        let viewport = Rect::xywh(0.0, 0.0, 300.0, 400.0);
        let state = SelectState {
            open: true,
            scroll: jian_core::scroll::ScrollState {
                offset: t.density.row_height(),
            },
            ..SelectState::default()
        };
        let labels: Vec<_> = (0..20).map(|i| format!("Item {i}")).collect();
        let items: Vec<_> = labels
            .iter()
            .map(|label| SelectItem {
                label: label.as_str(),
                selected: false,
                disabled: false,
            })
            .collect();
        let select = Select {
            state: &state,
            items: &items,
        };
        let mut p = crate::test_support::CapturePainter::default();

        select.paint(&mut p, anchor, viewport, &t);

        let labels: Vec<_> = p
            .texts()
            .map(|(content, _, _)| content.to_owned())
            .collect();
        assert_eq!(labels.first().map(String::as_str), Some("Item 1"));
    }

    #[test]
    fn stale_scroll_offset_is_clamped_after_items_shrink() {
        let t = Tokens::dark();
        let anchor = Rect::xywh(20.0, 20.0, 100.0, 20.0);
        let viewport = Rect::xywh(0.0, 0.0, 300.0, 220.0);
        let state = SelectState {
            open: true,
            scroll: jian_core::scroll::ScrollState { offset: 20.0 },
            ..SelectState::default()
        };
        let items = [
            SelectItem {
                label: "One",
                selected: false,
                disabled: false,
            },
            SelectItem {
                label: "Two",
                selected: false,
                disabled: false,
            },
        ];
        let select = Select {
            state: &state,
            items: &items,
        };
        let mut p = crate::test_support::CapturePainter::default();

        select.paint(&mut p, anchor, viewport, &t);

        let labels: Vec<_> = p
            .texts()
            .map(|(content, _, _)| content.to_owned())
            .collect();
        assert_eq!(labels, vec!["One", "Two"]);

        let popup = Select::popup_rect(anchor, viewport, items.len(), &t);
        assert_eq!(
            Select::hit(
                &state,
                anchor,
                viewport,
                items.len(),
                Point2D::new(popup.origin.x + 4.0, popup.origin.y + 5.0),
                &t,
            ),
            SelectHit::Row(0)
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
            &enabled_items(20),
        );

        assert_eq!(state.hover, Some(8));
        assert_eq!(state.scroll.offset, crate::Density::Desktop.row_height());
    }

    #[test]
    fn arrow_up_clamps_stale_hover_after_items_shrink() {
        let mut state = SelectState {
            open: true,
            hover: Some(10),
            ..SelectState::default()
        };

        state.apply_key(
            SelectKey {
                code: SelectKeyCode::ArrowUp,
                is_composing: false,
            },
            &enabled_items(2),
        );

        assert_eq!(state.hover, Some(0));
    }

    #[test]
    fn escape_closes_empty_select() {
        let mut state = SelectState {
            open: true,
            ..SelectState::default()
        };

        state.apply_key(
            SelectKey {
                code: SelectKeyCode::Escape,
                is_composing: false,
            },
            &[],
        );

        assert!(!state.open);
        assert_eq!(state.hover, None);
    }

    #[test]
    fn keyboard_navigation_skips_disabled_options() {
        let items = [
            SelectItem {
                label: "One",
                selected: false,
                disabled: false,
            },
            SelectItem {
                label: "Two",
                selected: false,
                disabled: true,
            },
            SelectItem {
                label: "Three",
                selected: false,
                disabled: false,
            },
        ];
        let mut state = SelectState {
            open: true,
            hover: Some(0),
            ..SelectState::default()
        };

        state.apply_key(
            SelectKey {
                code: SelectKeyCode::ArrowDown,
                is_composing: false,
            },
            &items,
        );
        assert_eq!(state.hover, Some(2));
        assert_eq!(
            state.apply_key(
                SelectKey {
                    code: SelectKeyCode::Enter,
                    is_composing: false,
                },
                &items,
            ),
            Some(2)
        );

        state.hover = Some(1);
        assert_eq!(
            state.apply_key(
                SelectKey {
                    code: SelectKeyCode::Enter,
                    is_composing: false,
                },
                &items,
            ),
            None
        );
    }

    #[test]
    fn small_viewport_keeps_hovered_row_visible() {
        let t = Tokens::dark();
        let anchor = Rect::xywh(20.0, 20.0, 100.0, 20.0);
        let viewport = Rect::xywh(0.0, 0.0, 300.0, t.density.row_height() * 2.0);
        let state = SelectState {
            open: true,
            hover: Some(9),
            scroll: jian_core::scroll::ScrollState { offset: 2.0 },
            ..SelectState::default()
        };
        let item_labels: Vec<_> = (0..10).map(|i| format!("Item {i}")).collect();
        let items: Vec<_> = item_labels
            .iter()
            .map(|label| SelectItem {
                label,
                selected: false,
                disabled: false,
            })
            .collect();
        let select = Select {
            state: &state,
            items: &items,
        };
        let mut p = crate::test_support::CapturePainter::default();

        select.paint(&mut p, anchor, viewport, &t);

        let labels: Vec<_> = p
            .texts()
            .map(|(content, _, _)| content.to_owned())
            .collect();
        assert_eq!(labels, vec!["Item 8", "Item 9"]);
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
            &enabled_items(10),
        );

        assert_eq!(state.hover, Some(3));
        assert_eq!(state.scroll.offset, 0.0);
    }

    #[test]
    fn composing_enter_and_escape_do_not_select_or_close() {
        let mut state = SelectState {
            open: true,
            hover: Some(3),
            ..SelectState::default()
        };

        assert_eq!(
            state.apply_key(
                SelectKey {
                    code: SelectKeyCode::Enter,
                    is_composing: true,
                },
                &enabled_items(10),
            ),
            None
        );
        assert!(state.open);
        assert_eq!(state.hover, Some(3));

        state.apply_key(
            SelectKey {
                code: SelectKeyCode::Escape,
                is_composing: true,
            },
            &enabled_items(10),
        );

        assert!(state.open);
    }

    #[test]
    fn disabled_hovered_row_does_not_paint_active_background() {
        let t = Tokens::dark();
        let anchor = Rect::xywh(20.0, 20.0, 100.0, 20.0);
        let viewport = Rect::xywh(0.0, 0.0, 300.0, 220.0);
        let state = SelectState {
            open: true,
            hover: Some(0),
            pressed: Some(0),
            ..SelectState::default()
        };
        let items = [SelectItem {
            label: "Disabled",
            selected: false,
            disabled: true,
        }];
        let select = Select {
            state: &state,
            items: &items,
        };
        let mut p = crate::test_support::CapturePainter::default();

        select.paint(&mut p, anchor, viewport, &t);

        assert_eq!(p.fills_with(t.button_hover), 0);
        assert_eq!(
            p.fills_with(t.button_hover.with_alpha(t.button_hover.a * 1.8)),
            0
        );
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
                &enabled_items(5),
            ),
            Some(2)
        );
    }
}
