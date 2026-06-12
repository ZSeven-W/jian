use crate::{Painter, Point2D, Rect, Tokens};

pub const MIN_THUMB_LEN: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThumbDrag {
    pub grab_y: f32,
}

pub struct ScrollArea<'a> {
    pub state: &'a mut jian_core::scroll::ScrollState,
    pub content_h: f32,
    pub view_h: f32,
    pub hovered: bool,
    pub drag: Option<ThumbDrag>,
}

impl ScrollArea<'_> {
    pub fn paint_scrollbar(&self, p: &mut dyn Painter, track_rect: Rect, t: &Tokens) {
        if let Some(thumb) = self.thumb_rect(track_rect) {
            let width = if self.hovered { 4.0 } else { 2.0 };
            let rect = Rect::xywh(
                track_rect.origin.x + track_rect.size.x - width,
                thumb.origin.y,
                width,
                thumb.size.y,
            );
            p.fill_round_rect(rect, 1.0, t.muted_foreground.with_alpha(0.55));
        }
    }

    pub fn wheel(&mut self, delta: f32) {
        self.state.scroll_by(delta, self.content_h, self.view_h);
    }

    pub fn thumb_hit(&self, track_rect: Rect, point: Point2D) -> bool {
        self.thumb_rect(track_rect)
            .is_some_and(|thumb| thumb.contains(point))
    }

    pub fn begin_thumb_drag(&mut self, track_rect: Rect, point: Point2D) {
        self.drag = self.thumb_rect(track_rect).and_then(|thumb| {
            thumb.contains(point).then_some(ThumbDrag {
                grab_y: point.y - thumb.origin.y,
            })
        });
    }

    pub fn drag_to(&mut self, track_rect: Rect, point: Point2D) {
        if let Some(drag) = self.drag {
            let thumb_top = point.y - track_rect.origin.y - drag.grab_y;
            self.state.set_from_thumb(
                thumb_top,
                track_rect.size.y,
                self.content_h,
                self.view_h,
                MIN_THUMB_LEN,
            );
        }
    }

    pub fn fling(&mut self, _velocity: f32) {}

    fn thumb_rect(&self, track_rect: Rect) -> Option<Rect> {
        self.state
            .thumb(
                track_rect.size.y,
                self.content_h,
                self.view_h,
                MIN_THUMB_LEN,
            )
            .map(|thumb| {
                Rect::xywh(
                    track_rect.origin.x,
                    track_rect.origin.y + thumb.offset,
                    track_rect.size.x,
                    thumb.len,
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn wheel_clamps_offset_to_content_bounds() {
        let mut state = jian_core::scroll::ScrollState::default();
        let mut area = ScrollArea {
            state: &mut state,
            content_h: 300.0,
            view_h: 100.0,
            hovered: false,
            drag: None,
        };

        area.wheel(500.0);
        assert_eq!(area.state.offset, 200.0);
        area.wheel(-500.0);
        assert_eq!(area.state.offset, 0.0);
    }

    #[test]
    fn hovered_scrollbar_paints_wider_thumb() {
        let mut state = jian_core::scroll::ScrollState { offset: 50.0 };
        let area = ScrollArea {
            state: &mut state,
            content_h: 300.0,
            view_h: 100.0,
            hovered: true,
            drag: None,
        };
        let t = Tokens::dark();
        let mut p = CapturePainter::default();

        area.paint_scrollbar(&mut p, Rect::xywh(10.0, 20.0, 8.0, 100.0), &t);

        assert!(p.ops.iter().any(|op| {
            matches!(op, PaintOp::FillRoundRect(rect, 1.0, _) if rect.size.x == 4.0)
        }));
    }

    #[test]
    fn thumb_drag_updates_scroll_offset() {
        let mut state = jian_core::scroll::ScrollState::default();
        let mut area = ScrollArea {
            state: &mut state,
            content_h: 300.0,
            view_h: 100.0,
            hovered: false,
            drag: None,
        };
        let track = Rect::xywh(0.0, 0.0, 8.0, 100.0);

        area.begin_thumb_drag(track, Point2D::new(4.0, 10.0));
        area.drag_to(track, Point2D::new(4.0, 80.0));

        assert!(area.state.offset > 150.0);
    }
}
