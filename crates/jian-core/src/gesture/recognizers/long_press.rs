//! LongPressRecognizer — claim if still pressed & still at start position
//! after `duration_ms` elapses. Driven by `tick(now)` from the host.

use crate::document::NodeKey;
use crate::gesture::pointer::{PointerEvent, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::SemanticEvent;

pub struct LongPressRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    down_time_ms: Option<u64>,
    down_position: Option<crate::geometry::Point>,
    duration_ms: u64,
    slop_px: f32,
}

impl LongPressRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            down_time_ms: None,
            down_position: None,
            duration_ms: 500,
            slop_px: 8.0,
        }
    }
    pub fn duration(&self) -> u32 {
        self.duration_ms as u32
    }
}

impl Recognizer for LongPressRecognizer {
    fn id(&self) -> RecognizerId {
        self.id
    }
    fn kind(&self) -> &'static str {
        "LongPress"
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
                self.down_time_ms = Some(event.t_ms);
                self.down_position = Some(event.position);
                self.state = RecognizerState::Defer;
            }
            PointerPhase::Move => {
                if let Some(p0) = self.down_position {
                    let dx = event.position.x - p0.x;
                    let dy = event.position.y - p0.y;
                    if (dx * dx + dy * dy).sqrt() > self.slop_px {
                        self.state = RecognizerState::Rejected;
                    }
                }
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                // Release before timeout → not a long-press.
                if matches!(self.state, RecognizerState::Defer) {
                    self.state = RecognizerState::Rejected;
                }
            }
            PointerPhase::Hover => {}
        }
        self.state
    }

    fn tick(&mut self, now_ms: u64, arena: &mut ArenaHandle<'_>) {
        if !matches!(self.state, RecognizerState::Defer) {
            return;
        }
        if let (Some(t0), Some(p0)) = (self.down_time_ms, self.down_position) {
            if now_ms.saturating_sub(t0) >= self.duration_ms {
                arena.emit(SemanticEvent::LongPress {
                    node: self.node,
                    position: p0,
                    duration_ms: self.duration_ms as u32,
                });
                self.state = RecognizerState::Claimed;
            }
        }
    }

    fn next_wake_ms(&self) -> Option<u64> {
        matches!(self.state, RecognizerState::Defer)
            .then(|| {
                self.down_time_ms
                    .map(|start| start.saturating_add(self.duration_ms))
            })
            .flatten()
    }

    fn accept(&mut self, _: &mut ArenaHandle<'_>) {
        self.state = RecognizerState::Claimed;
    }
    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
    }
}
