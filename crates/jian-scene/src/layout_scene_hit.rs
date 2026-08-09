//! Canvas hit-test over a [`LayoutScene`].
//!
//! These are the input-path geometry queries that used to live as
//! `&Document`-bound helpers (`Document::node_at_doc_point`,
//! `Document::nodes_intersecting_doc_rect`). The reorg moves the
//! editor hosts' input dispatch off the derived `Document` snapshot
//! and onto the layout-resolved [`LayoutScene`], so the hit-test
//! reads the same resolved geometry the painter walks.
//!
//! Hit semantics carry over from `document/walkers.rs`:
//! top-most-first z-order (`children[0]` is frontmost), per-node rotation inverse-transform, the
//! tighter Ellipse / Polygon / Line geometry, the locked-node
//! body-opts-out-children-stay rule, and the hidden-subtree skip.
//!
//! One rule is newer: a Frame / Group that paints no body of its own is
//! not selectable across its empty area (see [`paints_body`]). Painted
//! containers are unchanged, and a top-level frame stays reachable
//! through its canvas name label, which hit-tests ahead of this walk.

use crate::layout_scene::NodeKind;
use crate::layout_scene::{regular_polygon_points, LayoutScene, SceneNode};
use crate::path_geometry::{flatten_path_points, PathPoints};
use jian_widgets::geometry::{Point2D, Rect};

impl LayoutScene {
    /// Topmost node id whose geometry contains `point` (doc space)
    /// on the active page. Walks children in front-to-back z-order
    /// (`children[0]` is frontmost). `None` on dead space.
    ///
    /// `zoom` is the live viewport zoom — it scales the Line stroke
    /// hit slack so a thin line stays clickable at any zoom.
    pub fn node_at_doc_point(&self, point: Point2D, zoom: f32) -> Option<String> {
        let zoom = zoom.max(0.0001);
        let page = self.active_page()?;
        for child in &page.children {
            if let Some(hit) = hit_test_walk(child, point, zoom) {
                return Some(hit);
            }
        }
        None
    }

    /// Root-to-deepest structural path for the same topmost hit returned by
    /// [`Self::node_at_doc_point`]. The last id is always the selectable hit;
    /// preceding ids are its scene ancestors in document order.
    ///
    /// Locked ancestors stay in the path when one of their descendants is
    /// hittable, while a locked node's own body is never the terminal hit.
    /// Hidden subtrees are omitted entirely. Rotation and flips are resolved
    /// at every ancestor exactly as in the single-id hit-test.
    pub fn node_path_at_doc_point(&self, point: Point2D, zoom: f32) -> Option<Vec<String>> {
        self.node_path_at_doc_point_impl(point, zoom, false)
    }

    /// Like [`Self::node_path_at_doc_point`], but an unpainted container body
    /// still terminates the path. Clicks skip an empty decoration shell so it
    /// cannot swallow the content behind it; an image dropped onto an empty
    /// placeholder box wants exactly that box, so the drop-target resolver
    /// opts back in here rather than falling through to a fresh insertion.
    pub fn node_path_at_doc_point_for_fill(
        &self,
        point: Point2D,
        zoom: f32,
    ) -> Option<Vec<String>> {
        self.node_path_at_doc_point_impl(point, zoom, true)
    }

    fn node_path_at_doc_point_impl(
        &self,
        point: Point2D,
        zoom: f32,
        include_empty_body: bool,
    ) -> Option<Vec<String>> {
        let zoom = zoom.max(0.0001);
        let page = self.active_page()?;
        let mut path = Vec::new();
        for child in &page.children {
            if hit_test_path_walk(child, point, zoom, &mut path, include_empty_body) {
                return Some(path.into_iter().map(str::to_owned).collect());
            }
            debug_assert!(path.is_empty());
        }
        None
    }

    /// Top-level node ids on the active page whose aggregate bounds
    /// intersect `rect` (doc space). Backs the marquee rect-select.
    /// Descends only into top-level children — same as the click
    /// hit-test, so the result set selects as a unit.
    pub fn nodes_intersecting_doc_rect(&self, rect: Rect) -> Vec<String> {
        let Some(page) = self.active_page() else {
            return Vec::new();
        };
        let nx = rect.origin.x.min(rect.origin.x + rect.size.x);
        let ny = rect.origin.y.min(rect.origin.y + rect.size.y);
        let nw = rect.size.x.abs();
        let nh = rect.size.y.abs();
        let mut out = Vec::new();
        for child in &page.children {
            let b = child.aggregate_bounds();
            if b.size.x <= 0.0 && b.size.y <= 0.0 {
                continue;
            }
            let bx = b.origin.x.min(b.origin.x + b.size.x);
            let by = b.origin.y.min(b.origin.y + b.size.y);
            let bw = b.size.x.abs();
            let bh = b.size.y.abs();
            // AABB intersection test.
            if bx + bw < nx || nx + nw < bx || by + bh < ny || ny + nh < by {
                continue;
            }
            out.push(child.id.clone());
        }
        out
    }
}

/// Recursive hit-test — returns the topmost id whose geometry
/// contains `point`. When a node carries rotation, the test point
/// is inverse-rotated about the node's pivot BEFORE testing
/// children + self, so the hit area matches the rendered geometry.
fn hit_test_walk(node: &SceneNode, point: Point2D, zoom: f32) -> Option<String> {
    // Hidden nodes skip hit-test entirely — the subtree inherits.
    if node.hidden {
        return None;
    }
    let bounds = node.aggregate_bounds();
    let local = point_in_node_space(node, point, bounds);
    // `visible_children` — not `children` — so hit-test and paint share one
    // rule for which of a tabs node's overlapping panels is live.
    for child in node.visible_children() {
        if let Some(hit) = hit_test_walk(child, local, zoom) {
            return Some(hit);
        }
    }
    // Locked nodes can't be selected via canvas hit, but their
    // children still can — this check runs AFTER the child walk so
    // descendants of a locked Frame remain hittable; only the
    // Frame's own body opts out.
    if node.locked {
        return None;
    }
    // Clicks never terminate on an unpainted container body.
    if point_in_node(node, local, bounds, zoom, false) {
        return Some(node.id.clone());
    }
    None
}

/// Path-preserving counterpart of [`hit_test_walk`]. `path` borrows ids while
/// probing so misses do not clone strings; the public API allocates only the
/// ancestors of the final hit.
fn hit_test_path_walk<'a>(
    node: &'a SceneNode,
    point: Point2D,
    zoom: f32,
    path: &mut Vec<&'a str>,
    include_empty_body: bool,
) -> bool {
    if node.hidden {
        return false;
    }
    let bounds = node.aggregate_bounds();
    let local = point_in_node_space(node, point, bounds);
    path.push(node.id.as_str());
    // Same painted-subtree rule as `hit_test_walk`.
    for child in node.visible_children() {
        if hit_test_path_walk(child, local, zoom, path, include_empty_body) {
            return true;
        }
    }
    if !node.locked && point_in_node(node, local, bounds, zoom, include_empty_body) {
        return true;
    }
    path.pop();
    false
}

fn point_in_node_space(node: &SceneNode, point: Point2D, bounds: Rect) -> Point2D {
    let local = if node.rotation.abs() > f32::EPSILON {
        if let Some(pivot) = rotation_pivot(node, bounds) {
            let dx = point.x - pivot.x;
            let dy = point.y - pivot.y;
            let cos_t = (-node.rotation).cos();
            let sin_t = (-node.rotation).sin();
            Point2D::new(
                pivot.x + dx * cos_t - dy * sin_t,
                pivot.y + dx * sin_t + dy * cos_t,
            )
        } else {
            point
        }
    } else {
        point
    };
    if node.flip_x || node.flip_y {
        if let Some(pivot) = rotation_pivot(node, bounds) {
            Point2D::new(
                if node.flip_x {
                    2.0 * pivot.x - local.x
                } else {
                    local.x
                },
                if node.flip_y {
                    2.0 * pivot.y - local.y
                } else {
                    local.y
                },
            )
        } else {
            local
        }
    } else {
        local
    }
}

/// Rotation pivot for hit-test. Most kinds rotate around the
/// aggregate-bounds center; Lines need a kind-specific path because
/// a Line with a negative-size dimension collapses its aggregate to
/// `Rect::ZERO` (the segment midpoint is still well defined from
/// `bounds`). Returns `None` when no valid pivot exists.
fn rotation_pivot(node: &SceneNode, bounds: Rect) -> Option<Point2D> {
    if matches!(node.kind, NodeKind::Line) {
        let raw = node.bounds;
        if raw.size.x.abs() < f32::EPSILON && raw.size.y.abs() < f32::EPSILON {
            return None;
        }
        return Some(Point2D::new(
            raw.origin.x + raw.size.x / 2.0,
            raw.origin.y + raw.size.y / 2.0,
        ));
    }
    if bounds.size.x > 0.0 && bounds.size.y > 0.0 {
        return Some(Point2D::new(
            bounds.origin.x + bounds.size.x / 2.0,
            bounds.origin.y + bounds.size.y / 2.0,
        ));
    }
    None
}

/// Per-NodeKind hit-test. Frames / Groups / Rects / Text / Other
/// use the axis-aligned bounds; Ellipse / Polygon / Line use tighter
/// geometry so the click area matches what the painter draws.
fn point_in_node(
    node: &SceneNode,
    local: Point2D,
    bounds: Rect,
    zoom: f32,
    include_empty_body: bool,
) -> bool {
    // Lines get a dedicated path: horizontal / vertical segments
    // have a zero-dimension bounds rect, and negative-size bounds
    // collapse the aggregate to `Rect::ZERO`, so the Line path reads
    // `node.bounds` directly. The distance-to-segment helper is
    // sign-independent.
    if matches!(node.kind, NodeKind::Line) {
        let raw = node.bounds;
        if raw.size.x.abs() < f32::EPSILON && raw.size.y.abs() < f32::EPSILON {
            return false;
        }
        let from = raw.origin;
        let to = Point2D::new(raw.origin.x + raw.size.x, raw.origin.y + raw.size.y);
        let stroke_half = node.stroke.map(|s| s.width / 2.0).unwrap_or(1.0);
        // 4 screen px of slack regardless of zoom — the point is in
        // doc space, so scale by `1/zoom`.
        let screen_slack = 4.0 / zoom.max(0.0001);
        return distance_point_to_segment(local, from, to) <= stroke_half + screen_slack;
    }
    // Paths hit-test against their flattened (bezier-aware) outline,
    // not the bounding box — a curved / thin path otherwise selects
    // empty bbox space, and a zero-height stroked path (which fails
    // the positive-area gate below) stays clickable.
    if matches!(node.kind, NodeKind::Path) {
        let poly = path_hit_points(node);
        let points = poly.as_slice();
        if points.len() < 2 {
            return false;
        }
        let stroke_half = node.stroke.map(|s| s.width / 2.0).unwrap_or(1.0);
        let slack = 4.0 / zoom.max(0.0001);
        for seg in points.windows(2) {
            if distance_point_to_segment(local, seg[0], seg[1]) <= stroke_half + slack {
                return true;
            }
        }
        // A filled closed path is also hittable across its interior.
        return node.path_closed && node.fill.is_some() && point_in_polygon(local, points);
    }
    // A container that paints nothing does not claim its own body. This
    // runs after the child walk in both walkers, so descendants stay
    // hittable — only the empty body opts out, the same shape as the
    // locked-node rule. Without it a full-bleed transparent decoration
    // layer (the `layout: none` overlay idiom: sparse tape / punch-hole
    // art in a `fill_container` wrapper) is an invisible solid board that
    // swallows every click over the content behind it.
    if !include_empty_body
        && matches!(node.kind, NodeKind::Frame | NodeKind::Group)
        && !paints_body(node)
    {
        return false;
    }
    // Non-line kinds need real positive area on both axes.
    if bounds.size.x <= 0.0 || bounds.size.y <= 0.0 {
        return false;
    }
    let in_bounds = local.x >= bounds.origin.x
        && local.x <= bounds.origin.x + bounds.size.x
        && local.y >= bounds.origin.y
        && local.y <= bounds.origin.y + bounds.size.y;
    if !in_bounds {
        return false;
    }
    let cx = bounds.origin.x + bounds.size.x / 2.0;
    let cy = bounds.origin.y + bounds.size.y / 2.0;
    let rx = (bounds.size.x / 2.0).max(0.0001);
    let ry = (bounds.size.y / 2.0).max(0.0001);
    match node.kind {
        NodeKind::Ellipse => {
            let dx = (local.x - cx) / rx;
            let dy = (local.y - cy) / ry;
            let r2 = dx * dx + dy * dy;
            if r2 > 1.0 {
                return false;
            }
            // Arc ellipse: also require the point inside the pie /
            // donut sector so a missing wedge is not selectable.
            let inner = node.arc_inner_radius.unwrap_or(0.0).clamp(0.0, 1.0);
            let has_arc =
                node.arc_start_angle.is_some() || node.arc_sweep_angle.is_some() || inner > 0.001;
            if !has_arc {
                return true;
            }
            let sweep = node.arc_sweep_angle.unwrap_or(360.0);
            if sweep.abs() < 359.9 {
                let start = node.arc_start_angle.unwrap_or(0.0);
                // Normalise to a forward sweep — a negative sweep
                // covers the angular range `[start + sweep, start]`.
                let (sector_start, span) = if sweep < 0.0 {
                    (start + sweep, -sweep)
                } else {
                    (start, sweep)
                };
                let ang = dy.atan2(dx).to_degrees();
                let rel = (ang - sector_start).rem_euclid(360.0);
                if rel > span {
                    return false;
                }
            }
            r2 >= inner * inner
        }
        NodeKind::Polygon => {
            let points = regular_polygon_points(bounds, node.polygon_sides);
            point_in_polygon(local, &points)
        }
        // Frame, Group, Rect, Text, Other, Path — bounds-only hit.
        _ => true,
    }
}

/// Whether a container draws anything of its own. Children are not
/// consulted — a container whose only ink comes from its descendants
/// still has an empty body, and those descendants are hit-tested on
/// their own geometry.
fn paints_body(node: &SceneNode) -> bool {
    node.fill.is_some()
        || !node.fill_layers.is_empty()
        || node.stroke.is_some()
        || node.image_src.is_some()
        || !node.effects.is_empty()
}

fn path_hit_points(node: &SceneNode) -> PathPoints<'_> {
    flatten_path_points(node)
}

/// Even-odd ray-cast point-in-polygon test over a closed vertex ring.
fn point_in_polygon(p: Point2D, poly: &[Point2D]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Shortest distance from `p` to the segment `a`–`b`.
fn distance_point_to_segment(p: Point2D, a: Point2D, b: Point2D) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len_sq = dx * dx + dy * dy;
    if len_sq < f32::EPSILON {
        let pdx = p.x - a.x;
        let pdy = p.y - a.y;
        return (pdx * pdx + pdy * pdy).sqrt();
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len_sq).clamp(0.0, 1.0);
    let cx = a.x + t * dx;
    let cy = a.y + t * dy;
    let pdx = p.x - cx;
    let pdy = p.y - cy;
    (pdx * pdx + pdy * pdy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_scene::{LayoutScene, SceneNode, ScenePage};

    fn leaf(id: &str, kind: NodeKind, bounds: Rect) -> SceneNode {
        let mut n = SceneNode::leaf(id, kind);
        n.bounds = bounds;
        n
    }

    /// A Frame that paints a body, so it is selectable across its whole
    /// rect rather than opting out through `paints_body`.
    fn filled(id: &str, bounds: Rect) -> SceneNode {
        let mut n = leaf(id, NodeKind::Frame, bounds);
        n.fill = Some(jian_widgets::Color::rgb_u8(0x11, 0x22, 0x33));
        n
    }

    fn one_page(children: Vec<SceneNode>) -> LayoutScene {
        LayoutScene {
            pages: vec![ScenePage {
                id: "p".into(),
                name: "P".into(),
                children,
            }],
            active_page_index: 0,
        }
    }

    fn path(ids: &[&str]) -> Option<Vec<String>> {
        Some(ids.iter().map(|id| (*id).to_owned()).collect())
    }

    #[test]
    fn node_at_doc_point_treats_first_child_as_frontmost() {
        let scene = one_page(vec![
            leaf("over", NodeKind::Rect, Rect::xywh(0.0, 0.0, 50.0, 50.0)),
            leaf("under", NodeKind::Rect, Rect::xywh(10.0, 10.0, 50.0, 50.0)),
        ]);
        // Overlap region -> children[0] is the top layer.
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(20.0, 20.0), 1.0)
                .as_deref(),
            Some("over")
        );
        // Only "under" covers (55, 55).
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(55.0, 55.0), 1.0)
                .as_deref(),
            Some("under")
        );
        // Dead space.
        assert!(scene
            .node_at_doc_point(Point2D::new(200.0, 200.0), 1.0)
            .is_none());
    }

    #[test]
    fn node_path_returns_frontmost_root_to_deepest_hit() {
        let mut front = leaf(
            "front-root",
            NodeKind::Frame,
            Rect::xywh(0.0, 0.0, 100.0, 100.0),
        );
        front.children = vec![leaf(
            "front-leaf",
            NodeKind::Rect,
            Rect::xywh(10.0, 10.0, 30.0, 30.0),
        )];
        let mut back = leaf(
            "back-root",
            NodeKind::Frame,
            Rect::xywh(0.0, 0.0, 100.0, 100.0),
        );
        back.children = vec![leaf(
            "back-leaf",
            NodeKind::Rect,
            Rect::xywh(10.0, 10.0, 30.0, 30.0),
        )];
        let scene = one_page(vec![front, back]);
        let point = Point2D::new(20.0, 20.0);

        let hit_path = scene.node_path_at_doc_point(point, 1.0);
        assert_eq!(hit_path, path(&["front-root", "front-leaf"]));
        assert_eq!(
            hit_path.as_ref().and_then(|ids| ids.last()).cloned(),
            scene.node_at_doc_point(point, 1.0),
            "the terminal path id must preserve the existing hit-test result"
        );
        assert_eq!(
            scene.node_path_at_doc_point(Point2D::new(200.0, 200.0), 1.0),
            None
        );
    }

    #[test]
    fn node_path_skips_hidden_front_subtree() {
        let mut hidden = leaf(
            "hidden-root",
            NodeKind::Frame,
            Rect::xywh(0.0, 0.0, 100.0, 100.0),
        );
        hidden.hidden = true;
        hidden.children = vec![leaf(
            "hidden-leaf",
            NodeKind::Rect,
            Rect::xywh(10.0, 10.0, 30.0, 30.0),
        )];
        let mut visible = leaf(
            "visible-root",
            NodeKind::Frame,
            Rect::xywh(0.0, 0.0, 100.0, 100.0),
        );
        visible.children = vec![leaf(
            "visible-leaf",
            NodeKind::Rect,
            Rect::xywh(10.0, 10.0, 30.0, 30.0),
        )];
        let scene = one_page(vec![hidden, visible]);

        assert_eq!(
            scene.node_path_at_doc_point(Point2D::new(20.0, 20.0), 1.0),
            path(&["visible-root", "visible-leaf"])
        );
    }

    /// A tabs frame with two fully overlapping panels — the shape jian
    /// compiles a `layout: none` / single-cell grid into. Only the panel the
    /// painter draws may be hittable.
    fn tabs_scene(active: Option<&str>) -> LayoutScene {
        let mut tabs = leaf("tabs", NodeKind::Frame, Rect::xywh(0.0, 0.0, 200.0, 200.0));
        tabs.widget = Some(crate::layout_scene::SceneWidget {
            kind: "tabs".into(),
            value_str: active.map(str::to_owned),
            options: vec![
                crate::layout_scene::SceneWidgetOption {
                    value: "overview".into(),
                    label: "Overview".into(),
                },
                crate::layout_scene::SceneWidgetOption {
                    value: "details".into(),
                    label: "Details".into(),
                },
            ],
            ..Default::default()
        });
        // The panels carry a fill because this test is about which panel
        // the walk routes to, not about whether an empty container claims
        // its own body — an unfilled panel would opt out via `paints_body`
        // and mask the routing this asserts.
        tabs.children = vec![
            filled("overview-panel", Rect::xywh(0.0, 40.0, 200.0, 160.0)),
            filled("details-panel", Rect::xywh(0.0, 40.0, 200.0, 160.0)),
        ];
        one_page(vec![tabs])
    }

    /// The `layout: none` decoration idiom: a full-bleed wrapper holding
    /// a few small marks, stacked in front of the content it decorates.
    /// It paints no body, so its empty area must fall through to the
    /// content behind rather than swallowing every click in the frame.
    #[test]
    fn a_full_bleed_transparent_decoration_layer_does_not_swallow_clicks() {
        let mut decoration = leaf(
            "decoration",
            NodeKind::Group,
            Rect::xywh(0.0, 0.0, 200.0, 200.0),
        );
        decoration.children = vec![leaf(
            "tape",
            NodeKind::Rect,
            Rect::xywh(0.0, 0.0, 20.0, 20.0),
        )];
        let mut content = leaf(
            "content",
            NodeKind::Frame,
            Rect::xywh(0.0, 0.0, 200.0, 200.0),
        );
        content.children = vec![leaf(
            "headline",
            NodeKind::Text,
            Rect::xywh(50.0, 100.0, 100.0, 40.0),
        )];
        let mut root = filled("board", Rect::xywh(0.0, 0.0, 200.0, 200.0));
        root.children = vec![decoration, content];
        let scene = one_page(vec![root]);

        // Over the text: the decoration is frontmost and its rect covers
        // this point, but only the text is painted here.
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(100.0, 120.0), 1.0)
                .as_deref(),
            Some("headline"),
            "a transparent decoration layer must not intercept the content below it"
        );
        // The path walk agrees, so canvas drill-down and text editing —
        // which both read the hit path — can reach the text node.
        assert_eq!(
            scene.node_path_at_doc_point(Point2D::new(100.0, 120.0), 1.0),
            path(&["board", "content", "headline"]),
        );
        // The decoration's own marks stay selectable.
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(10.0, 10.0), 1.0)
                .as_deref(),
            Some("tape"),
        );
        // Empty space in both wrappers falls through to the painted root.
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(180.0, 180.0), 1.0)
                .as_deref(),
            Some("board"),
        );
    }

    #[test]
    fn a_painted_container_still_claims_its_whole_body() {
        let mut card = filled("card", Rect::xywh(0.0, 0.0, 100.0, 100.0));
        card.children = vec![leaf(
            "label",
            NodeKind::Text,
            Rect::xywh(10.0, 10.0, 20.0, 20.0),
        )];
        let scene = one_page(vec![card]);
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(80.0, 80.0), 1.0)
                .as_deref(),
            Some("card"),
            "a filled card is selectable across its padding, not just on its children"
        );
    }

    /// A stroke-only outline box paints ink without a fill, so it keeps
    /// its body — `paints_body` is about ink, not about `fill` alone.
    #[test]
    fn a_stroke_only_container_keeps_its_body() {
        let mut outlined = leaf(
            "outlined",
            NodeKind::Frame,
            Rect::xywh(0.0, 0.0, 100.0, 100.0),
        );
        outlined.stroke = Some(crate::layout_scene::SceneStroke {
            color: jian_widgets::Color::rgb_u8(0, 0, 0),
            width: 2.0,
            sides: None,
            align: crate::layout_scene::SceneStrokeAlign::Center,
        });
        let scene = one_page(vec![outlined]);
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(50.0, 50.0), 1.0)
                .as_deref(),
            Some("outlined"),
        );
    }

    #[test]
    fn tabs_hit_test_only_reaches_the_active_panel() {
        let point = Point2D::new(100.0, 120.0);

        // Missing / stale values deterministically select the first panel.
        for active in [None, Some("nope")] {
            let scene = tabs_scene(active);
            assert_eq!(
                scene.node_at_doc_point(point, 1.0).as_deref(),
                Some("overview-panel"),
                "unresolved active value must fall back to the first panel"
            );
        }

        let scene = tabs_scene(Some("details"));
        assert_eq!(
            scene.node_at_doc_point(point, 1.0).as_deref(),
            Some("details-panel"),
            "the second tab's panel is on top of the first — the hit must \
             follow the active value, not document order"
        );
        assert_eq!(
            scene.node_path_at_doc_point(point, 1.0),
            path(&["tabs", "details-panel"]),
            "the path walk must apply the same active-panel rule"
        );
    }

    #[test]
    fn node_path_tracks_ancestor_rotation_and_flip() {
        let mut rotated = leaf(
            "rotated-root",
            NodeKind::Frame,
            Rect::xywh(0.0, 0.0, 100.0, 100.0),
        );
        rotated.rotation = std::f32::consts::FRAC_PI_2;
        rotated.children = vec![leaf(
            "rotated-leaf",
            NodeKind::Rect,
            Rect::xywh(10.0, 40.0, 20.0, 20.0),
        )];
        let rotated_scene = one_page(vec![rotated]);
        assert_eq!(
            rotated_scene.node_path_at_doc_point(Point2D::new(50.0, 20.0), 1.0),
            path(&["rotated-root", "rotated-leaf"])
        );

        let mut flipped = leaf(
            "flipped-root",
            NodeKind::Frame,
            Rect::xywh(0.0, 0.0, 100.0, 100.0),
        );
        flipped.flip_x = true;
        flipped.children = vec![leaf(
            "flipped-leaf",
            NodeKind::Rect,
            Rect::xywh(10.0, 10.0, 20.0, 20.0),
        )];
        let flipped_scene = one_page(vec![flipped]);
        assert_eq!(
            flipped_scene.node_path_at_doc_point(Point2D::new(80.0, 20.0), 1.0),
            path(&["flipped-root", "flipped-leaf"])
        );
    }

    #[test]
    fn node_path_keeps_locked_ancestors_but_not_locked_terminal_nodes() {
        let child = leaf("child", NodeKind::Rect, Rect::xywh(10.0, 10.0, 20.0, 20.0));
        let mut locked_root = leaf(
            "locked-root",
            NodeKind::Frame,
            Rect::xywh(0.0, 0.0, 100.0, 100.0),
        );
        locked_root.locked = true;
        locked_root.children = vec![child];
        let scene = one_page(vec![locked_root]);
        assert_eq!(
            scene.node_path_at_doc_point(Point2D::new(20.0, 20.0), 1.0),
            path(&["locked-root", "child"])
        );
        assert_eq!(
            scene.node_path_at_doc_point(Point2D::new(80.0, 80.0), 1.0),
            None,
            "a locked ancestor's bare body must not become the path endpoint"
        );

        let mut locked_front = leaf(
            "locked-front",
            NodeKind::Rect,
            Rect::xywh(10.0, 10.0, 20.0, 20.0),
        );
        locked_front.locked = true;
        let back = leaf(
            "unlocked-back",
            NodeKind::Rect,
            Rect::xywh(10.0, 10.0, 20.0, 20.0),
        );
        let mut root = leaf("root", NodeKind::Frame, Rect::xywh(0.0, 0.0, 100.0, 100.0));
        root.children = vec![locked_front, back];
        let scene = one_page(vec![root]);
        assert_eq!(
            scene.node_path_at_doc_point(Point2D::new(20.0, 20.0), 1.0),
            path(&["root", "unlocked-back"])
        );
    }

    #[test]
    fn hidden_node_is_not_hit() {
        let mut n = leaf("h", NodeKind::Rect, Rect::xywh(0.0, 0.0, 50.0, 50.0));
        n.hidden = true;
        let scene = one_page(vec![n]);
        assert!(scene
            .node_at_doc_point(Point2D::new(25.0, 25.0), 1.0)
            .is_none());
    }

    #[test]
    fn locked_node_body_opts_out_but_child_stays_hittable() {
        let child = leaf("c", NodeKind::Rect, Rect::xywh(10.0, 10.0, 10.0, 10.0));
        let mut frame = leaf("f", NodeKind::Frame, Rect::xywh(0.0, 0.0, 100.0, 100.0));
        frame.locked = true;
        frame.children = vec![child];
        let scene = one_page(vec![frame]);
        // Click on the child → child id even though parent is locked.
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(15.0, 15.0), 1.0)
                .as_deref(),
            Some("c")
        );
        // Click on the locked frame's bare body → no hit.
        assert!(scene
            .node_at_doc_point(Point2D::new(80.0, 80.0), 1.0)
            .is_none());
    }

    #[test]
    fn ellipse_uses_tight_oval_geometry() {
        let scene = one_page(vec![leaf(
            "e",
            NodeKind::Ellipse,
            Rect::xywh(0.0, 0.0, 100.0, 100.0),
        )]);
        // Center → inside the oval.
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(50.0, 50.0), 1.0)
                .as_deref(),
            Some("e")
        );
        // Corner of the bounds → outside the oval.
        assert!(scene
            .node_at_doc_point(Point2D::new(2.0, 2.0), 1.0)
            .is_none());
    }

    #[test]
    fn line_hit_uses_segment_distance() {
        let mut line = leaf("l", NodeKind::Line, Rect::xywh(0.0, 0.0, 100.0, 0.0));
        line.stroke = Some(crate::layout_scene::SceneStroke {
            color: jian_widgets::geometry::Color::WHITE,
            width: 1.0,
            sides: None,
            align: crate::layout_scene::SceneStrokeAlign::Center,
        });
        let scene = one_page(vec![line]);
        // Right on the horizontal segment.
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(50.0, 0.0), 1.0)
                .as_deref(),
            Some("l")
        );
        // Far from the segment → miss.
        assert!(scene
            .node_at_doc_point(Point2D::new(50.0, 40.0), 1.0)
            .is_none());
    }

    #[test]
    fn rotated_node_hit_tracks_rendered_geometry() {
        let mut n = leaf("r", NodeKind::Rect, Rect::xywh(0.0, 0.0, 100.0, 20.0));
        n.rotation = std::f32::consts::FRAC_PI_2; // 90°
        let scene = one_page(vec![n]);
        // After a 90° rotation about (50, 10), the painted rect spans
        // roughly x 40..60, y -40..60 — a point at (50, 50) lands
        // inside the rotated body though it's outside authored bounds.
        assert_eq!(
            scene
                .node_at_doc_point(Point2D::new(50.0, 50.0), 1.0)
                .as_deref(),
            Some("r")
        );
    }

    #[test]
    fn path_hit_points_borrows_handle_free_open_path() {
        let mut path = leaf("p", NodeKind::Path, Rect::xywh(0.0, 0.0, 100.0, 0.0));
        path.points = vec![Point2D::new(0.0, 0.0), Point2D::new(100.0, 0.0)];

        let points = path_hit_points(&path);

        match points {
            crate::path_geometry::PathPoints::Borrowed(slice) => {
                assert!(std::ptr::eq(slice.as_ptr(), path.points.as_ptr()));
            }
            crate::path_geometry::PathPoints::Owned(_) => {
                panic!("handle-free open path hit-test should borrow points")
            }
        }
    }

    #[test]
    fn nodes_intersecting_doc_rect_returns_overlapping_top_level_ids() {
        let scene = one_page(vec![
            leaf("a", NodeKind::Rect, Rect::xywh(0.0, 0.0, 30.0, 30.0)),
            leaf("b", NodeKind::Rect, Rect::xywh(100.0, 100.0, 30.0, 30.0)),
        ]);
        let hits = scene.nodes_intersecting_doc_rect(Rect::xywh(10.0, 10.0, 80.0, 80.0));
        assert_eq!(hits, vec!["a".to_string()]);
    }

    #[test]
    fn nodes_intersecting_doc_rect_handles_negative_size() {
        let scene = one_page(vec![leaf(
            "a",
            NodeKind::Rect,
            Rect::xywh(0.0, 0.0, 30.0, 30.0),
        )]);
        // Negative-size marquee still intersects.
        let hits = scene.nodes_intersecting_doc_rect(Rect::xywh(40.0, 40.0, -30.0, -30.0));
        assert_eq!(hits, vec!["a".to_string()]);
    }
}
