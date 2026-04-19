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
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self {
            tree: TaffyTree::new(),
            map: SecondaryMap::new(),
        }
    }

    /// Build a taffy tree mirroring the NodeTree. Returns the root NodeIds.
    pub fn build(&mut self, doc_tree: &NodeTree) -> CoreResult<Vec<NodeId>> {
        use jian_ops_schema::node::PenNode;
        self.tree = TaffyTree::new();
        self.map = SecondaryMap::new();

        // Pass 1: create a taffy node for each doc node.
        for (key, data) in doc_tree.nodes.iter() {
            let style = match &data.schema {
                PenNode::Frame(f) => resolve::container_to_style(&f.container),
                PenNode::Group(g) => resolve::container_to_style(&g.container),
                PenNode::Rectangle(r) => resolve::container_to_style(&r.container),
                _ => Style {
                    size: Size {
                        width: auto(),
                        height: auto(),
                    },
                    ..Default::default()
                },
            };
            let id = self
                .tree
                .new_leaf(style)
                .map_err(|e| CoreError::Layout(e.to_string()))?;
            self.map.insert(key, id);
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

    pub fn node_rect(&self, key: NodeKey) -> Option<Rect> {
        let id = self.map.get(key)?;
        let l = self.tree.layout(*id).ok()?;
        Some(rect(
            l.location.x,
            l.location.y,
            l.size.width,
            l.size.height,
        ))
    }
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}
