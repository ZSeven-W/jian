//! Colour conversion — `jian_core::scene::Color` → `skia_safe::Color4f`.

use jian_core::scene::Color;
use skia_safe::Color4f;

pub fn to_sk_color(c: Color) -> Color4f {
    Color4f {
        r: c.r() as f32 / 255.0,
        g: c.g() as f32 / 255.0,
        b: c.b() as f32 / 255.0,
        a: c.a() as f32 / 255.0,
    }
}

pub fn to_sk_color_u32(argb: u32) -> skia_safe::Color {
    skia_safe::Color::new(argb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_blue_roundtrips() {
        let c = Color::rgb(0x1e, 0x88, 0xe5);
        let sk = to_sk_color(c);
        assert!((sk.r - 0x1e as f32 / 255.0).abs() < f32::EPSILON);
        assert!((sk.a - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn argb_u32_to_skia_color() {
        let c = to_sk_color_u32(0xff_1e_88_e5);
        assert_eq!(c.a(), 0xff);
    }
}
