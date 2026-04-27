//! Spatial index over SceneNodes for hit testing and viewport culling.
//!
//! Plan 19 Task 5 adds two cold-start helpers to the simple `rebuild` API:
//!
//! - [`SpatialIndex::rebuild_from_visible`] — pre-filters items by
//!   intersection with the first-frame viewport and bulk-loads only
//!   those. Hit testing during the first frame stays fast even on
//!   1000-node documents because the tree only carries the ~100 nodes
//!   actually on screen.
//! - [`SpatialIndex::fill_rest`] — async chunked insert that catches the
//!   tree up with off-viewport nodes after `EventPumpReady`. Yields to
//!   the executor every `chunk_size` items so the main thread stays
//!   responsive while the index grows.

use crate::document::NodeKey;
use crate::geometry::{Point, Rect};
use rstar::{RTree, RTreeObject, AABB};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

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

    /// Bulk-load only the items whose bbox intersects `viewport`. Items
    /// outside the viewport are silently dropped — call
    /// [`Self::fill_rest`] on the remainder once `EventPumpReady` has
    /// fired.
    ///
    /// The bulk-load takes O(n log n) on the *visible* set, not on the
    /// whole document; for a 1000-node doc with 100 visible nodes that
    /// drops the first spatial build from ~5 ms to ~0.5 ms (per Plan 19
    /// §C19 measurement target).
    pub fn rebuild_from_visible<I: IntoIterator<Item = NodeBBox>>(
        &mut self,
        items: I,
        viewport: Rect,
    ) {
        let visible: Vec<NodeBBox> = items
            .into_iter()
            .filter(|b| b.rect.intersects(&viewport))
            .collect();
        self.tree = RTree::bulk_load(visible);
    }

    /// Insert `items` into the tree in chunks of `chunk_size`, yielding
    /// to the executor between chunks. Use after [`Self::rebuild_from_visible`]
    /// to fold in the off-viewport remainder over a few post-paint
    /// frames instead of in one main-thread stall.
    ///
    /// `chunk_size == 0` is treated as `1` so a buggy caller can't
    /// disable the yield. The caller controls the iterator order — this
    /// method does not re-prioritise items by distance from the
    /// viewport.
    ///
    /// **Pass only the off-viewport remainder.** `fill_rest` blindly
    /// inserts every item it receives; passing the full list (including
    /// already-visible items) duplicates them in the tree and makes
    /// [`Self::hit`] return duplicate keys. Use [`Self::partition_by_viewport`]
    /// to split the source list into `(visible, rest)` cleanly.
    pub async fn fill_rest<I>(&mut self, items: I, chunk_size: usize)
    where
        I: IntoIterator<Item = NodeBBox>,
    {
        let chunk_size = chunk_size.max(1);
        let mut in_chunk = 0_usize;
        for item in items {
            self.tree.insert(item);
            in_chunk += 1;
            if in_chunk >= chunk_size {
                in_chunk = 0;
                yield_once().await;
            }
        }
    }

    /// Split `items` into `(visible, rest)` by intersection with
    /// `viewport`. Convenience for the canonical first-frame pattern:
    ///
    /// ```ignore
    /// let (visible, rest) = SpatialIndex::partition_by_viewport(items, viewport);
    /// idx.rebuild(visible);
    /// idx.fill_rest(rest, 64).await;
    /// ```
    ///
    /// Closes the caller-discipline footgun on [`Self::fill_rest`]:
    /// callers who use this helper can't accidentally double-insert
    /// already-visible items.
    #[must_use = "the partition is the only output; ignoring it discards the work"]
    pub fn partition_by_viewport<I: IntoIterator<Item = NodeBBox>>(
        items: I,
        viewport: Rect,
    ) -> (Vec<NodeBBox>, Vec<NodeBBox>) {
        let mut visible = Vec::new();
        let mut rest = Vec::new();
        for item in items {
            if item.rect.intersects(&viewport) {
                visible.push(item);
            } else {
                rest.push(item);
            }
        }
        (visible, rest)
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

/// One-shot yield future. Returns `Pending` once (waking the task
/// immediately), then `Ready(())`. Used by [`SpatialIndex::fill_rest`]
/// to give the executor a chance to schedule other work between
/// insertion chunks.
fn yield_once() -> impl Future<Output = ()> {
    struct YieldOnce(bool);
    impl Future for YieldOnce {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                Poll::Ready(())
            } else {
                self.0 = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
    YieldOnce(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{point, rect};
    use futures::executor::block_on;
    use slotmap::KeyData;

    fn nk(idx: u32) -> NodeKey {
        // Plan 19 Task 5 tests don't need to round-trip a real
        // SlotMap — bypass via raw key data.
        NodeKey::from(KeyData::from_ffi(((1u64) << 32) | idx as u64))
    }

    fn bbox(idx: u32, x: f32, y: f32, w: f32, h: f32) -> NodeBBox {
        NodeBBox {
            key: nk(idx),
            rect: rect(x, y, w, h),
        }
    }

    #[test]
    fn rebuild_from_visible_keeps_only_intersecting_items() {
        let mut idx = SpatialIndex::new();
        let items = vec![
            bbox(1, 10.0, 10.0, 50.0, 50.0),     // inside viewport (0,0,200,200)
            bbox(2, 150.0, 150.0, 100.0, 100.0), // partial overlap
            bbox(3, 500.0, 500.0, 50.0, 50.0),   // far off-screen
            bbox(4, -200.0, -200.0, 100.0, 100.0), // negative far off-screen
        ];
        let viewport = rect(0.0, 0.0, 200.0, 200.0);
        idx.rebuild_from_visible(items, viewport);
        assert_eq!(idx.len(), 2, "only the two intersecting items survive");
        // Sanity: query a point inside item 1 finds it.
        assert_eq!(idx.hit(point(20.0, 20.0)), vec![nk(1)]);
        // The far off-screen items are gone.
        let everything = idx.query_rect(rect(-1000.0, -1000.0, 2000.0, 2000.0));
        let keys: std::collections::HashSet<_> = everything.into_iter().collect();
        assert!(keys.contains(&nk(1)));
        assert!(keys.contains(&nk(2)));
        assert!(!keys.contains(&nk(3)));
        assert!(!keys.contains(&nk(4)));
    }

    #[test]
    fn rebuild_from_visible_empty_iterator_yields_empty_tree() {
        let mut idx = SpatialIndex::new();
        idx.rebuild_from_visible(std::iter::empty::<NodeBBox>(), rect(0.0, 0.0, 100.0, 100.0));
        assert!(idx.is_empty());
    }

    #[test]
    fn rebuild_from_visible_viewport_off_screen_keeps_nothing() {
        let mut idx = SpatialIndex::new();
        let items = vec![
            bbox(1, 10.0, 10.0, 50.0, 50.0),
            bbox(2, 100.0, 100.0, 30.0, 30.0),
        ];
        // Viewport positioned far off any item.
        let viewport = rect(10_000.0, 10_000.0, 100.0, 100.0);
        idx.rebuild_from_visible(items, viewport);
        assert!(idx.is_empty());
    }

    #[test]
    fn fill_rest_appends_all_items() {
        let mut idx = SpatialIndex::new();
        idx.rebuild(vec![bbox(0, 0.0, 0.0, 10.0, 10.0)]);
        let extras = vec![
            bbox(1, 100.0, 100.0, 10.0, 10.0),
            bbox(2, 200.0, 200.0, 10.0, 10.0),
            bbox(3, 300.0, 300.0, 10.0, 10.0),
        ];
        block_on(idx.fill_rest(extras, 2));
        assert_eq!(idx.len(), 4);
        assert_eq!(idx.hit(point(105.0, 105.0)), vec![nk(1)]);
        assert_eq!(idx.hit(point(305.0, 305.0)), vec![nk(3)]);
    }

    #[test]
    fn fill_rest_chunk_size_zero_treated_as_one() {
        let mut idx = SpatialIndex::new();
        // chunk_size of 0 must not divide-by-zero or skip yields. We
        // can't directly observe the yield count, but we can confirm
        // the items still landed.
        let items = vec![bbox(1, 0.0, 0.0, 5.0, 5.0), bbox(2, 10.0, 10.0, 5.0, 5.0)];
        block_on(idx.fill_rest(items, 0));
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn partition_by_viewport_splits_into_visible_and_rest() {
        let items = vec![
            bbox(1, 10.0, 10.0, 20.0, 20.0),     // inside (0,0,200,200)
            bbox(2, 150.0, 150.0, 100.0, 100.0), // partial overlap → visible
            bbox(3, 500.0, 500.0, 30.0, 30.0),   // off-screen
            bbox(4, -200.0, -200.0, 50.0, 50.0), // off-screen negative
        ];
        let viewport = rect(0.0, 0.0, 200.0, 200.0);
        let (visible, rest) = SpatialIndex::partition_by_viewport(items, viewport);
        let visible_keys: std::collections::HashSet<_> = visible.iter().map(|b| b.key).collect();
        let rest_keys: std::collections::HashSet<_> = rest.iter().map(|b| b.key).collect();
        assert_eq!(visible.len(), 2);
        assert_eq!(rest.len(), 2);
        assert!(visible_keys.contains(&nk(1)));
        assert!(visible_keys.contains(&nk(2)));
        assert!(rest_keys.contains(&nk(3)));
        assert!(rest_keys.contains(&nk(4)));
        // visible ∩ rest is empty — proves no duplication footgun.
        assert!(visible_keys.is_disjoint(&rest_keys));
    }

    #[test]
    fn partition_then_rebuild_then_fill_rest_has_no_duplicates() {
        // Documents the footgun-free pattern partition_by_viewport
        // exists to enforce: every item lands exactly once.
        let mut idx = SpatialIndex::new();
        let items = vec![
            bbox(1, 10.0, 10.0, 20.0, 20.0),
            bbox(2, 500.0, 500.0, 30.0, 30.0),
            bbox(3, 800.0, 800.0, 30.0, 30.0),
        ];
        let viewport = rect(0.0, 0.0, 200.0, 200.0);
        let (visible, rest) = SpatialIndex::partition_by_viewport(items, viewport);
        idx.rebuild(visible);
        block_on(idx.fill_rest(rest, 1));
        assert_eq!(idx.len(), 3);
        // Each query point hits exactly one item — no duplicates.
        assert_eq!(idx.hit(point(15.0, 15.0)).len(), 1);
        assert_eq!(idx.hit(point(515.0, 515.0)).len(), 1);
        assert_eq!(idx.hit(point(815.0, 815.0)).len(), 1);
    }

    #[test]
    fn rebuild_from_visible_then_fill_rest_recovers_full_tree() {
        let mut idx = SpatialIndex::new();
        let inside = [bbox(1, 10.0, 10.0, 20.0, 20.0)];
        let outside = [
            bbox(2, 500.0, 500.0, 30.0, 30.0),
            bbox(3, 800.0, 800.0, 30.0, 30.0),
        ];
        let viewport = rect(0.0, 0.0, 200.0, 200.0);

        // First-frame rebuild only sees `inside`.
        idx.rebuild_from_visible(inside.iter().chain(outside.iter()).cloned(), viewport);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.hit(point(15.0, 15.0)), vec![nk(1)]);
        assert!(idx.hit(point(515.0, 515.0)).is_empty());

        // Post-paint fill catches the rest up.
        block_on(idx.fill_rest(outside, 1));
        assert_eq!(idx.len(), 3);
        assert_eq!(idx.hit(point(515.0, 515.0)), vec![nk(2)]);
        assert_eq!(idx.hit(point(815.0, 815.0)), vec![nk(3)]);
    }
}
