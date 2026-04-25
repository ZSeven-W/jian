//! ScaleRecognizer — Phase 1 placeholder (single-pointer).
//!
//! True pinch-to-scale needs two simultaneous PointerEvents and the
//! gesture arena's multi-pointer dispatch (Plan 8 host driver). This
//! recognizer ships now as a no-op placeholder so the registration
//! surface is in place — `priority.rs` already reserves "Scale", and
//! the action surface already derives Scale-related actions when an
//! authored `.op` declares `events.onScaleStart` / `onScaleEnd`. The
//! recognizer claims arena slots only when wired through a future
//! multi-pointer router; today it stays in `Possible` and never
//! emits, which keeps the rest of the gesture pipeline intact.
//!
//! When multi-pointer support lands, replace the body of
//! `handle_pointer` with focal-point + scale-factor tracking; spec
//! tests already cover the arena interaction in
//! `arena_recognizer_priority_test.rs`.

use crate::document::NodeKey;
use crate::gesture::pointer::PointerEvent;
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};

pub struct ScaleRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
}

impl ScaleRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
        }
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
        _event: &PointerEvent,
        _arena: &mut ArenaHandle<'_>,
    ) -> RecognizerState {
        // Multi-pointer required; single-pointer dispatch never
        // claims for Scale. Stay Possible so the arena doesn't
        // mark the recognizer as Rejected and keep the slot live
        // for a future multi-pointer wire-up.
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
    fn scale_starts_possible_and_stays() {
        let node = make_key();
        let r = ScaleRecognizer::new(1, node);
        assert_eq!(r.state(), RecognizerState::Possible);
        assert_eq!(r.kind(), "Scale");
    }
}
