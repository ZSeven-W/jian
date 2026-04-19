//! Spatial index over SceneNodes for hit testing and viewport culling.

use crate::document::NodeKey;
use crate::geometry::{Point, Rect};
use rstar::{RTree, RTreeObject, AABB};

#[derive(Debug, Clone)]
pub struct NodeBBox {
    pub key: NodeKey,
    pub rect: Rect,
}

impl PartialEq for NodeBBox {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl RTreeObject for NodeBBox {
    type Envelope = AABB<[f32; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            [self.rect.min_x(), self.rect.min_y()],
            [self.rect.max_x(), self.rect.max_y()],
        )
    }
}

pub struct SpatialIndex {
    tree: RTree<NodeBBox>,
}

impl SpatialIndex {
    pub fn new() -> Self {
        Self { tree: RTree::new() }
    }

    pub fn rebuild<I: IntoIterator<Item = NodeBBox>>(&mut self, items: I) {
        self.tree = RTree::bulk_load(items.into_iter().collect());
    }

    pub fn insert(&mut self, bbox: NodeBBox) {
        self.tree.insert(bbox)
    }
    pub fn len(&self) -> usize {
        self.tree.size()
    }
    pub fn is_empty(&self) -> bool {
        self.tree.size() == 0
    }

    /// Returns all nodes whose bbox contains the point.
    pub fn hit(&self, p: Point) -> Vec<NodeKey> {
        let env = AABB::from_point([p.x, p.y]);
        self.tree
            .locate_in_envelope_intersecting(&env)
            .map(|b| b.key)
            .collect()
    }

    /// Returns all nodes whose bbox intersects the rect.
    pub fn query_rect(&self, r: Rect) -> Vec<NodeKey> {
        let envelope = AABB::from_corners([r.min_x(), r.min_y()], [r.max_x(), r.max_y()]);
        self.tree
            .locate_in_envelope_intersecting(&envelope)
            .map(|b| b.key)
            .collect()
    }
}

impl Default for SpatialIndex {
    fn default() -> Self {
        Self::new()
    }
}
