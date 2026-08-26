//! TapRecognizer + DoubleTapRecognizer.

use crate::document::NodeKey;
use crate::gesture::pointer::{MouseButtons, PointerEvent, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::{PointerFacts, SemanticEvent, SemanticEventEnvelope};

pub struct TapRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    down_position: Option<crate::geometry::Point>,
    down_time_ms: Option<u64>,
    /// The initiating Down's provable single button (absent when the Down
    /// was button-less or ambiguous) — retained on the Tap's facts while
    /// phase/position/timestamp/buttons stay from the triggering Up.
    down_button: Option<MouseButtons>,
    slop_px: f32,
    timeout_ms: u64,
    /// Claim-time Tap, emitted from `accept` AFTER losers were rejected
    /// (so a Press cancellation precedes the Tap).
    pending_claim: Option<SemanticEventEnvelope>,
}

impl TapRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            down_position: None,
            down_time_ms: None,
            down_button: None,
            slop_px: 8.0,
            timeout_ms: 500,
            pending_claim: None,
        }
    }
}

impl Recognizer for TapRecognizer {
    fn id(&self) -> RecognizerId {
        self.id
    }
    fn kind(&self) -> &'static str {
        "Tap"
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
        _arena: &mut ArenaHandle<'_>,
    ) -> RecognizerState {
        match event.phase {
            PointerPhase::Down => {
                self.down_position = Some(event.position);
                self.down_time_ms = Some(event.t_ms);
                self.down_button = PointerFacts::from_event(event).button;
                self.state = RecognizerState::Possible;
            }
            PointerPhase::Move => {
                if let Some(dp) = self.down_position {
                    let dx = event.position.x - dp.x;
                    let dy = event.position.y - dp.y;
                    if (dx * dx + dy * dy).sqrt() > self.slop_px {
                        self.state = RecognizerState::Rejected;
                    }
                }
            }
            PointerPhase::Up => {
                if matches!(self.state, RecognizerState::Rejected) {
                    return self.state;
                }
                if let (Some(dt), Some(_dp)) = (self.down_time_ms, self.down_position) {
                    if event.t_ms.saturating_sub(dt) <= self.timeout_ms {
                        // Claim-time event is emitted from `accept` so the
                        // arena can order it after loser cancellations.
                        self.pending_claim = Some(SemanticEventEnvelope {
                            event: SemanticEvent::Tap {
                                node: self.node,
                                position: event.position,
                            },
                            pointer_facts: Some(
                                PointerFacts::from_event(event)
                                    .with_initiating_button(self.down_button),
                            ),
                            gesture: Default::default(),
                        });
                        self.state = RecognizerState::Claimed;
                    } else {
                        self.state = RecognizerState::Rejected;
                    }
                }
            }
            PointerPhase::Cancel => {
                self.state = RecognizerState::Rejected;
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
    }
}

pub struct DoubleTapRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    first_up: Option<(u64, crate::geometry::Point)>,
    down_time_ms: Option<u64>,
    down_position: Option<crate::geometry::Point>,
    /// Provable single button of the CURRENT tap's initiating Down —
    /// retained on the DoubleTap facts while phase/position/timestamp/
    /// buttons stay from the triggering Up.
    down_button: Option<MouseButtons>,
    slop_px: f32,
    gap_ms: u64,
}

impl DoubleTapRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            first_up: None,
            down_time_ms: None,
            down_position: None,
            down_button: None,
            slop_px: 16.0,
            gap_ms: 300,
        }
    }
}

impl Recognizer for DoubleTapRecognizer {
    fn id(&self) -> RecognizerId {
        self.id
    }
    fn kind(&self) -> &'static str {
        "DoubleTap"
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
                self.down_time_ms = Some(event.t_ms);
                self.down_position = Some(event.position);
                self.down_button = PointerFacts::from_event(event).button;
                if let Some((t, p)) = self.first_up {
                    let dt = event.t_ms.saturating_sub(t);
                    let dx = event.position.x - p.x;
                    let dy = event.position.y - p.y;
                    if dt > self.gap_ms || (dx * dx + dy * dy).sqrt() > self.slop_px {
                        // Too far in time or space — reset to single-tap tracking.
                        self.first_up = None;
                    }
                }
            }
            PointerPhase::Move => {}
            PointerPhase::Up => {
                if let Some((_, _)) = self.first_up {
                    // Second up → double tap.
                    arena.emit_with_facts(
                        SemanticEvent::DoubleTap {
                            node: self.node,
                            position: event.position,
                        },
                        PointerFacts::from_event(event).with_initiating_button(self.down_button),
                    );
                    self.state = RecognizerState::Claimed;
                    self.first_up = None;
                } else {
                    self.first_up = Some((event.t_ms, event.position));
                }
            }
            PointerPhase::Cancel => {
                self.state = RecognizerState::Rejected;
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
    use crate::gesture::recognizer::Recognizer;
    use slotmap::SlotMap;

    fn make_key() -> NodeKey {
        let mut sm: SlotMap<NodeKey, u32> = SlotMap::with_key();
        sm.insert(0)
    }

    fn event(id: u32, phase: PointerPhase, x: f32, y: f32) -> PointerEvent {
        PointerEvent::simple(id, phase, point(x, y))
    }

    #[test]
    fn tap_claims_on_fast_up_at_same_spot() {
        let node = make_key();
        let mut r = TapRecognizer::new(1, node);
        let mut pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        let _ = r.handle_pointer(&event(0, PointerPhase::Down, 10.0, 10.0), &mut h);
        assert_eq!(r.state(), RecognizerState::Possible);
        let _ = r.handle_pointer(&event(0, PointerPhase::Up, 10.5, 10.5), &mut h);
        assert_eq!(r.state(), RecognizerState::Claimed);
        // The claim event is emitted from `accept`, not from the Up.
        assert!(pending.is_none());
        let mut h2 = ArenaHandle {
            pending_semantic: &mut pending,
        };
        r.accept(&mut h2);
        assert!(matches!(
            pending.map(|e| e.event),
            Some(SemanticEvent::Tap { .. })
        ));
    }

    #[test]
    fn tap_rejects_on_move_past_slop() {
        let node = make_key();
        let mut r = TapRecognizer::new(1, node);
        let mut pending = None;
        let mut h = ArenaHandle {
            pending_semantic: &mut pending,
        };
        let _ = r.handle_pointer(&event(0, PointerPhase::Down, 0.0, 0.0), &mut h);
        let _ = r.handle_pointer(&event(0, PointerPhase::Move, 20.0, 0.0), &mut h);
        assert_eq!(r.state(), RecognizerState::Rejected);
    }
}
