//! `winit::event::WindowEvent` → `jian_core::gesture::PointerEvent`.
//!
//! The translator is a small state machine: we cache the last cursor
//! position because winit's `MouseInput` carries no coordinates and
//! `CursorMoved` needs to emit `PointerPhase::Move` only when a button
//! is held (otherwise it's `Hover`). Touch events are self-contained —
//! winit `Touch` carries position, phase, and finger id in one shot.

use jian_core::geometry::{point, Point};
use jian_core::gesture::{
    Modifiers as JianModifiers, MouseButtons as JianMouseButtons, PointerEvent, PointerId,
    PointerKind, PointerPhase,
};
use std::time::Instant;
use winit::event::{ElementState, MouseButton, Touch, TouchPhase, WindowEvent};
use winit::keyboard::ModifiersState;

/// Tracks the cursor state across winit events so we can synthesize
/// complete `PointerEvent`s from winit's scattered inputs.
#[derive(Debug, Clone, Default)]
pub struct PointerTranslator {
    pub cursor: Option<Point>,
    pub buttons: JianMouseButtons,
    pub modifiers: JianModifiers,
    /// Monotonic id used for mouse events (winit doesn't assign one).
    /// Touch events use the `Touch::id` directly.
    pub mouse_id: u32,
}

impl PointerTranslator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the cached modifier state from `WindowEvent::ModifiersChanged`.
    pub fn update_modifiers(&mut self, m: ModifiersState) {
        let mut out = JianModifiers::empty();
        if m.shift_key() {
            out |= JianModifiers::SHIFT;
        }
        if m.control_key() {
            out |= JianModifiers::CTRL;
        }
        if m.alt_key() {
            out |= JianModifiers::ALT;
        }
        if m.super_key() {
            out |= JianModifiers::CMD;
        }
        self.modifiers = out;
    }

    /// Translate a single `WindowEvent` into zero or one `PointerEvent`.
    /// The return value is optional because many winit events carry no
    /// pointer semantics (resize, focus, …).
    pub fn translate(&mut self, ev: &WindowEvent) -> Option<PointerEvent> {
        match ev {
            WindowEvent::CursorMoved { position, .. } => {
                let p = point(position.x as f32, position.y as f32);
                self.cursor = Some(p);
                let phase = if self.buttons.is_empty() {
                    PointerPhase::Hover
                } else {
                    PointerPhase::Move
                };
                Some(self.make_mouse_event(phase, p))
            }
            WindowEvent::CursorLeft { .. } => {
                let p = self.cursor.unwrap_or_else(|| point(0.0, 0.0));
                self.cursor = None;
                // Hover-leave is represented in the router by the absence
                // of a hit target; we still surface a Hover phase so a
                // node that was previously hovered sees one last tick.
                Some(self.make_mouse_event(PointerPhase::Hover, p))
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pos = self.cursor?;
                let flag = mouse_button_flag(*button);
                match state {
                    ElementState::Pressed => self.buttons |= flag,
                    ElementState::Released => self.buttons -= flag,
                }
                let phase = match state {
                    ElementState::Pressed => PointerPhase::Down,
                    ElementState::Released => PointerPhase::Up,
                };
                self.mouse_id = self.mouse_id.wrapping_add(1);
                Some(self.make_mouse_event(phase, pos))
            }
            WindowEvent::Touch(Touch {
                id,
                phase,
                location,
                ..
            }) => {
                let p = point(location.x as f32, location.y as f32);
                let jian_phase = match phase {
                    TouchPhase::Started => PointerPhase::Down,
                    TouchPhase::Moved => PointerPhase::Move,
                    TouchPhase::Ended => PointerPhase::Up,
                    TouchPhase::Cancelled => PointerPhase::Cancel,
                };
                Some(PointerEvent {
                    id: PointerId(*id as u32),
                    kind: PointerKind::Touch,
                    phase: jian_phase,
                    position: p,
                    pressure: 1.0,
                    buttons: JianMouseButtons::empty(),
                    modifiers: self.modifiers,
                    tilt: None,
                    timestamp: Instant::now(),
                })
            }
            _ => None,
        }
    }

    fn make_mouse_event(&self, phase: PointerPhase, position: Point) -> PointerEvent {
        PointerEvent {
            id: PointerId(self.mouse_id.max(1)),
            kind: PointerKind::Mouse,
            phase,
            position,
            pressure: 1.0,
            buttons: self.buttons,
            modifiers: self.modifiers,
            tilt: None,
            timestamp: Instant::now(),
        }
    }
}

fn mouse_button_flag(b: MouseButton) -> JianMouseButtons {
    match b {
        MouseButton::Left => JianMouseButtons::LEFT,
        MouseButton::Right => JianMouseButtons::RIGHT,
        MouseButton::Middle => JianMouseButtons::MIDDLE,
        MouseButton::Back => JianMouseButtons::BACK,
        MouseButton::Forward => JianMouseButtons::FORWARD,
        MouseButton::Other(_) => JianMouseButtons::empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;
    use winit::event::DeviceId;

    // winit::event::DeviceId::dummy isn't public in 0.30; use an
    // unsafe raw construction for tests. winit itself does this for
    // tests. Since the DeviceId is opaque data we just need any valid
    // instance.
    fn dummy_device_id() -> DeviceId {
        // Safety: DeviceId is #[non_exhaustive] / opaque; winit's public
        // examples use `unsafe { DeviceId::dummy() }` where available.
        // In 0.30 the public API only offers `DeviceId::dummy()` under
        // `cfg(test)` in winit's own crate; external crates can build
        // one by transmuting a zeroed placeholder. Since we only use
        // it as an opaque value and tests don't inspect it, we rely on
        // `DeviceId::dummy()` being available as of winit 0.30.5.
        DeviceId::dummy()
    }

    fn moved_event(x: f64, y: f64) -> WindowEvent {
        WindowEvent::CursorMoved {
            device_id: dummy_device_id(),
            position: PhysicalPosition::new(x, y),
        }
    }

    #[test]
    fn cursor_moved_without_button_is_hover() {
        let mut t = PointerTranslator::new();
        let ev = t.translate(&moved_event(10.0, 20.0)).unwrap();
        assert_eq!(ev.phase, PointerPhase::Hover);
        assert_eq!(ev.position, point(10.0, 20.0));
        assert_eq!(ev.kind, PointerKind::Mouse);
    }

    #[test]
    fn press_then_move_emits_move_phase() {
        let mut t = PointerTranslator::new();
        // Prime the cursor cache.
        let _ = t.translate(&moved_event(5.0, 5.0));
        let press = WindowEvent::MouseInput {
            device_id: dummy_device_id(),
            state: ElementState::Pressed,
            button: MouseButton::Left,
        };
        let down = t.translate(&press).unwrap();
        assert_eq!(down.phase, PointerPhase::Down);
        assert!(down.buttons.contains(JianMouseButtons::LEFT));

        let move_ev = t.translate(&moved_event(7.0, 7.0)).unwrap();
        assert_eq!(move_ev.phase, PointerPhase::Move);
        assert!(move_ev.buttons.contains(JianMouseButtons::LEFT));

        let release = WindowEvent::MouseInput {
            device_id: dummy_device_id(),
            state: ElementState::Released,
            button: MouseButton::Left,
        };
        let up = t.translate(&release).unwrap();
        assert_eq!(up.phase, PointerPhase::Up);
        assert!(up.buttons.is_empty());
    }

    #[test]
    fn mouse_without_cursor_cache_yields_none() {
        let mut t = PointerTranslator::new();
        let press = WindowEvent::MouseInput {
            device_id: dummy_device_id(),
            state: ElementState::Pressed,
            button: MouseButton::Left,
        };
        // No CursorMoved yet → no position known → skip event.
        assert!(t.translate(&press).is_none());
    }

    #[test]
    fn touch_events_roundtrip_phases() {
        let mut t = PointerTranslator::new();
        let ev = WindowEvent::Touch(Touch {
            device_id: dummy_device_id(),
            phase: TouchPhase::Started,
            location: PhysicalPosition::new(100.0, 200.0),
            force: None,
            id: 42,
        });
        let out = t.translate(&ev).unwrap();
        assert_eq!(out.id, PointerId(42));
        assert_eq!(out.phase, PointerPhase::Down);
        assert_eq!(out.kind, PointerKind::Touch);
    }

    #[test]
    fn modifiers_carry_into_translated_events() {
        let mut t = PointerTranslator::new();
        let mut mods = ModifiersState::empty();
        mods.insert(ModifiersState::SHIFT);
        t.update_modifiers(mods);
        let ev = t.translate(&moved_event(0.0, 0.0)).unwrap();
        assert!(ev.modifiers.contains(JianModifiers::SHIFT));
    }
}
