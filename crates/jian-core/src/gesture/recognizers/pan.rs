//! PanRecognizer — claim after pointer moves > dragThreshold.

use crate::document::NodeKey;
use crate::geometry::{point, Point};
use crate::gesture::pointer::{MouseButtons, PointerEvent, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::{GestureFacts, PointerFacts, SemanticEvent, SemanticEventEnvelope};

pub struct PanRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    start: Option<(Point, u64)>,
    last: Option<(Point, u64)>,
    /// The initiating Down's provable single button — retained on every
    /// Pan envelope while phase/position/timestamp/buttons stay from the
    /// triggering event. `None` when the Down was button-less or
    /// ambiguous (keeps the key absent).
    down_button: Option<MouseButtons>,
    /// Last measured velocity (a real `delta/dt` of some segment), kept
    /// so an end event with no measurably-spaced final segment can still
    /// report a retained measured value instead of a fabricated one.
    last_velocity: Option<Point>,
    threshold: f32,
    claimed: bool,
    /// Claim-time PanStart, emitted from `accept` AFTER losers were
    /// rejected (so a Press cancellation precedes the PanStart).
    pending_claim: Option<SemanticEventEnvelope>,
}

impl PanRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            start: None,
            last: None,
            down_button: None,
            last_velocity: None,
            threshold: 8.0,
            claimed: false,
            pending_claim: None,
        }
    }

    pub fn with_threshold(mut self, px: f32) -> Self {
        self.threshold = px;
        self
    }
}

impl Recognizer for PanRecognizer {
    fn id(&self) -> RecognizerId {
        self.id
    }
    fn kind(&self) -> &'static str {
        "Pan"
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
        match event.phase {
            PointerPhase::Down => {
                self.start = Some((event.position, event.t_ms));
                self.last = self.start;
                self.down_button = PointerFacts::from_event(event).button;
                self.last_velocity = None;
                self.state = RecognizerState::Possible;
                self.claimed = false;
                self.pending_claim = None;
            }
            PointerPhase::Move => {
                let (start_pos, _) = match self.start {
                    Some(s) => s,
                    None => return self.state,
                };
                if !self.claimed {
                    let dx = event.position.x - start_pos.x;
                    let dy = event.position.y - start_pos.y;
                    if (dx * dx + dy * dy).sqrt() >= self.threshold {
                        // Claim-time event is emitted from `accept` so the
                        // arena can order it after loser cancellations.
                        // Factual values for the threshold-crossing Move:
                        // `start` = the Down, `current` = this Move,
                        // `delta` = current − previous sample, `velocity`
                        // = delta / dt (absent when dt is not computable).
                        let facts = PointerFacts::from_event(event)
                            .with_initiating_button(self.down_button);
                        let (prev_pos, prev_t) = match self.last {
                            Some(l) => l,
                            None => return self.state,
                        };
                        let delta =
                            point(event.position.x - prev_pos.x, event.position.y - prev_pos.y);
                        let dt = event.t_ms.saturating_sub(prev_t) as f32 / 1000.0;
                        let velocity = (dt > 0.0).then(|| point(delta.x / dt, delta.y / dt));
                        if velocity.is_some() {
                            self.last_velocity = velocity;
                        }
                        let gesture = GestureFacts {
                            pan_start: Some(start_pos),
                            pan_current: Some(event.position),
                            pan_delta: Some(delta),
                            pan_translation: Some(point(
                                event.position.x - start_pos.x,
                                event.position.y - start_pos.y,
                            )),
                            pan_velocity: velocity,
                            ..Default::default()
                        };
                        self.pending_claim = Some(SemanticEventEnvelope {
                            event: SemanticEvent::PanStart {
                                node: self.node,
                                position: event.position,
                            },
                            pointer_facts: Some(facts),
                            gesture,
                        });
                        self.state = RecognizerState::Claimed;
                        self.claimed = true;
                    }
                } else if let Some((last_pos, last_t)) = self.last {
                    let delta = point(event.position.x - last_pos.x, event.position.y - last_pos.y);
                    let dt = event.t_ms.saturating_sub(last_t) as f32 / 1000.0;
                    // Factual per-segment velocity: `delta` over this
                    // segment's time. When dt is not computable, fall back
                    // to the last measured velocity, never a zero guess.
                    let measured = (dt > 0.0).then(|| point(delta.x / dt, delta.y / dt));
                    let velocity = measured.or(self.last_velocity);
                    if measured.is_some() {
                        self.last_velocity = measured;
                    }
                    let facts =
                        PointerFacts::from_event(event).with_initiating_button(self.down_button);
                    let gesture = GestureFacts {
                        pan_start: self.start.map(|(p, _)| p),
                        pan_current: Some(event.position),
                        pan_delta: Some(delta),
                        pan_translation: Some(point(
                            event.position.x - start_pos.x,
                            event.position.y - start_pos.y,
                        )),
                        pan_velocity: velocity,
                        ..Default::default()
                    };
                    // Same legacy-field rule as PanEnd: the facts carry
                    // `None` when nothing was measured; the Point field
                    // keeps the zero vector when no value exists.
                    arena.emit_with(
                        SemanticEvent::PanUpdate {
                            node: self.node,
                            delta,
                            velocity: velocity.unwrap_or_else(|| point(0.0, 0.0)),
                        },
                        facts,
                        gesture,
                    );
                }
                self.last = Some((event.position, event.t_ms));
            }
            PointerPhase::Up => {
                if self.claimed {
                    let start_pos = self.start.map(|(p, _)| p);
                    // Final segment: from the last observed sample to the
                    // Up. Velocity is that segment's delta over its own
                    // time — never the total travel over total duration,
                    // and never reversed. Fall back to the retained last
                    // measured velocity when the final segment time is not
                    // computable.
                    let (last_pos, last_t) = match self.last {
                        Some(l) => l,
                        None => return self.state,
                    };
                    let pan_delta =
                        point(event.position.x - last_pos.x, event.position.y - last_pos.y);
                    let dt = event.t_ms.saturating_sub(last_t) as f32 / 1000.0;
                    let measured = (dt > 0.0).then(|| point(pan_delta.x / dt, pan_delta.y / dt));
                    let velocity = measured.or(self.last_velocity);
                    let facts =
                        PointerFacts::from_event(event).with_initiating_button(self.down_button);
                    let gesture = GestureFacts {
                        pan_start: start_pos,
                        pan_current: Some(event.position),
                        pan_delta: Some(pan_delta),
                        pan_translation: start_pos
                            .map(|p| point(event.position.x - p.x, event.position.y - p.y)),
                        pan_velocity: velocity,
                        ..Default::default()
                    };
                    // `PanEnd.velocity` is a non-optional Point for
                    // source compatibility; when no measurement exists
                    // (dt ≤ 0 AND no retained velocity) the gesture facts
                    // report None (the payload omits velocity — never a
                    // fabricated number), and the legacy field keeps the
                    // zero vector as the only remaining value.
                    arena.emit_with(
                        SemanticEvent::PanEnd {
                            node: self.node,
                            velocity: velocity.unwrap_or_else(|| point(0.0, 0.0)),
                        },
                        facts,
                        gesture,
                    );
                }
            }
            PointerPhase::Cancel => {
                self.state = RecognizerState::Rejected;
                self.pending_claim = None;
            }
            PointerPhase::Hover => {}
        }
        self.state
    }

    fn accept(&mut self, arena: &mut ArenaHandle<'_>) {
        self.state = RecognizerState::Claimed;
        if let Some(claim) = self.pending_claim.take() {
            *arena.pending_semantic = Some(claim);
        }
    }
    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
        self.pending_claim = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gesture::pointer::PointerId;
    use slotmap::SlotMap;

    fn make_key() -> NodeKey {
        let mut sm: SlotMap<NodeKey, u32> = SlotMap::with_key();
        sm.insert(0)
    }

    /// Dispatch and return the pending envelope (claim events are only
    /// emitted from `accept`, so accept afterwards for those).
    fn dispatch_env(r: &mut PanRecognizer, ev: PointerEvent) -> Option<SemanticEventEnvelope> {
        let mut pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        let _ = r.handle_pointer(&ev, &mut h);
        pending
    }

    fn mouse(
        id: u32,
        phase: PointerPhase,
        x: f32,
        y: f32,
        t_ms: u64,
        button: MouseButtons,
    ) -> PointerEvent {
        PointerEvent {
            id: PointerId(id),
            kind: crate::gesture::pointer::PointerKind::Mouse,
            phase,
            position: point(x, y),
            pressure: 0.0,
            buttons: button,
            modifiers: Default::default(),
            tilt: None,
            t_ms,
        }
    }

    #[test]
    fn pan_claims_after_threshold() {
        let node = make_key();
        let mut r = PanRecognizer::new(1, node);
        let mut pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        let _ = r.handle_pointer(
            &PointerEvent::simple(0, PointerPhase::Down, point(0.0, 0.0)),
            &mut h,
        );
        // A 3px move stays Possible.
        pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        let _ = r.handle_pointer(
            &PointerEvent::simple(0, PointerPhase::Move, point(3.0, 0.0)),
            &mut h,
        );
        assert_eq!(r.state(), RecognizerState::Possible);
        // A 10px move crosses threshold.
        pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        let _ = r.handle_pointer(
            &PointerEvent::simple(0, PointerPhase::Move, point(10.0, 0.0)),
            &mut h,
        );
        assert_eq!(r.state(), RecognizerState::Claimed);
        assert!(pending.is_none(), "claim emitted from accept, not here");
        let mut h2 = ArenaHandle {
            pending_semantic: &mut pending,
        };
        r.accept(&mut h2);
        assert!(matches!(
            pending.map(|e| e.event),
            Some(SemanticEvent::PanStart { .. })
        ));
    }

    /// PanStart at the threshold-crossing Move carries factual values:
    /// start = Down, current = Move, delta = current − previous sample,
    /// translation = current − start, velocity = delta / dt. No zero
    /// placeholders for known values.
    #[test]
    fn pan_start_carries_factual_crossing_values() {
        let node = make_key();
        let mut r = PanRecognizer::new(1, node);
        // Down at (0,0) t=0; a sub-threshold Move at (3,0) t=10 becomes
        // the "previous sample"; the threshold-crossing Move is (10,0)
        // t=30.
        assert!(dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Down, 0.0, 0.0, 0, MouseButtons::LEFT),
        )
        .is_none());
        assert!(dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Move, 3.0, 0.0, 10, MouseButtons::LEFT),
        )
        .is_none());
        assert!(dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Move, 10.0, 0.0, 30, MouseButtons::LEFT),
        )
        .is_none());
        let mut pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        r.accept(&mut h);
        let env = pending.expect("claim envelope");
        assert!(matches!(env.event, SemanticEvent::PanStart { .. }));
        let g = &env.gesture;
        assert_eq!(g.pan_start, Some(point(0.0, 0.0)));
        assert_eq!(g.pan_current, Some(point(10.0, 0.0)));
        // Previous sample is the (3,0) Move, not the Down.
        assert_eq!(g.pan_delta, Some(point(7.0, 0.0)));
        assert_eq!(g.pan_translation, Some(point(10.0, 0.0)));
        // 7px over 20ms = 350 px/s.
        assert_eq!(g.pan_velocity, Some(point(350.0, 0.0)));
        // The initiating Down was a single LEFT → retained on the Move
        // triggering facts.
        let f = env.pointer_facts.as_ref().expect("facts");
        assert_eq!(f.phase, PointerPhase::Move);
        assert_eq!(f.button, Some(MouseButtons::LEFT));
        assert_eq!(f.buttons, Some(MouseButtons::LEFT));
    }

    /// PanEnd velocity is the final segment (last sample → Up) over its
    /// own time — NOT total travel over total duration, and NOT reversed.
    #[test]
    fn pan_end_velocity_is_final_segment_not_reversed_total() {
        let node = make_key();
        let mut r = PanRecognizer::new(1, node);
        // 0ms Down; 100ms crossing Move (Δ=(10,0), v=100); 200ms update
        // (Δ=(10,0), v=100); Up at (25,0) t=300.
        let _ = dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Down, 0.0, 0.0, 0, MouseButtons::LEFT),
        );
        let _ = dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Move, 10.0, 0.0, 100, MouseButtons::LEFT),
        );
        let mut pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        r.accept(&mut h);
        assert!(pending.is_some());
        let update = dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Move, 20.0, 0.0, 200, MouseButtons::LEFT),
        )
        .expect("update envelope");
        assert!(matches!(update.event, SemanticEvent::PanUpdate { .. }));
        assert_eq!(update.gesture.pan_velocity, Some(point(100.0, 0.0)));

        let end = dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Up, 25.0, 0.0, 300, MouseButtons::empty()),
        )
        .expect("end envelope");
        match end.event {
            SemanticEvent::PanEnd { velocity, .. } => {
                // Final segment (20,0)→(25,0) over 100ms = 50 px/s.
                assert_eq!(velocity, point(50.0, 0.0), "not reversed, not total");
            }
            other => panic!("expected PanEnd, got {other:?}"),
        }
        assert_eq!(end.gesture.pan_delta, Some(point(5.0, 0.0)));
        assert_eq!(end.gesture.pan_velocity, Some(point(50.0, 0.0)));
        // phase/position/timestamp from the Up; buttons (release) empty;
        // the initiating LEFT is retained.
        let f = end.pointer_facts.as_ref().expect("facts");
        assert_eq!(f.phase, PointerPhase::Up);
        assert_eq!(f.position, point(25.0, 0.0));
        assert_eq!(f.t_ms, 300);
        assert_eq!(f.buttons, None);
        assert_eq!(f.button, Some(MouseButtons::LEFT));
    }

    /// When the final segment time is not computable (Up at the same
    /// timestamp as the last sample), the retained last measured velocity
    /// is used — never a fabricated zero or a total-duration division.
    #[test]
    fn pan_end_falls_back_to_retained_measured_velocity() {
        let node = make_key();
        let mut r = PanRecognizer::new(1, node);
        let _ = dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Down, 0.0, 0.0, 0, MouseButtons::LEFT),
        );
        let _ = dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Move, 10.0, 0.0, 100, MouseButtons::LEFT),
        );
        let mut pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        r.accept(&mut h);
        let _ = dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Move, 20.0, 0.0, 200, MouseButtons::LEFT),
        );
        // Up at the same millisecond as the last sample: dt = 0.
        let end = dispatch_env(
            &mut r,
            mouse(0, PointerPhase::Up, 25.0, 0.0, 200, MouseButtons::empty()),
        )
        .expect("end envelope");
        assert_eq!(
            end.gesture.pan_velocity,
            Some(point(100.0, 0.0)),
            "retained measured velocity, not fabricated"
        );
    }
}
