//! The part of the framed root a host actually shows, for in-view triggers.
//!
//! A page-scrolling root grows to its content height and the host scrolls it
//! by painting at an offset, so the root's own bounds contain every child.
//! In-view observation therefore needs the host's visible window: the scroll
//! offset, the strip a pinned status bar covers, and the visible height
//! between pinned strips. Pinned nodes never scroll, so they always count as
//! in view. Moved from OpenPencil's preview, where the window comes from its
//! device frame.

use super::scene_view::MotionSceneView;
use crate::geometry::{rect, Rect};

/// The visible window, relative to the framed root so it stays valid when a
/// screen switch replaces the root.
#[derive(Debug, Clone, PartialEq)]
pub struct InViewWindow {
    scroll_y: f32,
    top_inset: f32,
    height: f32,
    pinned: Vec<String>,
}

impl InViewWindow {
    /// `pinned` lists nodes that never scroll; empty ids are ignored.
    pub fn new(scroll_y: f32, top_inset: f32, height: f32, pinned: Vec<String>) -> Self {
        Self {
            scroll_y: scroll_y.max(0.0),
            top_inset,
            height: height.max(0.0),
            pinned: pinned.into_iter().filter(|id| !id.is_empty()).collect(),
        }
    }

    /// The visible window in scene space for a framed root at `root`.
    pub fn rect_for_root(&self, root: Rect) -> Rect {
        rect(
            root.origin.x,
            root.origin.y + self.scroll_y + self.top_inset,
            root.size.width,
            self.height,
        )
    }

    /// Whether `target` is a pinned node or sits inside one.
    pub fn is_pinned(&self, view: &dyn MotionSceneView, target: &str) -> bool {
        self.pinned
            .iter()
            .any(|id| view.subtree_contains(id, target))
    }
}

/// A node counts as in view once a tenth of its height shows.
pub fn intersects_viewport(node: Rect, viewport: Rect) -> bool {
    if node.size.height <= 0.0 || node.size.width <= 0.0 {
        return false;
    }
    let left = node.origin.x.max(viewport.origin.x);
    let right = (node.origin.x + node.size.width).min(viewport.origin.x + viewport.size.width);
    let top = node.origin.y.max(viewport.origin.y);
    let bottom = (node.origin.y + node.size.height).min(viewport.origin.y + viewport.size.height);
    right > left && bottom > top && (bottom - top) / node.size.height >= 0.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::services::AnimationProperty;
    use serde_json::Value;

    struct Tree;

    impl MotionSceneView for Tree {
        fn node_rect(&self, _: &str) -> Option<Rect> {
            None
        }
        fn property(&self, _: &str, _: &AnimationProperty) -> Option<Value> {
            None
        }
        fn root_rect(&self) -> Option<Rect> {
            None
        }
        fn switches(&self) -> Vec<(String, bool)> {
            Vec::new()
        }
        fn subtree_contains(&self, ancestor: &str, id: &str) -> bool {
            ancestor == id || (ancestor == "nav" && id == "nav-icon")
        }
    }

    #[test]
    fn the_window_follows_scroll_and_inset_below_the_root_top() {
        let window = InViewWindow::new(300.0, 44.0, 700.0, vec!["nav".into(), String::new()]);
        let visible = window.rect_for_root(rect(10.0, 20.0, 390.0, 1600.0));
        assert_eq!(visible, rect(10.0, 364.0, 390.0, 700.0));
        assert_eq!(
            InViewWindow::new(-5.0, 0.0, -1.0, Vec::new()).rect_for_root(rect(0.0, 0.0, 1.0, 1.0)),
            rect(0.0, 0.0, 1.0, 0.0)
        );
    }

    #[test]
    fn pinned_nodes_and_their_children_are_always_visible() {
        let window = InViewWindow::new(0.0, 0.0, 100.0, vec!["nav".into()]);
        assert!(window.is_pinned(&Tree, "nav"));
        assert!(window.is_pinned(&Tree, "nav-icon"));
        assert!(!window.is_pinned(&Tree, "card"));
    }

    #[test]
    fn a_tenth_of_the_height_must_show() {
        let viewport = rect(0.0, 0.0, 100.0, 100.0);
        assert!(intersects_viewport(rect(0.0, 95.0, 50.0, 50.0), viewport));
        assert!(!intersects_viewport(rect(0.0, 96.0, 50.0, 50.0), viewport));
        assert!(!intersects_viewport(rect(0.0, 10.0, 0.0, 10.0), viewport));
        assert!(!intersects_viewport(
            rect(200.0, 10.0, 10.0, 10.0),
            viewport
        ));
    }
}
