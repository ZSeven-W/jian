//! Pure scene geometry helpers, split out of `layout_scene.rs` to keep that
//! file under the 800-line cap. These touch no `SceneNode` internals — only
//! `Rect` / `Point2D` / `str` — and are re-exported from `layout_scene` so the
//! existing `layout_scene::{regular_polygon_points, stable_image_source_id}`
//! call paths stay valid.

use jian_widgets::{Point2D, Rect};

pub(crate) fn rect_has_extent(rect: Rect) -> bool {
    rect.size.x > 0.0 || rect.size.y > 0.0
}

pub fn stable_image_source_id(src: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let bytes = src.as_bytes();
    // Hash length + bounded head/middle/tail windows rather than the whole
    // string, so the id is O(1) even for multi-MB data URLs. Re-hashing the
    // full base64 on every drag frame (the scene rebuilds on each node move)
    // was the source of the image-drag lag. Distinct images differ in length
    // and/or these windows, so collisions are negligible.
    bytes.len().hash(&mut h);
    const W: usize = 512;
    if bytes.len() <= 3 * W {
        bytes.hash(&mut h);
    } else {
        let mid = bytes.len() / 2;
        bytes[..W].hash(&mut h);
        bytes[mid..mid + W].hash(&mut h);
        bytes[bytes.len() - W..].hash(&mut h);
    }
    h.finish()
}

/// Vertices for a regular polygon fitted inside `rect`.
pub fn regular_polygon_points(rect: Rect, sides: u32) -> Vec<Point2D> {
    let n = sides.clamp(3, 100) as usize;
    let cx = rect.origin.x + rect.size.x / 2.0;
    let cy = rect.origin.y + rect.size.y / 2.0;
    let rx = rect.size.x / 2.0;
    let ry = rect.size.y / 2.0;
    let start = -std::f32::consts::FRAC_PI_2;
    (0..n)
        .map(|i| {
            let angle = start + i as f32 * std::f32::consts::TAU / n as f32;
            Point2D::new(cx + rx * angle.cos(), cy + ry * angle.sin())
        })
        .collect()
}
