//! Scene graph — the **render-ready** view of the document.
//!
//! Each `SceneNode` corresponds to a `NodeKey` in the document tree and adds
//! resolved layout (position, size in scene coords) and resolved visual
//! properties. Produced from the runtime document + computed layout; consumed
//! by the `RenderBackend` trait.

pub mod dirty;
pub mod properties;

pub use dirty::DirtyFlags;
pub use properties::{Color, ResolvedVisual};

use crate::document::NodeKey;
use crate::geometry::Rect;
use slotmap::SecondaryMap;

#[derive(Debug, Clone, Default)]
pub struct SceneNode {
    pub bounds: Rect,
    pub visual: ResolvedVisual,
    pub visible: bool,
    pub dirty: DirtyFlags,
}

pub struct SceneGraph {
    pub nodes: SecondaryMap<NodeKey, SceneNode>,
}

impl SceneGraph {
    pub fn new() -> Self {
        Self {
            nodes: SecondaryMap::new(),
        }
    }
}

impl Default for SceneGraph {
    fn default() -> Self {
        Self::new()
    }
}
