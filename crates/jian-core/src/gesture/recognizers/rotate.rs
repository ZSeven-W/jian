//! RotateRecognizer — two-pointer rotation geometry.
//!
//! Mirrors `ScaleRecognizer` but tracks the angle of the line
//! between the two pointers. `delta = current_angle - initial_angle`,
//! clamped to `(-π, π]` to keep small flips from registering as
//! near-2π jumps. Activation threshold is 5° (`PI / 36`).
//!
//! Like Scale, the recognizer is owned by `PointerRouter::multi`,
//! NOT a per-pointer arena. When it Claims, the router cancels the
//! per-pointer arenas it participated in.

use crate::document::NodeKey;
use crate::geometry::Point;
use crate::gesture::pointer::{PointerEvent, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::SemanticEvent;
use std::f32::consts::{PI, TAU};

/// Activation threshold = 5° (`PI / 36`). Plan 5 Task 9 / multi-
/// pointer plan §Recognizer state.
const ROTATE_ACTIVATION: f32 = PI / 36.0;

pub struct RotateRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    pids: Vec<(u32, Point)>,
    /// Initial angle (`atan2(b - a)`) sampled at the second Down.
    initial_angle: Option<f32>,
    started: bool,
    ended: bool,
}

impl RotateRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            pids: Vec::with_capacity(2),
            initial_angle: None,
            started: false,
            ended: false,
        }
    }

    fn angle(a: Point, b: Point) -> f32 {
        (b.y - a.y).atan2(b.x - a.x)
    }

    /// Wrap the unbounded difference back into `(-π, π]` so a small
    /// rotation across the 0/π boundary doesn't read as ~2π.
    fn wrap(delta: f32) -> f32 {
        let mut d = delta;
        while d > PI {
            d -= TAU;
        }
        while d <= -PI {
            d += TAU;
        }
        d
    }
}

impl Recognizer for RotateRecognizer {
    fn id(&self) -> RecognizerId {
        self.id
    }
    fn kind(&self) -> &'static str {
        "Rotate"
    }
    fn node(&self) -> NodeKey {
        self.node
    }
    fn state(&self) -> RecognizerState {
        self.state
    }

    fn handle_pointer(
        &mut self,
        event: &PointerEvent,
        arena: &mut ArenaHandle<'_>,
    ) -> RecognizerState {
        let pid = event.id.0;
        match event.phase {
            PointerPhase::Down => {
                if self.pids.iter().any(|(p, _)| *p == pid) || self.pids.len() >= 2 {
                    return self.state;
                }
                self.pids.push((pid, event.position));
                if self.pids.len() == 2 {
                    self.initial_angle = Some(Self::angle(self.pids[0].1, self.pids[1].1));
                }
            }
            PointerPhase::Move => {
                if let Some(slot) = self.pids.iter_mut().find(|(p, _)| *p == pid) {
                    slot.1 = event.position;
                } else {
                    return self.state;
                }
                if self.pids.len() < 2 {
                    return self.state;
                }
                let Some(initial) = self.initial_angle else {
                    return self.state;
                };
                let cur = Self::angle(self.pids[0].1, self.pids[1].1);
                let radians = Self::wrap(cur - initial);
                if !self.started {
                    if radians.abs() > ROTATE_ACTIVATION {
                        self.started = true;
                        self.state = RecognizerState::Claimed;
                        arena.emit(SemanticEvent::RotateStart { node: self.node });
                    }
                } else {
                    arena.emit(SemanticEvent::RotateUpdate {
                        node: self.node,
                        radians,
                    });
                }
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                let was_two = self.pids.len() == 2;
                self.pids.retain(|(p, _)| *p != pid);
                if was_two && self.started && !self.ended {
                    self.ended = true;
                    arena.emit(SemanticEvent::RotateEnd { node: self.node });
                }
                if self.pids.is_empty() {
                    self.initial_angle = None;
                    self.started = false;
                    self.ended = false;
                }
            }
            PointerPhase::Hover => {}
        }
        self.state
    }

    fn accept(&mut self, _: &mut ArenaHandle<'_>) {
        self.state = RecognizerState::Claimed;
    }
    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::point;
    use slotmap::SlotMap;

    fn make_key() -> NodeKey {
        let mut sm: SlotMap<NodeKey, u32> = SlotMap::with_key();
        sm.insert(0)
    }

    fn dispatch(r: &mut RotateRecognizer, ev: PointerEvent) -> Option<SemanticEvent> {
        let mut pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        let _ = r.handle_pointer(&ev, &mut h);
        pending
    }

    #[test]
    fn rotate_starts_possible_and_stays() {
        let node = make_key();
        let r = RotateRecognizer::new(1, node);
        assert_eq!(r.state(), RecognizerState::Possible);
        assert_eq!(r.kind(), "Rotate");
    }

    #[test]
    fn rotate_claims_past_5_degrees() {
        let node = make_key();
        let mut r = RotateRecognizer::new(1, node);
        // Initial: a (0,0), b (100,0) → angle 0°.
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Down, point(0.0, 0.0)),
        );
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Down, point(100.0, 0.0)),
        );
        // Move b to (100, 4) ≈ 2.3° — under threshold.
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(100.0, 4.0)),
        );
        assert!(ev.is_none());
        assert_eq!(r.state(), RecognizerState::Possible);
        // Move b to (100, 10) ≈ 5.7° — claims.
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(100.0, 10.0)),
        );
        assert!(matches!(ev, Some(SemanticEvent::RotateStart { .. })));
        assert_eq!(r.state(), RecognizerState::Claimed);
    }

    #[test]
    fn rotate_update_carries_signed_radians() {
        let node = make_key();
        let mut r = RotateRecognizer::new(1, node);
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Down, point(0.0, 0.0)),
        );
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Down, point(100.0, 0.0)),
        );
        // Cross threshold +y direction (clockwise = positive in screen coords).
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(100.0, 20.0)),
        );
        // Now b is at (100, 50) → angle ≈ 0.4636 rad.
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(100.0, 50.0)),
        );
        match ev {
            Some(SemanticEvent::RotateUpdate { radians, .. }) => {
                assert!(radians > 0.4 && radians < 0.5, "got {radians}");
            }
            other => panic!("expected RotateUpdate, got {other:?}"),
        }
    }

    #[test]
    fn rotate_wrap_keeps_small_flips_small() {
        // Initial angle ≈ π (b on the -x side). A small CCW move flips
        // the angle to just under -π. Without wrap, the delta would be
        // ~ -2π; with wrap, the recognizer reports a small positive delta.
        let node = make_key();
        let mut r = RotateRecognizer::new(1, node);
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Down, point(100.0, 0.0)),
        );
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Down, point(0.0, 0.0)),
        );
        // Move b slightly so the line angle goes from +π to just past
        // (still ≈ ±π). Move to (0, -1) → angle ≈ π + tiny.
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(0.0, -10.0)),
        );
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(0.0, -20.0)),
        );
        // Whatever the exact delta, |radians| must be small (< 1 rad).
        if let Some(SemanticEvent::RotateUpdate { radians, .. }) = ev {
            assert!(
                radians.abs() < 1.0,
                "wrap should keep small flips small; got {radians}"
            );
        }
    }

    #[test]
    fn rotate_end_on_first_up_after_start() {
        let node = make_key();
        let mut r = RotateRecognizer::new(1, node);
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Down, point(0.0, 0.0)),
        );
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Down, point(100.0, 0.0)),
        );
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(100.0, 50.0)),
        );
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Up, point(100.0, 50.0)),
        );
        assert!(matches!(ev, Some(SemanticEvent::RotateEnd { .. })));
    }
}
