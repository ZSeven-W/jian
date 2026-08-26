//! LongPressRecognizer — claim if still pressed & still at start position
//! after `duration_ms` elapses. Driven by `tick(now)` from the host.
//!
//! Also serves the touch ContextMenu fallback: when no node on the hit
//! chain declares an enabled `onLongPress` but one declares an enabled
//! `onContextMenu`, the same long-press deadline emits `ContextMenu`
//! instead. A single press never runs both.

use crate::document::NodeKey;
use crate::gesture::pointer::{PointerEvent, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::{GestureFacts, PointerFacts, SemanticEvent, SemanticEventEnvelope};

/// What a deadline that elapses still-pressed emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongPressMode {
    LongPress,
    /// Touch long-press with no explicit `onLongPress` on the chain but
    /// with an enabled `onContextMenu` — the fallback emits ContextMenu.
    ContextMenu,
}

pub struct LongPressRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    down_time_ms: Option<u64>,
    down_position: Option<crate::geometry::Point>,
    duration_ms: u64,
    slop_px: f32,
    mode: LongPressMode,
    /// Facts captured at the pointer Down; the deadline fires with no new
    /// pointer event, so these are the factual metadata of the press.
    down_facts: Option<PointerFacts>,
    /// Claim-time event, emitted from `accept` AFTER losers were rejected
    /// (so a Press cancellation precedes the winner's event).
    pending_claim: Option<SemanticEventEnvelope>,
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
            mode: LongPressMode::LongPress,
            down_facts: None,
            pending_claim: None,
        }
    }

    /// Build the recognizer in ContextMenu-fallback mode (touch long-press
    /// with no explicit onLongPress on the chain).
    pub fn for_context_menu(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            mode: LongPressMode::ContextMenu,
            ..Self::new(id, node)
        }
    }

    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
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
        match self.mode {
            LongPressMode::LongPress => "LongPress",
            LongPressMode::ContextMenu => "ContextMenu",
        }
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
                self.down_facts = Some(PointerFacts::from_event(event));
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

    fn tick(&mut self, now_ms: u64, _arena: &mut ArenaHandle<'_>) {
        if !matches!(self.state, RecognizerState::Defer) {
            return;
        }
        if let (Some(t0), Some(p0)) = (self.down_time_ms, self.down_position) {
            if now_ms.saturating_sub(t0) >= self.duration_ms {
                let event = match self.mode {
                    LongPressMode::LongPress => SemanticEvent::LongPress {
                        node: self.node,
                        position: p0,
                        duration_ms: self.duration_ms as u32,
                    },
                    LongPressMode::ContextMenu => SemanticEvent::ContextMenu {
                        node: self.node,
                        position: p0,
                    },
                };
                // `down_facts` is seeded on Down (the only state that can
                // reach a deadline); the deadline fires with no new pointer
                // event, so the press-down facts are the factual metadata.
                let Some(facts) = self.down_facts.clone() else {
                    return;
                };
                // Claim-time event is emitted from `accept` so the arena
                // can order it after loser cancellations.
                self.pending_claim = Some(SemanticEventEnvelope {
                    event,
                    pointer_facts: Some(facts),
                    gesture: GestureFacts {
                        duration_ms: Some(self.duration_ms as u32),
                        ..Default::default()
                    },
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
