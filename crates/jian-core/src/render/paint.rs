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

#[derive(Debug, Clone)]
pub struct TextRun {
    pub content: String,
    pub font_family: String,
    pub font_size: f32,
    pub color: Color,
    pub origin: Point,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageHandle(pub u64);

/// A single colour stop in a gradient, `offset` in `[0.0, 1.0]`.
#[derive(Debug, Clone, Copy)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}

/// Gradient fill description — a linear sweep across the target rect
/// at `angle_deg` (0° = left-to-right, 90° = top-to-bottom). MVP
/// supports only linear; radial can join as a sibling variant.
#[derive(Debug, Clone)]
pub struct LinearGradient {
    pub angle_deg: f32,
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
        image: ImageHandle,
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
    /// A rounded rect with an outer drop shadow drawn underneath. The
    /// paint / gradient layer draws on top of the blur. Emitted when a
    /// node has `effects: [{ type: "shadow", ... }]`.
    ShadowedRect {
        rect: Rect,
        radii: BorderRadii,
        shadow: ShadowSpec,
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
