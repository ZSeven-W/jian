//! Pure path-flatten geometry for the render scene — bezier tessellation
//! shared by hit-testing and painting. Moved out of OpenPencil's
//! `canvas_viewport_paint.rs`; depends only on the scene model + Point2D
//! (no backend / widget / theme state).

use crate::layout_scene::{SceneAnchor, SceneNode};
use jian_widgets::geometry::Point2D;

pub fn cubic_point(p0: Point2D, p1: Point2D, p2: Point2D, p3: Point2D, t: f32) -> Point2D {
    let u = 1.0 - t;
    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point2D::new(
        w0 * p0.x + w1 * p1.x + w2 * p2.x + w3 * p3.x,
        w0 * p0.y + w1 * p1.y + w2 * p2.y + w3 * p3.y,
    )
}

/// One flattened segment `a → b` appended onto `out` — a cubic when
/// either endpoint carries a handle, else a straight line.
fn flatten_segment(a: &SceneAnchor, b: &SceneAnchor, out: &mut Vec<Point2D>) {
    let (p0, p3) = (a.pos, b.pos);
    let p1 = a.handle_out.unwrap_or(p0);
    let p2 = b.handle_in.unwrap_or(p3);
    if p1 == p0 && p2 == p3 {
        out.push(p3); // straight segment
    } else {
        for i in 1..=16 {
            out.push(cubic_point(p0, p1, p2, p3, i as f32 / 16.0));
        }
    }
}

pub enum PathPoints<'a> {
    Borrowed(&'a [Point2D]),
    Owned(Vec<Point2D>),
}

impl<'a> PathPoints<'a> {
    pub fn as_slice(&self) -> &[Point2D] {
        match self {
            Self::Borrowed(points) => points,
            Self::Owned(points) => points.as_slice(),
        }
    }
}

/// Flatten a Path scene node into doc-space points, borrowing the
/// original point slice for the common handle-free open-path case.
pub fn flatten_path_points(node: &SceneNode) -> PathPoints<'_> {
    let anchors = &node.path_anchors;
    let has_handle = anchors
        .iter()
        .any(|a| a.handle_in.is_some() || a.handle_out.is_some());
    if anchors.len() < 2 || !has_handle {
        if !node.path_closed {
            return PathPoints::Borrowed(&node.points);
        }
        let mut out = node.points.clone();
        // Closed handle-free path — link the polyline back to its
        // start so the closing edge is drawn.
        if node.path_closed && out.len() > 2 {
            out.push(out[0]);
        }
        return PathPoints::Owned(out);
    }
    let mut out = Vec::with_capacity(anchors.len() * 16 + 16);
    out.push(anchors[0].pos);
    for pair in anchors.windows(2) {
        flatten_segment(&pair[0], &pair[1], &mut out);
    }
    if node.path_closed {
        flatten_segment(&anchors[anchors.len() - 1], &anchors[0], &mut out);
    }
    PathPoints::Owned(out)
}
