//! TapRecognizer + DoubleTapRecognizer.

use crate::document::NodeKey;
use crate::gesture::pointer::{PointerEvent, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::SemanticEvent;
use std::time::{Duration, Instant};

pub struct TapRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    down_position: Option<crate::geometry::Point>,
    down_time: Option<Instant>,
    slop_px: f32,
    timeout: Duration,
}

impl TapRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            down_position: None,
            down_time: None,
            slop_px: 8.0,
            timeout: Duration::from_millis(500),
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
        arena: &mut ArenaHandle<'_>,
    ) -> RecognizerState {
        match event.phase {
            PointerPhase::Down => {
                self.down_position = Some(event.position);
                self.down_time = Some(event.timestamp);
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
                if let (Some(dt), Some(_dp)) = (self.down_time, self.down_position) {
                    if event.timestamp.duration_since(dt) <= self.timeout {
                        arena.emit(SemanticEvent::Tap {
                            node: self.node,
                            position: event.position,
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

    fn accept(&mut self, _: &mut ArenaHandle<'_>) {
        self.state = RecognizerState::Claimed;
    }
    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
    }
}

pub struct DoubleTapRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    first_up: Option<(Instant, crate::geometry::Point)>,
    down_time: Option<Instant>,
    down_position: Option<crate::geometry::Point>,
    slop_px: f32,
    gap: Duration,
}

impl DoubleTapRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            first_up: None,
            down_time: None,
            down_position: None,
            slop_px: 16.0,
            gap: Duration::from_millis(300),
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
                self.down_time = Some(event.timestamp);
                self.down_position = Some(event.position);
                if let Some((t, p)) = self.first_up {
                    let dt = event.timestamp.duration_since(t);
                    let dx = event.position.x - p.x;
                    let dy = event.position.y - p.y;
                    if dt > self.gap || (dx * dx + dy * dy).sqrt() > self.slop_px {
                        // Too far in time or space — reset to single-tap tracking.
                        self.first_up = None;
                    }
                }
            }
            PointerPhase::Move => {}
            PointerPhase::Up => {
                if let Some((_, _)) = self.first_up {
                    // Second up → double tap.
                    arena.emit(SemanticEvent::DoubleTap {
                        node: self.node,
                        position: event.position,
                    });
                    self.state = RecognizerState::Claimed;
                    self.first_up = None;
                } else {
                    self.first_up = Some((event.timestamp, event.position));
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
    use crate::gesture::recognizer::Recognizer;
    use crate::geometry::point;
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
        assert!(matches!(pending, Some(SemanticEvent::Tap { .. })));
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
