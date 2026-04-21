//! PanRecognizer — claim after pointer moves > dragThreshold.

use crate::document::NodeKey;
use crate::geometry::{point, Point};
use crate::gesture::pointer::{PointerEvent, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::SemanticEvent;
use std::time::Instant;

pub struct PanRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    start: Option<(Point, Instant)>,
    last: Option<(Point, Instant)>,
    threshold: f32,
    claimed: bool,
}

impl PanRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            start: None,
            last: None,
            threshold: 8.0,
            claimed: false,
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
                self.start = Some((event.position, event.timestamp));
                self.last = self.start;
                self.state = RecognizerState::Possible;
                self.claimed = false;
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
                        arena.emit(SemanticEvent::PanStart {
                            node: self.node,
                            position: event.position,
                        });
                        self.state = RecognizerState::Claimed;
                        self.claimed = true;
                    }
                } else if let Some((last_pos, last_t)) = self.last {
                    let delta = point(event.position.x - last_pos.x, event.position.y - last_pos.y);
                    let dt = event.timestamp.duration_since(last_t).as_secs_f32();
                    let velocity = if dt > 0.0 {
                        point(delta.x / dt, delta.y / dt)
                    } else {
                        point(0.0, 0.0)
                    };
                    arena.emit(SemanticEvent::PanUpdate {
                        node: self.node,
                        delta,
                        velocity,
                    });
                }
                self.last = Some((event.position, event.timestamp));
            }
            PointerPhase::Up => {
                if self.claimed {
                    let velocity = match (self.start, self.last) {
                        (Some((_, t0)), Some((p_last, t1))) => {
                            let dt = t1.duration_since(t0).as_secs_f32().max(1e-3);
                            point(
                                (p_last.x - event.position.x) / dt,
                                (p_last.y - event.position.y) / dt,
                            )
                        }
                        _ => point(0.0, 0.0),
                    };
                    arena.emit(SemanticEvent::PanEnd {
                        node: self.node,
                        velocity,
                    });
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
    use slotmap::SlotMap;

    fn make_key() -> NodeKey {
        let mut sm: SlotMap<NodeKey, u32> = SlotMap::with_key();
        sm.insert(0)
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
        assert!(matches!(pending, Some(SemanticEvent::PanStart { .. })));
    }
}
