//! The read-only view of a painted scene that motion triggers work against.
//!
//! OpenPencil paints a `jian_scene::LayoutScene`; a Jian host paints from its
//! runtime document and layout. Both implement this trait, so the trigger
//! logic (mount, in-view, transitions, the switch tween) is written once.

use crate::action::services::AnimationProperty;
use crate::geometry::Rect;
use serde_json::Value;

pub trait MotionSceneView {
    /// The node's painted bounds; `None` when it is not on the active page.
    fn node_rect(&self, id: &str) -> Option<Rect>;

    /// The node's current value for an animatable property, in the
    /// timeline's value form: numbers, degrees for rotation, `#rrggbb[aa]`
    /// colours, 0 for translations and 1 for scales (the painted base).
    fn property(&self, id: &str, property: &AnimationProperty) -> Option<Value>;

    /// Bounds of the framed root, the active page's first node.
    fn root_rect(&self) -> Option<Rect>;

    /// Every switch widget on the active page with its checked state.
    fn switches(&self) -> Vec<(String, bool)>;

    /// Whether `id` is `ancestor` itself or one of its descendants.
    fn subtree_contains(&self, ancestor: &str, id: &str) -> bool;
}
