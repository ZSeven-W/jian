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
use crate::gesture::semantic::{GestureFacts, PointerFacts, SemanticEvent};
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
    /// Last reported rotation (radians, for per-frame delta).
    last_radians: Option<f32>,
    /// Last reported focal point (midpoint), for end payloads.
    last_focal: Option<Point>,
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
            last_radians: None,
            last_focal: None,
        }
    }

    fn angle(a: Point, b: Point) -> f32 {
        (b.y - a.y).atan2(b.x - a.x)
    }

    fn midpoint(a: Point, b: Point) -> Point {
        crate::geometry::point((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
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
                    // Regaining the two-finger quorum opens a FRESH
                    // session (R2B2 2→1→2 contract): new baseline, and
                    // Start/End symmetry requires a new Possible→
                    // Claimed edge so the router re-runs preflight and
                    // a new `RotateStart` can fire.
                    self.initial_angle = Some(Self::angle(self.pids[0].1, self.pids[1].1));
                    self.state = RecognizerState::Possible;
                    self.started = false;
                    self.ended = false;
                    self.last_radians = None;
                    self.last_focal = None;
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
                let focal = Self::midpoint(self.pids[0].1, self.pids[1].1);
                if !self.started {
                    if radians.abs() > ROTATE_ACTIVATION {
                        self.started = true;
                        self.state = RecognizerState::Claimed;
                        self.last_radians = Some(radians);
                        self.last_focal = Some(focal);
                        let facts = PointerFacts::from_event(event);
                        arena.emit_with(
                            SemanticEvent::RotateStart { node: self.node },
                            facts,
                            GestureFacts {
                                // The threshold-crossing rotation itself;
                                // delta vs. the initial angle (0) equals it.
                                rotation: Some(radians),
                                delta_rotation: Some(radians),
                                focal: Some(focal),
                                ..Default::default()
                            },
                        );
                    }
                } else {
                    // Per-frame delta wraps across the ±π boundary so a
                    // small rotation near the seam never reads as ~2π.
                    let delta_rotation = self.last_radians.map(|prev| Self::wrap(radians - prev));
                    self.last_radians = Some(radians);
                    self.last_focal = Some(focal);
                    let facts = PointerFacts::from_event(event);
                    arena.emit_with(
                        SemanticEvent::RotateUpdate {
                            node: self.node,
                            radians,
                        },
                        facts,
                        GestureFacts {
                            rotation: Some(radians),
                            delta_rotation,
                            focal: Some(focal),
                            ..Default::default()
                        },
                    );
                }
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                // Only a TRACKED pointer participates in this teardown.
                // An untracked pointer must be a pure no-op — a third
                // finger's Up may never end someone else's gesture.
                // R2B2 third-finger contract.
                if !self.pids.iter().any(|(p, _)| *p == pid) {
                    return self.state;
                }
                self.pids.retain(|(p, _)| *p != pid);
                if self.started && !self.ended && self.pids.len() < 2 {
                    // Dropping below the two-finger quorum terminates
                    // THIS session symmetrically and arms the next Down
                    // pair for a fresh session with a freshly sampled
                    // baseline. R2B2 2→1→2 fix: previously the instance
                    // kept `started` true and a re-grab emitted Updates
                    // with stale deltas and no Start.
                    self.ended = true;
                    let facts = PointerFacts::from_event(event);
                    arena.emit_with(
                        SemanticEvent::RotateEnd { node: self.node },
                        facts,
                        GestureFacts {
                            rotation: self.last_radians,
                            focal: self.last_focal,
                            ..Default::default()
                        },
                    );
                }
                if self.pids.is_empty() {
                    self.initial_angle = None;
                    self.started = false;
                    self.ended = false;
                    self.last_radians = None;
                    self.last_focal = None;
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
    fn has_participant_capacity(&self) -> bool {
        self.pids.len() < 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::point;
    use crate::gesture::semantic::SemanticEventEnvelope;
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
        pending.map(|e| e.event)
    }

    /// Like `dispatch` but keeps the envelope so gesture facts can be
    /// asserted (exact rotation / deltaRotation values).
    fn dispatch_env(r: &mut RotateRecognizer, ev: PointerEvent) -> Option<SemanticEventEnvelope> {
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
    fn rotate_start_carries_actual_crossing_rotation() {
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
        // Crossing Move: b at (100, 10) → atan2(10, 100) ≈ 0.0997 rad.
        let start = dispatch_env(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(100.0, 10.0)),
        )
        .expect("RotateStart envelope");
        assert!(matches!(start.event, SemanticEvent::RotateStart { .. }));
        let expected = RotateRecognizer::angle(point(0.0, 0.0), point(100.0, 10.0));
        let rotation = start.gesture.rotation.expect("rotation");
        let delta = start.gesture.delta_rotation.expect("deltaRotation");
        assert!(
            (rotation - expected).abs() < 1e-4,
            "rotation must be the threshold-crossing value, got {rotation}"
        );
        assert_eq!(delta, rotation, "deltaRotation = rotation at start");
        // Update after start: delta is the wrapped per-frame difference.
        let update = dispatch_env(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(100.0, 20.0)),
        )
        .expect("RotateUpdate envelope");
        if let SemanticEvent::RotateUpdate { .. } = update.event {
            let prev = RotateRecognizer::angle(point(0.0, 0.0), point(100.0, 10.0));
            let cur = RotateRecognizer::angle(point(0.0, 0.0), point(100.0, 20.0));
            let delta = update.gesture.delta_rotation.expect("deltaRotation");
            assert!(
                (delta - (cur - prev)).abs() < 1e-4,
                "expected {} got {delta}",
                cur - prev
            );
        } else {
            panic!("expected RotateUpdate");
        }
    }

    /// deltaRotation wraps across the ±π seam: two updates on opposite
    /// sides of the boundary differ by a tiny angle, so the per-frame
    /// delta must be tiny — not ~2π.
    #[test]
    fn rotate_update_delta_wraps_across_pi_seam() {
        let node = make_key();
        let mut r = RotateRecognizer::new(1, node);
        // Initial angle 0 (a at origin, b at +x).
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Down, point(0.0, 0.0)),
        );
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Down, point(100.0, 0.0)),
        );
        // b to the -x side slightly above: angle ≈ π − ε (claims, rotation
        // ≈ +π − ε).
        let start = dispatch_env(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(-100.0, 1.0)),
        );
        assert!(matches!(
            start.map(|e| e.event),
            Some(SemanticEvent::RotateStart { .. })
        ));
        // b to the -x side slightly below: angle ≈ −π + ε (rotation ≈ −π + ε).
        let update = dispatch_env(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(-100.0, -1.0)),
        )
        .expect("RotateUpdate envelope");
        let delta = update.gesture.delta_rotation.expect("deltaRotation");
        assert!(
            delta.abs() < 0.1,
            "seam crossing must stay small, got {delta} rad"
        );
        // Unwrapped it would be ≈ −2π (+small), so the value is provably
        // wrapped.
        assert!(delta.abs() < PI, "delta must be within ±π, got {delta}");
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

    #[test]
    fn rotate_two_one_two_regrab_is_a_fresh_session() {
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
        // 26.6° twist crosses the π/36 activation gate.
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(100.0, 50.0)),
        );
        assert_eq!(r.state(), RecognizerState::Claimed);
        let end = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Up, point(100.0, 50.0)),
        );
        assert!(matches!(end, Some(SemanticEvent::RotateEnd { .. })));
        // Single-finger moves stay quiet.
        assert!(dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Move, point(30.0, 0.0))
        )
        .is_none());
        // Regaining quorum resets THIS instance's session bookkeeping
        // immediately: a later threshold crossing must surface as a
        // FRESH Possible→Claimed edge with a freshly sampled angle
        // baseline.
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Down, point(0.0, -100.0)),
        );
        assert_eq!(
            r.state(),
            RecognizerState::Possible,
            "regained pair must restart from Possible"
        );
        // Rotate slot0 far around slot1: ~45° past the new baseline
        // crosses the gate and emits a fresh Start.
        let start = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Move, point(70.71, -29.29)),
        );
        match start {
            Some(SemanticEvent::RotateStart { .. }) => {}
            other => panic!("expected a fresh RotateStart, got {other:?}"),
        }
        let upd = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Move, point(100.0, 0.0)),
        );
        match upd {
            Some(SemanticEvent::RotateUpdate { radians, .. }) => {
                assert!(
                    radians.abs() <= std::f32::consts::FRAC_PI_2 + 1e-3,
                    "rotation reported relative to the FRESH baseline, got {radians}"
                );
            }
            other => panic!("expected RotateUpdate on fresh session, got {other:?}"),
        }
    }

    #[test]
    fn rotate_up_of_an_untracked_pointer_is_a_pure_noop() {
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
            PointerEvent::simple(1, PointerPhase::Move, point(100.0, 60.0)),
        );
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(9, PointerPhase::Up, point(40.0, 0.0)),
        );
        assert!(ev.is_none(), "untracked Up emitted {ev:?}");
        assert_eq!(r.state(), RecognizerState::Claimed);
        let upd = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(100.0, 80.0)),
        );
        assert!(matches!(upd, Some(SemanticEvent::RotateUpdate { .. })));
    }
}
