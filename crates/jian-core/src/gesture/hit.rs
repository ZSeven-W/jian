//! HitTester over SpatialIndex. Returns a z-ordered path from topmost node
//! upward through ancestors to root.

use crate::document::{NodeKey, RuntimeDocument};
use crate::geometry::Point;
use crate::spatial::SpatialIndex;

#[derive(Debug, Clone, Default)]
pub struct HitPath(pub Vec<NodeKey>);

impl HitPath {
    pub fn topmost(&self) -> Option<NodeKey> {
        self.0.first().copied()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn contains(&self, key: NodeKey) -> bool {
        self.0.contains(&key)
    }
}

/// Hit-test at `p`; return the path from topmost (deepest) node up through
/// ancestors to root. Index 0 is the highest-in-z node.
pub fn hit_test(spatial: &SpatialIndex, doc: &RuntimeDocument, p: Point) -> HitPath {
    let mut candidates = spatial.hit(p);
    if candidates.is_empty() {
        return HitPath::default();
    }
    // Deepest wins: count depth via parent chain.
    candidates.sort_by_key(|k| std::cmp::Reverse(depth(doc, *k)));

    // Start from the top candidate and walk up its ancestor chain so that
    // all enclosing parents are also in the path (event bubbling).
    let top = candidates[0];
    let mut path = vec![top];
    let mut cur = top;
    while let Some(p) = doc.tree.nodes[cur].parent {
        path.push(p);
        cur = p;
    }
    HitPath(path)
}

fn depth(doc: &RuntimeDocument, key: NodeKey) -> u32 {
    let mut d = 0;
    let mut cur = key;
    while let Some(p) = doc.tree.nodes[cur].parent {
        d += 1;
        cur = p;
    }
    d
}
