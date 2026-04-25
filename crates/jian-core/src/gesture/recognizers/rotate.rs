//! RotateRecognizer — Phase 1 placeholder.
//!
//! Like `ScaleRecognizer`, true two-finger rotate needs the arena's
//! multi-pointer dispatch. This skeleton holds the registration slot
//! so the priority table + action-surface derivation already in place
//! don't dangle. Replace the body when the multi-pointer host driver
//! lands (Plan 8 follow-on).

use crate::document::NodeKey;
use crate::gesture::pointer::PointerEvent;
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};

pub struct RotateRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
}

impl RotateRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
        }
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
        _event: &PointerEvent,
        _arena: &mut ArenaHandle<'_>,
    ) -> RecognizerState {
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
    fn rotate_starts_possible_and_stays() {
        let node = make_key();
        let r = RotateRecognizer::new(1, node);
        assert_eq!(r.state(), RecognizerState::Possible);
        assert_eq!(r.kind(), "Rotate");
    }
}
