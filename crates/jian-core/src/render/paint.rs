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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::point;

    #[test]
    fn path_commands_build() {
        let cmds = vec![
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
