//! Unified PointerEvent — the single input type every host adapter produces.

use crate::geometry::Point;
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PointerId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerKind {
    Touch,
    Mouse,
    Pen,
    Stylus,
    Trackpad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
    Hover,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub struct MouseButtons: u32 {
        const LEFT    = 1;
        const RIGHT   = 2;
        const MIDDLE  = 4;
        const BACK    = 8;
        const FORWARD = 16;
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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
/// is in `mode` units (Pixel default; Line / Page only when the host
/// surfaces a non-pixel `deltaMode`).
///
/// **Sign convention (Jian-internal, mirrors winit's `MouseScrollDelta`):**
/// positive `delta.y` = user scrolled **up** (content moves up to
/// reveal what's above). This matches `winit::event::MouseScrollDelta`
/// passthrough on desktop and the existing `scroll_verb` action DSL.
///
/// Browser hosts must flip the sign of `WheelEvent.deltaY` (which is
/// W3C-positive-**down**) before constructing this struct. shell-web's
/// event/pointer.rs translator owns that sign flip; widget code reads
/// the Jian-internal convention.
#[derive(Debug, Clone)]
pub struct WheelEvent {
    pub position: Point,
    pub delta: Point,
    /// Mirror of W3C `WheelEvent.deltaZ`. Almost always `0.0` (3-axis
    /// devices are rare). Kept as a separate field instead of
    /// promoting `delta` to a 3-vector to keep the 2D-scroll-only call
    /// sites unchanged. Sign follows Jian's positive-up convention
    /// (per `delta.y`) when the host surfaces a non-zero value.
    pub delta_z: f32,
    /// Mirror of W3C `WheelEvent.deltaMode` — Pixel (0) / Line (1) /
    /// Page (2). Native hosts emit Pixel; web host fills from
    /// `WheelEvent.deltaMode`. Note: `mode` describes the unit of
    /// `delta`'s magnitude; the sign of `delta` follows Jian's
    /// internal convention (above), not the browser's deltaY sign.
    pub mode: ScrollMode,
    pub modifiers: Modifiers,
    pub timestamp: Instant,
}

/// Mirror of W3C `WheelEvent.deltaMode`. Step 1b shell-web sets this
/// from the browser's deltaMode field; native winit emits `Pixel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ScrollMode {
    #[default]
    Pixel,
    Line,
    Page,
}

impl WheelEvent {
    /// Minimal constructor used by host adapters and tests. Defaults
    /// `mode` to `ScrollMode::Pixel` (W3C `deltaMode = 0`) and
    /// `delta_z` to `0.0` (no z-axis scroll).
    pub fn simple(position: Point, delta: Point) -> Self {
        Self {
            position,
            delta,
            delta_z: 0.0,
            mode: ScrollMode::Pixel,
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
