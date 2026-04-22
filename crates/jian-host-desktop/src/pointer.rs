//! `winit::event::WindowEvent` → `jian_core::gesture::PointerEvent`.
//!
//! The translator is a small state machine:
//!
//! - Cache both the *current* cursor (`None` when outside the window
//!   after `CursorLeft`) and a *last-known* position so a mouse
//!   release that happens outside the window still produces a valid
//!   terminating `Up` event — core's arena needs that matching
//!   `Up`/`Cancel` to tear the gesture down cleanly.
//! - Track the set of currently held buttons so a multi-button drag
//!   (press-left → press-right → release-left) doesn't tear the
//!   gesture down on the first release; only emit `Up` when every
//!   button is released.
//! - Assign a fresh `PointerId` on each new gesture (empty button set →
//!   first press) and keep it stable through every subsequent `Move` /
//!   additional press / `Up` in the same gesture, so the core arena's
//!   per-pointer lookups don't strand mid-drag.
//!
//! Touch events are self-contained: winit's `Touch` carries position,
//! phase, and finger id in one shot.

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
    /// Current cursor position; `None` while the cursor is outside the
    /// window (after `CursorLeft`).
    pub cursor: Option<Point>,
    /// Last known cursor position — kept even after `CursorLeft` so a
    /// release outside the window still has a position to report.
    pub last_known_cursor: Point,
    pub buttons: JianMouseButtons,
    pub modifiers: JianModifiers,
    /// Pointer id for the current mouse gesture. Bumped on the
    /// first `Down` of a new gesture (when no buttons were held),
    /// stable through the rest of the gesture.
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
                self.last_known_cursor = p;
                let phase = if self.buttons.is_empty() {
                    PointerPhase::Hover
                } else {
                    PointerPhase::Move
                };
                Some(self.make_mouse_event(phase, p))
            }
            WindowEvent::CursorLeft { .. } => {
                let p = self.last_known_cursor;
                self.cursor = None;
                // Hover-leave is represented in the router by the absence
                // of a hit target; we still surface a Hover phase so a
                // node that was previously hovered sees one last tick.
                // `last_known_cursor` stays intact so a subsequent
                // MouseInput (release outside the window) still fires.
                Some(self.make_mouse_event(PointerPhase::Hover, p))
            }
            WindowEvent::MouseInput { state, button, .. } => {
                // Use `cursor` when available, fall back to
                // `last_known_cursor` so a release after `CursorLeft`
                // still completes the gesture.
                let pos = self.cursor.unwrap_or(self.last_known_cursor);
                let flag = mouse_button_flag(*button);
                let phase = match state {
                    ElementState::Pressed => {
                        let starting_new_gesture = self.buttons.is_empty();
                        self.buttons |= flag;
                        if starting_new_gesture {
                            // Fresh gesture — bump the pointer id so the
                            // core arena sees a clean Down for this
                            // session. Subsequent Move / Up reuse the
                            // same id.
                            self.mouse_id = self.mouse_id.wrapping_add(1).max(1);
                            PointerPhase::Down
                        } else {
                            // Additional button pressed mid-gesture; the
                            // gesture's pointer id doesn't change so
                            // arena lookups keep working. Surface as
                            // Move with the updated buttons bitset.
                            PointerPhase::Move
                        }
                    }
                    ElementState::Released => {
                        self.buttons.remove(flag);
                        if self.buttons.is_empty() {
                            PointerPhase::Up
                        } else {
                            // Other buttons still held — don't tear
                            // down the gesture. Emit Move so observers
                            // see the updated buttons bitset.
                            PointerPhase::Move
                        }
                    }
                };
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
                // winit finger id is u64; jian PointerId is u32.
                // Wraparound yields a best-effort identifier; widening
                // the core type is tracked as a follow-up item.
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

    // `DeviceId::dummy()` is a public `const fn` since winit 0.30.4 —
    // fine to call directly from tests of downstream crates.
    fn dummy_device_id() -> DeviceId {
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
    fn mouse_without_cursor_cache_uses_origin_fallback() {
        // Before any CursorMoved event the translator falls back to
        // the `last_known_cursor` default `(0, 0)` so core still gets
        // a terminating Up/Down rather than silently dropping the
        // event. A pristine Down at the origin is an acceptable
        // starting state for the arena.
        let mut t = PointerTranslator::new();
        let press = WindowEvent::MouseInput {
            device_id: dummy_device_id(),
            state: ElementState::Pressed,
            button: MouseButton::Left,
        };
        let ev = t.translate(&press).expect("fallback to origin");
        assert_eq!(ev.phase, PointerPhase::Down);
        assert_eq!(ev.position.x, 0.0);
        assert_eq!(ev.position.y, 0.0);
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

    // --- Codex-review regression tests ---

    #[test]
    fn pointer_id_is_stable_through_a_drag() {
        // Down → Move → Up must all share the same PointerId so the
        // core arena can track the gesture end-to-end.
        let mut t = PointerTranslator::new();
        let _ = t.translate(&moved_event(5.0, 5.0));
        let down = t
            .translate(&WindowEvent::MouseInput {
                device_id: dummy_device_id(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            })
            .unwrap();
        let mv = t.translate(&moved_event(10.0, 10.0)).unwrap();
        let up = t
            .translate(&WindowEvent::MouseInput {
                device_id: dummy_device_id(),
                state: ElementState::Released,
                button: MouseButton::Left,
            })
            .unwrap();
        assert_eq!(down.id, mv.id);
        assert_eq!(mv.id, up.id);
    }

    #[test]
    fn release_outside_window_still_fires_up() {
        // Press inside, CursorLeft, then Release — the Release must
        // still produce an `Up` so the arena tears down cleanly.
        let mut t = PointerTranslator::new();
        let _ = t.translate(&moved_event(5.0, 5.0));
        let _ = t.translate(&WindowEvent::MouseInput {
            device_id: dummy_device_id(),
            state: ElementState::Pressed,
            button: MouseButton::Left,
        });
        let _ = t.translate(&WindowEvent::CursorLeft {
            device_id: dummy_device_id(),
        });
        let up = t
            .translate(&WindowEvent::MouseInput {
                device_id: dummy_device_id(),
                state: ElementState::Released,
                button: MouseButton::Left,
            })
            .expect("release after CursorLeft must still translate");
        assert_eq!(up.phase, PointerPhase::Up);
        // Uses the last-known position, not (0, 0).
        assert_eq!(up.position.x, 5.0);
    }

    #[test]
    fn multi_button_release_keeps_gesture_alive() {
        // Press left + right; release left — must NOT emit Up (right
        // is still held). Releasing right then emits the Up.
        let mut t = PointerTranslator::new();
        let _ = t.translate(&moved_event(0.0, 0.0));
        let down_left = t
            .translate(&WindowEvent::MouseInput {
                device_id: dummy_device_id(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            })
            .unwrap();
        assert_eq!(down_left.phase, PointerPhase::Down);
        let gesture_id = down_left.id;

        let down_right = t
            .translate(&WindowEvent::MouseInput {
                device_id: dummy_device_id(),
                state: ElementState::Pressed,
                button: MouseButton::Right,
            })
            .unwrap();
        // Additional button mid-gesture → Move, not Down.
        assert_eq!(down_right.phase, PointerPhase::Move);
        assert_eq!(down_right.id, gesture_id);
        assert!(down_right.buttons.contains(JianMouseButtons::LEFT));
        assert!(down_right.buttons.contains(JianMouseButtons::RIGHT));

        let release_left = t
            .translate(&WindowEvent::MouseInput {
                device_id: dummy_device_id(),
                state: ElementState::Released,
                button: MouseButton::Left,
            })
            .unwrap();
        // Other button still held → Move, not Up.
        assert_eq!(release_left.phase, PointerPhase::Move);
        assert_eq!(release_left.id, gesture_id);
        assert!(!release_left.buttons.contains(JianMouseButtons::LEFT));
        assert!(release_left.buttons.contains(JianMouseButtons::RIGHT));

        let release_right = t
            .translate(&WindowEvent::MouseInput {
                device_id: dummy_device_id(),
                state: ElementState::Released,
                button: MouseButton::Right,
            })
            .unwrap();
        assert_eq!(release_right.phase, PointerPhase::Up);
        assert_eq!(release_right.id, gesture_id);
        assert!(release_right.buttons.is_empty());
    }

    #[test]
    fn fresh_gesture_bumps_pointer_id() {
        let mut t = PointerTranslator::new();
        let _ = t.translate(&moved_event(0.0, 0.0));
        let g1_down = t
            .translate(&WindowEvent::MouseInput {
                device_id: dummy_device_id(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            })
            .unwrap();
        let _g1_up = t.translate(&WindowEvent::MouseInput {
            device_id: dummy_device_id(),
            state: ElementState::Released,
            button: MouseButton::Left,
        });
        let g2_down = t
            .translate(&WindowEvent::MouseInput {
                device_id: dummy_device_id(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            })
            .unwrap();
        assert_ne!(g1_down.id, g2_down.id);
    }
}
