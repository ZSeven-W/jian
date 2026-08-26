//! ScaleRecognizer — two-pointer pinch geometry.
//!
//! Phase 1 model (Plan 5 §B.2 / multi-pointer plan Task 2): tracks
//! the first two pointers in arrival order. Computes scale =
//! `current_distance / initial_distance`, focal = midpoint. Crosses
//! threshold at `|ratio - 1| > 0.05` (5%) — the same constant Plan
//! 5 reserves for Scale's activation gate.
//!
//! Lifecycle table (matches the multi-pointer plan §Recognizer state):
//!
//! | `pids` before | event           | after | emit                 |
//! | ---           | ---             | ---   | ---                  |
//! |   0           | pid_a Down      |   1   | (single finger — quiet) |
//! |   1           | pid_b Down      |   2   | (quiet until threshold) |
//! |   2           | Move past 5%    |   2   | `ScaleStart` once    |
//! |   2           | Move (claimed)  |   2   | `ScaleUpdate`        |
//! |   2           | pid Up          |   1   | `ScaleEnd`           |
//! |   1           | pid Up          |   0   | (already ended)      |
//!
//! The recognizer is owned by `PointerRouter::multi`, NOT by a
//! per-pointer arena. Per-pointer arenas hand it events through
//! `dispatch_multi`; when it Claims, the router cancels the
//! per-pointer arenas it participated in.

use crate::document::NodeKey;
use crate::geometry::{point, Point};
use crate::gesture::pointer::{PointerEvent, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::{GestureFacts, PointerFacts, SemanticEvent};

/// Activation threshold. `|scale - 1| > 0.05` to claim. Plan 5 Task 9.
const SCALE_ACTIVATION: f32 = 0.05;

pub struct ScaleRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    /// First two pointers in arrival order. Phase 1 ignores 3+
    /// fingers; subsequent pointers don't engage this recognizer.
    pids: Vec<(u32, Point)>,
    /// Distance + focal sampled when the second pointer Down arrived.
    /// The scale ratio is computed against this baseline.
    initial: Option<(f32, Point)>,
    /// Whether the recognizer already emitted `ScaleStart` (and thus
    /// should emit `ScaleUpdate` on subsequent moves).
    started: bool,
    /// Whether the recognizer already emitted `ScaleEnd`. Set on the
    /// Up that drops `pids` from 2 → 1 so a stray third Up doesn't
    /// re-emit.
    ended: bool,
    /// Last reported scale ratio (for per-frame `deltaScale`).
    last_scale: Option<f32>,
    /// Last reported focal point (for end payloads).
    last_focal: Option<Point>,
}

impl ScaleRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            pids: Vec::with_capacity(2),
            initial: None,
            started: false,
            ended: false,
            last_scale: None,
            last_focal: None,
        }
    }

    fn distance(a: Point, b: Point) -> f32 {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        (dx * dx + dy * dy).sqrt()
    }

    fn midpoint(a: Point, b: Point) -> Point {
        point((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
    }
}

impl Recognizer for ScaleRecognizer {
    fn id(&self) -> RecognizerId {
        self.id
    }
    fn kind(&self) -> &'static str {
        "Scale"
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
                // Phase 1: ignore 3+ fingers. Existing pid re-Down is
                // a no-op (shouldn't happen in practice).
                if self.pids.iter().any(|(p, _)| *p == pid) || self.pids.len() >= 2 {
                    return self.state;
                }
                self.pids.push((pid, event.position));
                if self.pids.len() == 2 {
                    // Regaining the two-finger quorum opens a FRESH
                    // session (R2B2 2→1→2 contract): new baseline,
                    // and Start/End symmetry requires a new Possible→
                    // Claimed edge so the router re-runs preflight and
                    // a new `ScaleStart` can fire.
                    let a = self.pids[0].1;
                    let b = self.pids[1].1;
                    self.initial = Some((Self::distance(a, b), Self::midpoint(a, b)));
                    self.state = RecognizerState::Possible;
                    self.started = false;
                    self.ended = false;
                    self.last_scale = None;
                    self.last_focal = None;
                }
            }
            PointerPhase::Move => {
                // Update the moving pointer's last position.
                if let Some(slot) = self.pids.iter_mut().find(|(p, _)| *p == pid) {
                    slot.1 = event.position;
                } else {
                    return self.state;
                }
                if self.pids.len() < 2 {
                    return self.state;
                }
                let a = self.pids[0].1;
                let b = self.pids[1].1;
                let cur = Self::distance(a, b);
                let focal = Self::midpoint(a, b);
                let Some((init_dist, _)) = self.initial else {
                    return self.state;
                };
                if init_dist <= f32::EPSILON {
                    return self.state;
                }
                let scale = cur / init_dist;
                if !self.started {
                    if (scale - 1.0).abs() > SCALE_ACTIVATION {
                        self.started = true;
                        self.state = RecognizerState::Claimed;
                        self.last_scale = Some(scale);
                        self.last_focal = Some(focal);
                        let facts = PointerFacts::from_event(event);
                        arena.emit_with(
                            SemanticEvent::ScaleStart {
                                node: self.node,
                                focal,
                            },
                            facts,
                            GestureFacts {
                                // The threshold-crossing scale itself, with
                                // the per-frame delta vs. the gesture's
                                // initial baseline (scale − 1).
                                scale: Some(scale),
                                delta_scale: Some(scale - 1.0),
                                focal: Some(focal),
                                ..Default::default()
                            },
                        );
                    }
                } else {
                    let delta_scale = self.last_scale.map(|prev| scale - prev);
                    self.last_scale = Some(scale);
                    self.last_focal = Some(focal);
                    let facts = PointerFacts::from_event(event);
                    arena.emit_with(
                        SemanticEvent::ScaleUpdate {
                            node: self.node,
                            scale,
                            focal,
                        },
                        facts,
                        GestureFacts {
                            scale: Some(scale),
                            delta_scale,
                            focal: Some(focal),
                            ..Default::default()
                        },
                    );
                }
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                // Only a TRACKED pointer participates in this teardown.
                // An untracked pointer (never registered because the
                // transform owns its two-finger quorum) must be a pure
                // no-op — a third finger's Up may never end someone
                // else's gesture. R2B2 third-finger contract.
                if !self.pids.iter().any(|(p, _)| *p == pid) {
                    return self.state;
                }
                self.pids.retain(|(p, _)| *p != pid);
                if self.started && !self.ended && self.pids.len() < 2 {
                    // Dropping below the two-finger quorum terminates
                    // THIS session symmetrically (Start counted one End)
                    // and arms the next Down pair for a fresh session
                    // with a freshly sampled baseline. R2B2 2→1→2 fix:
                    // previously the instance kept `started` true and a
                    // re-grab emitted Updates with stale deltas and no
                    // Start.
                    self.ended = true;
                    let facts = PointerFacts::from_event(event);
                    arena.emit_with(
                        SemanticEvent::ScaleEnd { node: self.node },
                        facts,
                        GestureFacts {
                            scale: self.last_scale,
                            focal: self.last_focal,
                            ..Default::default()
                        },
                    );
                }
                if self.pids.is_empty() {
                    // Reset for hypothetical re-attach; the router
                    // typically drops the recognizer instance once
                    // `shared[id]` empties, so this branch is mostly
                    // defensive.
                    self.initial = None;
                    self.started = false;
                    self.ended = false;
                    self.last_scale = None;
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
    use crate::gesture::semantic::SemanticEventEnvelope;
    use slotmap::SlotMap;

    fn make_key() -> NodeKey {
        let mut sm: SlotMap<NodeKey, u32> = SlotMap::with_key();
        sm.insert(0)
    }

    fn dispatch(r: &mut ScaleRecognizer, ev: PointerEvent) -> Option<SemanticEvent> {
        let mut pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        let _ = r.handle_pointer(&ev, &mut h);
        pending.map(|e| e.event)
    }

    /// Like `dispatch` but keeps the envelope so gesture facts (exact
    /// scale / deltaScale / focal) can be asserted.
    fn dispatch_env(r: &mut ScaleRecognizer, ev: PointerEvent) -> Option<SemanticEventEnvelope> {
        let mut pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        let _ = r.handle_pointer(&ev, &mut h);
        pending
    }

    #[test]
    fn scale_starts_possible_and_stays() {
        let node = make_key();
        let r = ScaleRecognizer::new(1, node);
        assert_eq!(r.state(), RecognizerState::Possible);
        assert_eq!(r.kind(), "Scale");
    }

    #[test]
    fn scale_claims_at_5_percent_with_focal_at_midpoint() {
        let node = make_key();
        let mut r = ScaleRecognizer::new(1, node);
        // Two-finger Down at (0,0) and (100,0) → distance 100, focal (50,0).
        assert!(dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Down, point(0.0, 0.0))
        )
        .is_none());
        assert!(dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Down, point(100.0, 0.0))
        )
        .is_none());
        // 4% expansion — under threshold.
        assert!(dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(104.0, 0.0))
        )
        .is_none());
        assert_eq!(r.state(), RecognizerState::Possible);
        // 6% expansion — Claim.
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(106.0, 0.0)),
        );
        assert_eq!(r.state(), RecognizerState::Claimed);
        match ev {
            Some(SemanticEvent::ScaleStart { focal, .. }) => {
                assert_eq!(focal.x, 53.0);
                assert_eq!(focal.y, 0.0);
            }
            other => panic!("expected ScaleStart, got {other:?}"),
        }
    }

    #[test]
    fn scale_start_carries_actual_crossing_scale() {
        let node = make_key();
        let mut r = ScaleRecognizer::new(1, node);
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Down, point(0.0, 0.0)),
        );
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Down, point(100.0, 0.0)),
        );
        // 6% expansion: dist 100 → 106. The Start carries the actual
        // threshold-crossing scale (1.06) and deltaScale = scale − 1.
        let start = dispatch_env(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(106.0, 0.0)),
        )
        .expect("ScaleStart envelope");
        assert!(matches!(start.event, SemanticEvent::ScaleStart { .. }));
        assert_eq!(start.gesture.scale, Some(1.06));
        assert!(
            (start.gesture.delta_scale.unwrap() - 0.06).abs() < 1e-5,
            "deltaScale = scale − 1, got {:?}",
            start.gesture.delta_scale
        );
        assert_eq!(start.gesture.focal, Some(point(53.0, 0.0)));

        // Update: per-frame delta = scale − last scale (1.2 → 0.14).
        let update = dispatch_env(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(120.0, 0.0)),
        )
        .expect("ScaleUpdate envelope");
        assert!(matches!(update.event, SemanticEvent::ScaleUpdate { .. }));
        assert_eq!(update.gesture.scale, Some(1.2));
        assert!((update.gesture.delta_scale.unwrap() - 0.14).abs() < 1e-5);
    }

    #[test]
    fn scale_emits_update_after_start() {
        let node = make_key();
        let mut r = ScaleRecognizer::new(1, node);
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Down, point(0.0, 0.0)),
        );
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Down, point(100.0, 0.0)),
        );
        // Cross threshold.
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(120.0, 0.0)),
        );
        // Subsequent moves emit Update with current scale.
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(150.0, 0.0)),
        );
        match ev {
            Some(SemanticEvent::ScaleUpdate { scale, .. }) => {
                assert!((scale - 1.5).abs() < f32::EPSILON);
            }
            other => panic!("expected ScaleUpdate, got {other:?}"),
        }
    }

    #[test]
    fn scale_end_on_first_up_after_start() {
        let node = make_key();
        let mut r = ScaleRecognizer::new(1, node);
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
            PointerEvent::simple(1, PointerPhase::Move, point(120.0, 0.0)),
        );
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Up, point(120.0, 0.0)),
        );
        assert!(matches!(ev, Some(SemanticEvent::ScaleEnd { .. })));
        // A second Up (the other finger) doesn't re-emit End.
        let ev2 = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Up, point(0.0, 0.0)),
        );
        assert!(ev2.is_none());
    }

    #[test]
    fn scale_quiet_with_single_pointer() {
        // Only one finger Down — should never emit anything.
        let node = make_key();
        let mut r = ScaleRecognizer::new(1, node);
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Down, point(0.0, 0.0)),
        );
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Move, point(50.0, 0.0)),
        );
        assert!(ev.is_none());
        assert_eq!(r.state(), RecognizerState::Possible);
    }

    #[test]
    fn scale_two_one_two_regrab_is_a_fresh_session() {
        let node = make_key();
        let mut r = ScaleRecognizer::new(1, node);
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
            PointerEvent::simple(1, PointerPhase::Move, point(120.0, 0.0)),
        );
        assert_eq!(r.state(), RecognizerState::Claimed);
        // Lift to one finger: symmetric End.
        let end = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Up, point(120.0, 0.0)),
        );
        assert!(matches!(end, Some(SemanticEvent::ScaleEnd { .. })));
        // Single-finger moves are quiet — no stale-delta updates.
        assert!(dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Move, point(30.0, 0.0))
        )
        .is_none());
        // Regaining quorum resets THIS instance's session bookkeeping
        // immediately: the router may only treat a later threshold
        // crossing as a FRESH claim through Possible.
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Down, point(300.0, 0.0)),
        );
        assert_eq!(
            r.state(),
            RecognizerState::Possible,
            "regained pair must restart from Possible"
        );
        // Crossing 5% of the FRESH baseline (slot0=30 → slot1=300 ⇒
        // 270px) claims anew.
        let start = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Move, point(-5.0, 0.0)),
        );
        match start {
            // dist 305 / baseline 270 ≈ 1.13 — fresh-crossing scale.
            Some(SemanticEvent::ScaleStart { .. }) => {}
            other => panic!("expected a fresh ScaleStart, got {other:?}"),
        }
        let _ = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(320.0, 0.0)),
        );
        // A subsequent Update reports scale against the NEW baseline
        // (dist 315 / 270 ≈ 1.167), never the stale pre-lift one.
        let upd = dispatch(
            &mut r,
            PointerEvent::simple(0, PointerPhase::Move, point(5.0, 0.0)),
        );
        match upd {
            Some(SemanticEvent::ScaleUpdate { scale, .. }) => {
                assert!(
                    (scale - 315.0 / 270.0).abs() < 1e-4,
                    "stale baseline leaked: {scale}"
                );
            }
            other => panic!("expected ScaleUpdate on fresh session, got {other:?}"),
        }
    }

    #[test]
    fn scale_up_of_an_untracked_pointer_is_a_pure_noop() {
        let node = make_key();
        let mut r = ScaleRecognizer::new(1, node);
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
            PointerEvent::simple(1, PointerPhase::Move, point(130.0, 0.0)),
        );
        // A pointer the recognizer never tracked must not disturb it.
        let ev = dispatch(
            &mut r,
            PointerEvent::simple(9, PointerPhase::Up, point(50.0, 0.0)),
        );
        assert!(ev.is_none(), "untracked Up emitted {ev:?}");
        assert_eq!(r.state(), RecognizerState::Claimed);
        // The live pair keeps streaming updates afterwards.
        let upd = dispatch(
            &mut r,
            PointerEvent::simple(1, PointerPhase::Move, point(150.0, 0.0)),
        );
        assert!(matches!(upd, Some(SemanticEvent::ScaleUpdate { .. })));
    }
}
