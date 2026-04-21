//! rawPointer escape hatch.
//!
//! When a node (or any ancestor on the hit path) declares
//! `gestures.rawPointer: true`, its subtree opts out of arena arbitration:
//! Down/Move/Up events are delivered as `SemanticEvent::RawPointer` directly
//! to the rawPointer root, and no child recognizers claim.

use super::hit::HitPath;
use crate::document::{NodeKey, RuntimeDocument};

/// Walk a hit path from topmost to root; return the first node whose schema
/// opts in to rawPointer mode, or `None` if no ancestor opts in.
pub fn find_raw_root(path: &HitPath, doc: &RuntimeDocument) -> Option<NodeKey> {
    path.0
        .iter()
        .find(|&&key| node_opts_in(doc, key))
        .copied()
}

fn node_opts_in(doc: &RuntimeDocument, key: NodeKey) -> bool {
    let schema = &doc.tree.nodes[key].schema;
    let Ok(v) = serde_json::to_value(schema) else {
        return false;
    };
    v.as_object()
        .and_then(|o| o.get("gestures"))
        .and_then(|g| g.as_object())
        .and_then(|g| g.get("rawPointer"))
        .and_then(|b| b.as_bool())
        .unwrap_or(false)
}
