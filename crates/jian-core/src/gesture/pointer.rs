//! Unified PointerEvent — the single input type every host adapter produces.

use crate::geometry::Point;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PointerId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Touch,
    Mouse,
    Pen,
    Stylus,
    Trackpad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
    Hover,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct MouseButtons: u32 {
        const LEFT    = 1;
        const RIGHT   = 2;
        const MIDDLE  = 4;
        const BACK    = 8;
        const FORWARD = 16;
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Modifiers: u32 {
        const SHIFT = 1;
        const CTRL  = 2;
        const ALT   = 4;
        const CMD   = 8;
    }
}

#[derive(Debug, Clone)]
pub struct PointerEvent {
    pub id: PointerId,
    pub kind: PointerKind,
    pub phase: PointerPhase,
    pub position: Point,
    pub pressure: f32,
    pub buttons: MouseButtons,
    pub modifiers: Modifiers,
    pub tilt: Option<(f32, f32)>,
    pub timestamp: Instant,
}

impl PointerEvent {
    /// Minimal constructor used by host adapters and tests.
    pub fn simple(id: u32, phase: PointerPhase, position: Point) -> Self {
        Self {
            id: PointerId(id),
            kind: PointerKind::Touch,
            phase,
            position,
            pressure: 1.0,
            buttons: MouseButtons::LEFT,
            modifiers: Modifiers::empty(),
            tilt: None,
            timestamp: Instant::now(),
        }
    }
}

/// Mouse-wheel / two-finger-trackpad scroll. Scroll is **not** a
/// gesture-arena recognizer (no competition with Tap/Pan/etc.) —
/// hosts call `Runtime::dispatch_wheel` and the runtime hit-tests
/// directly to find the topmost node with `events.onScroll`. Delta
/// is in logical pixels per spec convention; positive Y = scroll up.
#[derive(Debug, Clone)]
pub struct WheelEvent {
    pub position: Point,
    pub delta: Point,
    pub modifiers: Modifiers,
    pub timestamp: Instant,
}

impl WheelEvent {
    /// Minimal constructor used by host adapters and tests.
    pub fn simple(position: Point, delta: Point) -> Self {
        Self {
            position,
            delta,
            modifiers: Modifiers::empty(),
            timestamp: Instant::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::point;

    #[test]
    fn pointer_event_clone() {
        let e = PointerEvent::simple(1, PointerPhase::Down, point(10.0, 20.0));
        let c = e.clone();
        assert_eq!(c.id, PointerId(1));
        assert_eq!(c.phase, PointerPhase::Down);
    }

    #[test]
    fn modifiers_flags() {
        let m = Modifiers::SHIFT | Modifiers::CTRL;
        assert!(m.contains(Modifiers::SHIFT));
        assert!(m.contains(Modifiers::CTRL));
        assert!(!m.contains(Modifiers::ALT));
    }

    #[test]
    fn mouse_buttons_default_empty() {
        let b: MouseButtons = Default::default();
        assert!(b.is_empty());
    }

    #[test]
    fn wheel_event_round_trip() {
        let w = WheelEvent::simple(point(10.0, 20.0), point(0.0, -5.0));
        assert_eq!(w.delta.x, 0.0);
        assert!(w.modifiers.is_empty());
    }
}
