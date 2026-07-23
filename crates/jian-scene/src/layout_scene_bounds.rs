//! Paint-visible scene bounds kept separate from selection/layout bounds.

use crate::layout_scene::SceneNode;
use crate::scene_geometry::rect_has_extent;
use jian_widgets::Rect;

impl SceneNode {
    /// Bounds of paint that can escape this node for viewport framing.
    ///
    /// Selection and layout intentionally keep using `aggregate_bounds`: a
    /// bounded frame reports its authored rectangle. Visual bounds differ only
    /// for open containers, whose descendants may paint beyond that rectangle.
    /// A clipping container stops the walk at its own bounds.
    pub fn visual_bounds(&self) -> Rect {
        if self.hidden {
            return Rect::ZERO;
        }
        if self.clip_content {
            return self.bounds;
        }

        let mut bounds = rect_has_extent(self.bounds).then_some(self.bounds);
        for child in &self.children {
            let child_bounds = child.visual_bounds();
            if !rect_has_extent(child_bounds) {
                continue;
            }
            bounds = Some(match bounds {
                Some(current) => union_rects(current, child_bounds),
                None => child_bounds,
            });
        }
        bounds.unwrap_or(Rect::ZERO)
    }
}

fn union_rects(a: Rect, b: Rect) -> Rect {
    let min_x = a.origin.x.min(b.origin.x);
    let min_y = a.origin.y.min(b.origin.y);
    let max_x = (a.origin.x + a.size.x).max(b.origin.x + b.size.x);
    let max_y = (a.origin.y + a.size.y).max(b.origin.y + b.size.y);
    Rect::xywh(min_x, min_y, max_x - min_x, max_y - min_y)
}
