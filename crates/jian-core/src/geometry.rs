//! 2D geometry primitives used across the crate.
//!
//! Wraps `euclid` unit-typed points/rects for ergonomics.
//! Scene coordinates are in **logical pixels** (DPI-independent).

use euclid::default::{Point2D, Rect as EuclidRect, Size2D, Transform2D};

pub type Point = Point2D<f32>;
pub type Size = Size2D<f32>;
pub type Rect = EuclidRect<f32>;
pub type Affine2 = Transform2D<f32>;

/// Convenience constructor for `Point`.
pub fn point(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

/// Convenience constructor for `Size`.
pub fn size(w: f32, h: f32) -> Size {
    Size::new(w, h)
}

/// Convenience constructor for `Rect` at `(x, y)` with `w × h`.
pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(point(x, y), size(w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_point() {
        let r = rect(10.0, 20.0, 100.0, 50.0);
        assert!(r.contains(point(50.0, 40.0)));
        assert!(!r.contains(point(0.0, 0.0)));
    }

    #[test]
    fn affine_identity_maps_point_to_self() {
        let m = Affine2::identity();
        let p = m.transform_point(point(3.0, 4.0));
        assert_eq!(p, point(3.0, 4.0));
    }

    #[test]
    fn translate_compose_scale() {
        let m = Affine2::translation(10.0, 20.0).then(&Affine2::scale(2.0, 2.0));
        let p = m.transform_point(point(1.0, 1.0));
        assert_eq!(p, point(22.0, 42.0));
    }
}
