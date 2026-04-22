//! LayoutEngine — wraps `taffy::TaffyTree` and maps SlotMap keys ↔ taffy NodeIds.

pub mod measure;
pub mod resolve;

use crate::document::{NodeKey, NodeTree};
use crate::error::{CoreError, CoreResult};
use crate::geometry::{rect, Rect};
use slotmap::SecondaryMap;
use taffy::prelude::*;

pub struct LayoutEngine {
    pub(crate) tree: TaffyTree<()>,
    pub(crate) map: SecondaryMap<NodeKey, NodeId>,
    /// Parent-node lookup, mirrored from `NodeTree` so `node_rect` can
    /// accumulate per-parent offsets into an absolute scene coordinate.
    pub(crate) parent: SecondaryMap<NodeKey, NodeKey>,
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self {
            tree: TaffyTree::new(),
            map: SecondaryMap::new(),
            parent: SecondaryMap::new(),
        }
    }

    /// Build a taffy tree mirroring the NodeTree. Returns the root NodeIds.
    pub fn build(&mut self, doc_tree: &NodeTree) -> CoreResult<Vec<NodeId>> {
        self.tree = TaffyTree::new();
        self.map = SecondaryMap::new();
        self.parent = SecondaryMap::new();

        // Pass 1: create a taffy node for each doc node. `node_to_style`
        // handles both containers (Frame/Group/Rectangle) and leaves
        // (Text / IconFont / Image / …) so leaf sizes propagate into
        // flex measurements.
        for (key, data) in doc_tree.nodes.iter() {
            let style = resolve::node_to_style(&data.schema);
            let id = self
                .tree
                .new_leaf(style)
                .map_err(|e| CoreError::Layout(e.to_string()))?;
            self.map.insert(key, id);
            if let Some(p) = data.parent {
                self.parent.insert(key, p);
            }
        }

        // Pass 2: wire parent/child relationships.
        for (key, data) in doc_tree.nodes.iter() {
            if !data.children.is_empty() {
                let parent = self.map[key];
                let child_ids: Vec<NodeId> = data.children.iter().map(|k| self.map[*k]).collect();
                self.tree
                    .set_children(parent, &child_ids)
                    .map_err(|e| CoreError::Layout(e.to_string()))?;
            }
        }

        Ok(doc_tree.roots.iter().map(|k| self.map[*k]).collect())
    }

    pub fn compute(&mut self, root: NodeId, available: (f32, f32)) -> CoreResult<()> {
        let space = Size {
            width: AvailableSpace::Definite(available.0),
            height: AvailableSpace::Definite(available.1),
        };
        self.tree
            .compute_layout(root, space)
            .map_err(|e| CoreError::Layout(e.to_string()))
    }

    /// Absolute scene-coord rect for `key`: taffy's `layout.location` is
    /// relative to the node's flex parent, so we walk up the parent
    /// chain and accumulate each ancestor's location offset.
    pub fn node_rect(&self, key: NodeKey) -> Option<Rect> {
        let id = self.map.get(key)?;
        let l = self.tree.layout(*id).ok()?;
        let (mut ax, mut ay) = (l.location.x, l.location.y);
        let (w, h) = (l.size.width, l.size.height);
        let mut cur = key;
        while let Some(&p) = self.parent.get(cur) {
            let pid = self.map.get(p)?;
            let pl = self.tree.layout(*pid).ok()?;
            ax += pl.location.x;
            ay += pl.location.y;
            cur = p;
        }
        Some(rect(ax, ay, w, h))
    }
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}
