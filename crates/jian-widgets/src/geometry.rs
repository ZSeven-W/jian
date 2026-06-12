//! Geometry and color types shared by widget painters.

/// 2D coordinate point.
pub type Point2D = glam::Vec2;

/// Rectangle represented as origin + size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub origin: Point2D,
    pub size: Point2D,
}

impl Rect {
    /// Zero-sized rect at the origin.
    pub const ZERO: Rect = Rect {
        origin: Point2D::new(0.0, 0.0),
        size: Point2D::new(0.0, 0.0),
    };

    /// Convenience builder: `Rect::xywh(x, y, w, h)`.
    pub const fn xywh(x: f32, y: f32, width: f32, height: f32) -> Self {
        Rect {
            origin: Point2D::new(x, y),
            size: Point2D::new(width, height),
        }
    }

    /// Closed-interval point hit-test.
    pub fn contains(&self, p: Point2D) -> bool {
        p.x >= self.origin.x
            && p.x <= self.origin.x + self.size.x
            && p.y >= self.origin.y
            && p.y <= self.origin.y + self.size.y
    }
}

/// RGBA color with 0.0..=1.0 components.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// Construct an opaque color from RGB byte components.
    pub const fn rgb_u8(r: u8, g: u8, b: u8) -> Self {
        Self::rgba_u8(r, g, b, 1.0)
    }

    /// Construct a color from RGB byte components plus a 0.0..=1.0 alpha.
    pub const fn rgba_u8(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a,
        }
    }

    pub const RED: Self = Self {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const GREEN: Self = Self {
        r: 0.0,
        g: 1.0,
        b: 0.0,
        a: 1.0,
    };
    pub const BLUE: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    };
    pub const BLACK: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const WHITE: Self = Self {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    pub fn to_jian(self) -> jian_core::scene::Color {
        fn ch(v: f32) -> u8 {
            (v.clamp(0.0, 1.0) * 255.0).round() as u8
        }
        jian_core::scene::Color::rgba(ch(self.r), ch(self.g), ch(self.b), ch(self.a))
    }
}

pub(crate) fn fold_alpha(c: Color, opacity: f32) -> Color {
    Color {
        a: c.a * opacity.clamp(0.0, 1.0),
        ..c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_is_closed_interval() {
        let r = Rect::xywh(10.0, 20.0, 100.0, 50.0);
        assert!(r.contains(Point2D::new(10.0, 20.0)));
        assert!(r.contains(Point2D::new(110.0, 70.0)));
        assert!(!r.contains(Point2D::new(9.99, 20.0)));
        assert!(!r.contains(Point2D::new(10.0, 70.01)));
    }

    #[test]
    fn with_alpha_overrides_only_alpha() {
        let c = Color::RED.with_alpha(0.5);
        assert_eq!((c.r, c.g, c.b, c.a), (1.0, 0.0, 0.0, 0.5));
    }

    #[test]
    fn byte_constructors_normalize_channels() {
        assert_eq!(
            Color::rgb_u8(255, 0, 128),
            Color {
                r: 1.0,
                g: 0.0,
                b: 128.0 / 255.0,
                a: 1.0,
            }
        );
        assert_eq!(Color::rgba_u8(0, 64, 255, 0.25).a, 0.25);
    }

    #[test]
    fn to_jian_round_trips_named_colors() {
        let j = Color::WHITE.to_jian();
        assert_eq!(j, jian_core::scene::Color::rgba(255, 255, 255, 255));
    }
}
