//! Recognizer trait — each implementation runs a state machine on a stream
//! of PointerEvents and decides Claim / Reject via the arena.

use super::pointer::PointerEvent;
use super::semantic::SemanticEvent;
use crate::document::NodeKey;

pub type RecognizerId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognizerState {
    /// Still watching; arena has not resolved.
    Possible,
    /// Will claim as soon as arena opens (no conflict observed yet).
    Eager,
    /// Defers to other recognizers until they reject.
    Defer,
    /// Locked in as the winner.
    Claimed,
    /// Permanently out of this pointer's arena.
    Rejected,
}

/// Handle for a recognizer to push a resolved SemanticEvent.
pub struct ArenaHandle<'a> {
    pub pending_semantic: &'a mut Option<SemanticEvent>,
}

impl<'a> ArenaHandle<'a> {
    pub fn emit(&mut self, event: SemanticEvent) {
        *self.pending_semantic = Some(event);
    }
}

/// Recognizer trait. Implementations are usually per-pointer, but
/// multi-pointer recognizers (Scale/Rotate) may be shared across arenas.
pub trait Recognizer {
    fn id(&self) -> RecognizerId;
    fn kind(&self) -> &'static str;
    fn node(&self) -> NodeKey;
    fn state(&self) -> RecognizerState;

    /// Consume a pointer event; update internal state. Returns the new state.
    fn handle_pointer(
        &mut self,
        event: &PointerEvent,
        arena: &mut ArenaHandle<'_>,
    ) -> RecognizerState;

    /// Called by the arena when this recognizer wins.
    fn accept(&mut self, arena: &mut ArenaHandle<'_>);

    /// Called by the arena when this recognizer loses.
    fn reject(&mut self);

    /// Called once per frame by the host adapter; enables time-based
    /// recognizers (LongPress, double-tap timeout) to wake up.
    fn tick(&mut self, _now: std::time::Instant, _arena: &mut ArenaHandle<'_>) {}
}
