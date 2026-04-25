//! Backend-agnostic paint + path primitives.

use crate::geometry::{Point, Rect};
use crate::scene::Color;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderRadii {
    pub tl: f32,
    pub tr: f32,
    pub br: f32,
    pub bl: f32,
}

impl BorderRadii {
    pub fn uniform(v: f32) -> Self {
        Self {
            tl: v,
            tr: v,
            br: v,
            bl: v,
        }
    }
    pub fn zero() -> Self {
        Self::uniform(0.0)
    }
}

#[derive(Debug, Clone)]
pub struct Paint {
    pub fill: Option<Color>,
    pub stroke: Option<StrokeOp>,
    pub opacity: f32,
}

impl Paint {
    pub fn solid(color: Color) -> Self {
        Self {
            fill: Some(color),
            stroke: None,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StrokeOp {
    pub color: Color,
    pub width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCommand {
    MoveTo(Point),
    LineTo(Point),
    QuadTo(Point, Point),
    CubicTo(Point, Point, Point),
    Close,
}

#[derive(Debug, Clone)]
pub struct ShadowSpec {
    pub color: Color,
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    pub spread: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Start,
    Center,
    End,
}

#[derive(Debug, Clone)]
pub struct TextRun {
    pub content: String,
    pub font_family: String,
    pub font_size: f32,
    /// CSS-weight number. 400 = Normal, 700 = Bold.
    pub font_weight: u16,
    pub color: Color,
    /// Top-left of the **containing box** — backend derives the
    /// baseline and horizontal alignment from here.
    pub origin: Point,
    /// Container width (for centering / right-align). `0.0` means
    /// "unknown; render at origin with no alignment adjustment."
    pub max_width: f32,
    pub align: TextAlign,
    /// CSS-ish line-height multiplier (`font_size * line_height`).
    /// 0 means "default".
    pub line_height: f32,
}

/// Where the bytes of an image come from.
///
/// Backends decode + cache by source. `DataUrl` carries an inline
/// `data:image/...;base64,...` URL — fast path that needs no host
/// resolver. `Bytes` carries pre-resolved bytes (e.g. zip-extracted
/// asset). `Url` is a host-resolved reference; backends that cannot
/// fetch it draw a placeholder + warn.
#[derive(Debug, Clone)]
pub enum ImageSource {
    DataUrl(String),
    Bytes(std::sync::Arc<Vec<u8>>),
    Url(String),
}

impl ImageSource {
    /// Stable, content-addressed cache key.
    ///
    /// `DataUrl` / `Url` use the string verbatim — already stable
    /// across allocations and across runs.
    ///
    /// `Bytes` keys by FNV-1a 64-bit content hash + length. Pointer
    /// addresses are *not* stable: an Arc that gets dropped after
    /// the cache key is computed can have its memory reused by a
    /// different `Bytes(...)` payload, returning the wrong cached
    /// image. Hashing the bytes is O(N) but only happens on the
    /// first insert per source.
    pub fn cache_key(&self) -> String {
        match self {
            Self::DataUrl(s) | Self::Url(s) => s.clone(),
            Self::Bytes(b) => {
                let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                for byte in b.iter() {
                    h ^= *byte as u64;
                    h = h.wrapping_mul(0x100_0000_01b3);
                }
                format!("bytes:{:016x}:{}", h, b.len())
            }
        }
    }
}

/// A single colour stop in a gradient, `offset` in `[0.0, 1.0]`.
#[derive(Debug, Clone, Copy)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}

/// Gradient fill description — a linear sweep across the target rect
/// at `angle_deg` (0° = left-to-right, 90° = top-to-bottom).
#[derive(Debug, Clone)]
pub struct LinearGradient {
    pub angle_deg: f32,
    pub stops: Vec<GradientStop>,
    pub opacity: f32,
}

/// Radial gradient description.
///
/// `cx` / `cy` are normalised within the target rect ([0, 1] — 0.5 = centre).
/// `radius` is a fraction of `max(width, height)` (matching the OpenPencil
/// TS convention used by pen-renderer's `node-renderer.ts`).
#[derive(Debug, Clone)]
pub struct RadialGradient {
    pub cx: f32,
    pub cy: f32,
    pub radius: f32,
    pub stops: Vec<GradientStop>,
    pub opacity: f32,
}

/// A self-contained drawing operation issued by the scene walker to the backend.
#[derive(Debug, Clone)]
pub enum DrawOp {
    Rect {
        rect: Rect,
        paint: Paint,
    },
    RoundedRect {
        rect: Rect,
        radii: BorderRadii,
        paint: Paint,
    },
    Path {
        commands: Vec<PathCommand>,
        paint: Paint,
    },
    Image {
        source: ImageSource,
        dst: Rect,
        opacity: f32,
    },
    Text(TextRun),
    /// Rounded rect with a linear gradient fill (and optional stroke).
    /// `radii` may be `BorderRadii::zero()` for a plain rect. Emitted
    /// directly by the scene walker for nodes whose `fill[]` starts
    /// with a `linear_gradient` entry.
    LinearGradientRect {
        rect: Rect,
        radii: BorderRadii,
        gradient: LinearGradient,
        stroke: Option<StrokeOp>,
    },
    /// Rounded rect with a radial gradient fill. Sibling to
    /// `LinearGradientRect`; emitted for nodes whose `fill[]` starts
    /// with a `radial_gradient` entry.
    RadialGradientRect {
        rect: Rect,
        radii: BorderRadii,
        gradient: RadialGradient,
        stroke: Option<StrokeOp>,
    },
    /// A rounded rect with an outer drop shadow drawn underneath. The
    /// paint / gradient layer draws on top of the blur. Emitted when a
    /// node has `effects: [{ type: "shadow", ... }]`.
    ShadowedRect {
        rect: Rect,
        radii: BorderRadii,
        shadow: ShadowSpec,
    },
    /// Vector icon (Lucide / Feather / bundled family) rendered by a
    /// name lookup in the backend's glyph table, scaled into the given
    /// rect and painted in `color`.
    Icon {
        rect: Rect,
        name: String,
        family: Option<String>,
        color: Color,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::point;

    #[test]
    fn path_commands_build() {
        let cmds = [
            PathCommand::MoveTo(point(0.0, 0.0)),
            PathCommand::LineTo(point(10.0, 0.0)),
            PathCommand::LineTo(point(10.0, 10.0)),
            PathCommand::Close,
        ];
        assert_eq!(cmds.len(), 4);
    }

    #[test]
    fn border_radii_uniform() {
        let b = BorderRadii::uniform(4.0);
        assert_eq!(b.tl, 4.0);
        assert_eq!(b.br, 4.0);
    }
}
