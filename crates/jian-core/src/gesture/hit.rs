//! HitTester over SpatialIndex. Returns a z-ordered path from topmost node
//! upward through ancestors to root.

use crate::document::{NodeKey, RuntimeDocument};
use crate::geometry::Point;
use crate::spatial::SpatialIndex;
use std::collections::HashMap;

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
///
/// Tiebreaker for siblings at the same depth: paint-order index. The
/// document tree is walked depth-first from each root in render order
/// (`NodeData.children` is already in render order), and each node
/// gets a sequential index. Higher index = painted later = on top, so
/// we sort `(depth desc, paint_index desc)` to get a deterministic
/// topmost regardless of spatial-index iteration order.
pub fn hit_test(spatial: &SpatialIndex, doc: &RuntimeDocument, p: Point) -> HitPath {
    let mut candidates = spatial.hit(p);
    if candidates.is_empty() {
        return HitPath::default();
    }
    let paint = paint_index_table(doc);
    // Sort: deeper first, then later in paint order first.
    candidates.sort_by(|a, b| {
        let da = depth(doc, *a);
        let db = depth(doc, *b);
        db.cmp(&da).then_with(|| {
            let pa = paint.get(a).copied().unwrap_or(0);
            let pb = paint.get(b).copied().unwrap_or(0);
            pb.cmp(&pa)
        })
    });

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

/// Walk the tree depth-first in render order and assign each node a
/// sequential paint index. Used as the same-depth tiebreaker so two
/// overlapping siblings hit-test deterministically (later child wins).
fn paint_index_table(doc: &RuntimeDocument) -> HashMap<NodeKey, u32> {
    let mut idx = 0u32;
    let mut out: HashMap<NodeKey, u32> = HashMap::new();
    for &root in &doc.tree.roots {
        walk_paint(doc, root, &mut idx, &mut out);
    }
    out
}

fn walk_paint(
    doc: &RuntimeDocument,
    key: NodeKey,
    idx: &mut u32,
    out: &mut HashMap<NodeKey, u32>,
) {
    out.insert(key, *idx);
    *idx += 1;
    for &child in &doc.tree.nodes[key].children {
        walk_paint(doc, child, idx, out);
    }
}
