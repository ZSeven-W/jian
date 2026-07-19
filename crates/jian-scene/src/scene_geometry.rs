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
    jian_ops_schema::node::image_src::paint_image_id(src)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_image_id_matches_the_persisted_schema_id() {
        assert_eq!(
            stable_image_source_id("data:image/png;base64,AA=="),
            0x641a_8b95_c7ff_c372
        );
    }
}
