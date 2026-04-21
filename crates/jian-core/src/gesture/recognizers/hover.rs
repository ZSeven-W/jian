//! HoverRecognizer — emits HoverEnter/Leave for non-touch pointers.

use crate::document::NodeKey;
use crate::gesture::pointer::{PointerEvent, PointerKind, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::SemanticEvent;

pub struct HoverRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    inside: bool,
}

impl HoverRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            inside: false,
        }
    }
}

impl Recognizer for HoverRecognizer {
    fn id(&self) -> RecognizerId {
        self.id
    }
    fn kind(&self) -> &'static str {
        "Hover"
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
        // Touch/stylus typically don't emit Hover phases.
        if matches!(event.kind, PointerKind::Touch) {
            return self.state;
        }
        match event.phase {
            PointerPhase::Hover => {
                if !self.inside {
                    arena.emit(SemanticEvent::HoverEnter {
                        node: self.node,
                        position: event.position,
                    });
                    self.inside = true;
                }
            }
            PointerPhase::Down | PointerPhase::Up | PointerPhase::Move => {
                // Stay passive — don't claim arena on ordinary input.
            }
            PointerPhase::Cancel => {
                if self.inside {
                    arena.emit(SemanticEvent::HoverLeave {
                        node: self.node,
                        position: event.position,
                    });
                    self.inside = false;
                }
            }
        }
        self.state
    }

    fn accept(&mut self, _: &mut ArenaHandle<'_>) {}
    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
    }
}
