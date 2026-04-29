//! Arena priority — when no recognizer claims, the arena uses this tuple
//! (depth, kind-priority) descending to pick a winner on pointer-up.

use super::recognizer::Recognizer;
use crate::document::RuntimeDocument;

/// `(depth, kind)` — higher wins.
pub fn rank(r: &dyn Recognizer, doc: &RuntimeDocument) -> (u32, u32) {
    let depth = depth_of(doc, r.node());
    let kind_priority = match r.kind() {
        "Pan" | "Scroll" => 5,
        "Scale" | "Rotate" => 4,
        "LongPress" => 3,
        "Tap" | "DoubleTap" => 2,
        "Hover" => 1,
        _ => 0,
    };
    (depth, kind_priority)
}

/// Distance from `key` to the document root. Cycle-bounded at the
/// tree's node count: a longer chain implies a parent cycle (which
/// shouldn't exist in a healthy `NodeTree` but `NodeData.parent` is
/// `pub`, so a buggy mutation could install one). Bailing out
/// returns the partial depth — better than hanging the arena
/// ranking on every pointer-up.
fn depth_of(doc: &RuntimeDocument, key: crate::document::NodeKey) -> u32 {
    let mut d: u32 = 0;
    let mut cur = key;
    let max_steps = doc.tree.nodes.len() as u32;
    while let Some(p) = doc.tree.nodes[cur].parent {
        if d > max_steps {
            break;
        }
        d += 1;
        cur = p;
    }
    d
}
