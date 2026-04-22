//! Geometry conversions: `jian_core::geometry` primitives → `skia_safe`.

use jian_core::geometry::{Affine2, Point, Rect};
use skia_safe::{Matrix, Point as SkPoint, Rect as SkRect};

pub fn to_sk_rect(r: Rect) -> SkRect {
    SkRect::from_ltrb(r.min_x(), r.min_y(), r.max_x(), r.max_y())
}

pub fn to_sk_point(p: Point) -> SkPoint {
    SkPoint::new(p.x, p.y)
}

/// `euclid::Transform2D` uses **row-vector × matrix** convention
/// (`[x y 1] · M`), so `to_array() = [m11, m12, m21, m22, m31, m32]`
/// where `x' = x*m11 + y*m21 + m31`. Skia's `Matrix::set_all` takes
/// `(scale_x, skew_x, trans_x, skew_y, scale_y, trans_y, 0, 0, 1)`
/// under the same row-vector convention, so the mapping is a direct
/// column-swap (columns 0 and 1 of euclid's matrix become rows 0 and 1
/// of Skia's).
pub fn to_sk_matrix(m: &Affine2) -> Matrix {
    let a = m.to_array();
    let mut out = Matrix::new_identity();
    out.set_all(a[0], a[2], a[4], a[1], a[3], a[5], 0.0, 0.0, 1.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::geometry::{point, rect};

    #[test]
    fn rect_roundtrips() {
        let r = rect(10.0, 20.0, 30.0, 40.0);
        let s = to_sk_rect(r);
        assert!((s.left - 10.0).abs() < f32::EPSILON);
        assert!((s.width() - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn point_roundtrips() {
        let p = point(7.5, 9.0);
        let s = to_sk_point(p);
        assert!((s.x - 7.5).abs() < f32::EPSILON);
    }

    #[test]
    fn identity_matrix() {
        let id = Affine2::identity();
        let sk = to_sk_matrix(&id);
        assert!(sk.is_identity());
    }
}
