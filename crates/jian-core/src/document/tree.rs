//! Tree storage separate from the schema types.
//!
//! `jian-ops-schema::PenNode` owns its children by value (recursive). For the
//! runtime we want O(1) parent lookup, stable ids that survive restructuring,
//! and the ability to store extra per-node runtime data (dirty flags, etc).
//!
//! Strategy: copy the schema tree into a SlotMap, and keep two maps:
//!   node_id (string)  → SlotKey
//!   SlotKey           → { parent, children, kind, schema_ref }

use jian_ops_schema::node::PenNode;
use slotmap::{new_key_type, SlotMap};
use std::collections::BTreeMap;

new_key_type! { pub struct NodeKey; }

pub struct NodeData {
    /// The schema-level representation (borrowed reference is not possible; we clone).
    pub schema: PenNode,
    /// Parent in the runtime tree; None for document root children.
    pub parent: Option<NodeKey>,
    /// Ordered children in render order.
    pub children: Vec<NodeKey>,
}

pub struct NodeTree {
    pub nodes: SlotMap<NodeKey, NodeData>,
    pub by_id: BTreeMap<String, NodeKey>,
    pub roots: Vec<NodeKey>, // document.children or active-page children
}

impl NodeTree {
    pub fn new() -> Self {
        Self {
            nodes: SlotMap::with_key(),
            by_id: BTreeMap::new(),
            roots: Vec::new(),
        }
    }

    /// Insert a schema node tree, returning the key of the inserted root.
    pub fn insert_subtree(&mut self, node: PenNode, parent: Option<NodeKey>) -> NodeKey {
        let id = node_schema_id(&node).to_owned();
        let children_schema = take_children(&node);
        let key = self.nodes.insert(NodeData {
            schema: node,
            parent,
            children: Vec::new(),
        });
        self.by_id.insert(id, key);
        if parent.is_none() {
            self.roots.push(key);
        } else if let Some(p) = parent {
            self.nodes[p].children.push(key);
        }
        for child in children_schema {
            self.insert_subtree(child, Some(key));
        }
        key
    }

    pub fn get(&self, id: &str) -> Option<NodeKey> {
        self.by_id.get(id).copied()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl Default for NodeTree {
    fn default() -> Self {
        Self::new()
    }
}

/// Read the node's stable id from any variant.
pub fn node_schema_id(n: &PenNode) -> &str {
    match n {
        PenNode::Frame(x) => &x.base.id,
        PenNode::Group(x) => &x.base.id,
        PenNode::Rectangle(x) => &x.base.id,
        PenNode::Ellipse(x) => &x.base.id,
        PenNode::Line(x) => &x.base.id,
        PenNode::Polygon(x) => &x.base.id,
        PenNode::Path(x) => &x.base.id,
        PenNode::Text(x) => &x.base.id,
        PenNode::TextInput(x) => &x.base.id,
        PenNode::Image(x) => &x.base.id,
        PenNode::IconFont(x) => &x.base.id,
        PenNode::Ref(x) => &x.base.id,
    }
}

/// Extract the children collection from a container-style node, cloning.
fn take_children(n: &PenNode) -> Vec<PenNode> {
    match n {
        PenNode::Frame(x) => x.children.clone().unwrap_or_default(),
        PenNode::Group(x) => x.children.clone().unwrap_or_default(),
        PenNode::Rectangle(x) => x.children.clone().unwrap_or_default(),
        PenNode::Ref(x) => x.children.clone().unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rect(id: &str) -> PenNode {
        serde_json::from_value(json!({
            "type":"rectangle","id":id
        }))
        .unwrap()
    }

    fn frame(id: &str, children: Vec<PenNode>) -> PenNode {
        let mut v = json!({"type":"frame","id":id});
        v["children"] = serde_json::Value::Array(
            children
                .into_iter()
                .map(|c| serde_json::to_value(c).unwrap())
                .collect(),
        );
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn empty() {
        let t = NodeTree::new();
        assert!(t.is_empty());
    }

    #[test]
    fn insert_leaf() {
        let mut t = NodeTree::new();
        let k = t.insert_subtree(rect("r1"), None);
        assert_eq!(t.len(), 1);
        assert_eq!(t.get("r1"), Some(k));
        assert_eq!(t.roots, vec![k]);
    }

    #[test]
    fn insert_frame_with_children() {
        let mut t = NodeTree::new();
        let f = frame("f1", vec![rect("r1"), rect("r2")]);
        let root = t.insert_subtree(f, None);
        assert_eq!(t.len(), 3);
        assert_eq!(t.nodes[root].children.len(), 2);
        let r1 = t.get("r1").unwrap();
        let r2 = t.get("r2").unwrap();
        assert_eq!(t.nodes[r1].parent, Some(root));
        assert_eq!(t.nodes[r2].parent, Some(root));
    }
}
