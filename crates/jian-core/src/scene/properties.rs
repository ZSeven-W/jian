//! Post-resolution visual properties attached to each SceneNode.
//!
//! At this layer we only canonicalise simple fields (color hex strings,
//! numeric sizes). Complex paints (gradients, images) are passed through
//! as-is to the RenderBackend trait, which knows how to render them.

use jian_ops_schema::style::{PenEffect, PenFill, PenStroke};

/// Opaque 32-bit color in RGBA order. Use [`Color::from_hex`] to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color(pub u32);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color(u32::from_be_bytes([r, g, b, 0xff]))
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color(u32::from_be_bytes([r, g, b, a]))
    }
    pub fn r(&self) -> u8 {
        ((self.0 >> 24) & 0xff) as u8
    }
    pub fn g(&self) -> u8 {
        ((self.0 >> 16) & 0xff) as u8
    }
    pub fn b(&self) -> u8 {
        ((self.0 >> 8) & 0xff) as u8
    }
    pub fn a(&self) -> u8 {
        (self.0 & 0xff) as u8
    }

    /// Parse `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`. Returns None on failure.
    pub fn from_hex(s: &str) -> Option<Self> {
        let h = s.strip_prefix('#')?;
        let (r, g, b, a) = match h.len() {
            3 => {
                let r = u8::from_str_radix(&h[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&h[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&h[2..3].repeat(2), 16).ok()?;
                (r, g, b, 0xff)
            }
            4 => {
                let r = u8::from_str_radix(&h[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&h[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&h[2..3].repeat(2), 16).ok()?;
                let a = u8::from_str_radix(&h[3..4].repeat(2), 16).ok()?;
                (r, g, b, a)
            }
            6 => {
                let r = u8::from_str_radix(&h[0..2], 16).ok()?;
                let g = u8::from_str_radix(&h[2..4], 16).ok()?;
                let b = u8::from_str_radix(&h[4..6], 16).ok()?;
                (r, g, b, 0xff)
            }
            8 => {
                let r = u8::from_str_radix(&h[0..2], 16).ok()?;
                let g = u8::from_str_radix(&h[2..4], 16).ok()?;
                let b = u8::from_str_radix(&h[4..6], 16).ok()?;
                let a = u8::from_str_radix(&h[6..8], 16).ok()?;
                (r, g, b, a)
            }
            _ => return None,
        };
        Some(Color::rgba(r, g, b, a))
    }
}

/// Resolved wrapper. For now the runtime keeps the schema types by value;
/// future optimisation: pre-compute per-draw-call GPU-ready structures.
#[derive(Debug, Clone, Default)]
pub struct ResolvedVisual {
    pub fills: Vec<PenFill>,
    pub stroke: Option<PenStroke>,
    pub effects: Vec<PenEffect>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_short() {
        let c = Color::from_hex("#f00").unwrap();
        assert_eq!(c, Color::rgba(0xff, 0x00, 0x00, 0xff));
    }

    #[test]
    fn hex_full_with_alpha() {
        let c = Color::from_hex("#12345678").unwrap();
        assert_eq!(c, Color::rgba(0x12, 0x34, 0x56, 0x78));
    }

    #[test]
    fn hex_invalid() {
        assert!(Color::from_hex("#zzz").is_none());
        assert!(Color::from_hex("abc").is_none());
        assert!(Color::from_hex("#12").is_none());
    }
}
