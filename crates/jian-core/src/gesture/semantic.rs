//! SemanticEvent — the normalized output of the gesture pipeline.
//!
//! Consumed by `EventDispatcher` which maps each variant to the corresponding
//! schema `events.*` ActionList and executes it through Plan 4.

use super::pointer::Modifiers;
use crate::document::NodeKey;
use crate::geometry::Point;

#[derive(Debug, Clone)]
pub enum SemanticEvent {
    Tap {
        node: NodeKey,
        position: Point,
    },
    DoubleTap {
        node: NodeKey,
        position: Point,
    },
    LongPress {
        node: NodeKey,
        position: Point,
        duration_ms: u32,
    },
    PanStart {
        node: NodeKey,
        position: Point,
    },
    PanUpdate {
        node: NodeKey,
        delta: Point,
        velocity: Point,
    },
    PanEnd {
        node: NodeKey,
        velocity: Point,
    },
    ScaleStart {
        node: NodeKey,
        focal: Point,
    },
    ScaleUpdate {
        node: NodeKey,
        scale: f32,
        focal: Point,
    },
    ScaleEnd {
        node: NodeKey,
    },
    RotateStart {
        node: NodeKey,
    },
    RotateUpdate {
        node: NodeKey,
        radians: f32,
    },
    RotateEnd {
        node: NodeKey,
    },
    Scroll {
        node: NodeKey,
        delta: Point,
    },
    HoverEnter {
        node: NodeKey,
        position: Point,
    },
    HoverLeave {
        node: NodeKey,
        position: Point,
    },
    KeyDown {
        node: NodeKey,
        key: String,
        modifiers: Modifiers,
    },
    /// Raw escape-hatch — delivered when an ancestor sets `gestures.rawPointer`.
    RawPointer {
        node: NodeKey,
        phase: super::pointer::PointerPhase,
        position: Point,
    },
}

impl SemanticEvent {
    /// Return the target node this event routes to.
    pub fn node(&self) -> NodeKey {
        match self {
            Self::Tap { node, .. }
            | Self::DoubleTap { node, .. }
            | Self::LongPress { node, .. }
            | Self::PanStart { node, .. }
            | Self::PanUpdate { node, .. }
            | Self::PanEnd { node, .. }
            | Self::ScaleStart { node, .. }
            | Self::ScaleUpdate { node, .. }
            | Self::ScaleEnd { node }
            | Self::RotateStart { node }
            | Self::RotateUpdate { node, .. }
            | Self::RotateEnd { node }
            | Self::Scroll { node, .. }
            | Self::HoverEnter { node, .. }
            | Self::HoverLeave { node, .. }
            | Self::KeyDown { node, .. }
            | Self::RawPointer { node, .. } => *node,
        }
    }

    /// Return the matching `events.*` handler name used by the schema
    /// (Plan 1 `EventHandlers` field naming, snake_case after serde).
    pub fn handler_key(&self) -> &'static str {
        match self {
            Self::Tap { .. } => "onTap",
            Self::DoubleTap { .. } => "onDoubleTap",
            Self::LongPress { .. } => "onLongPress",
            Self::PanStart { .. } => "onPanStart",
            Self::PanUpdate { .. } => "onPanUpdate",
            Self::PanEnd { .. } => "onPanEnd",
            Self::ScaleStart { .. } => "onScaleStart",
            Self::ScaleUpdate { .. } => "onScaleUpdate",
            Self::ScaleEnd { .. } => "onScaleEnd",
            Self::RotateStart { .. } | Self::RotateUpdate { .. } | Self::RotateEnd { .. } => {
                "onRotate"
            }
            Self::Scroll { .. } => "onScroll",
            Self::HoverEnter { .. } => "onHoverEnter",
            Self::HoverLeave { .. } => "onHoverLeave",
            Self::KeyDown { .. } => "onKey",
            Self::RawPointer { .. } => "onRawPointer",
        }
    }
}
