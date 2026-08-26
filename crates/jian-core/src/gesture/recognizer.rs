//! Recognizer trait — each implementation runs a state machine on a stream
//! of PointerEvents and decides Claim / Reject via the arena.

use super::pointer::{MouseButtons, PointerEvent};
use super::semantic::{PointerFacts, SemanticEvent, SemanticEventEnvelope};
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
///
/// The handle itself is public (external hosts use it for custom
/// recognizers); the only narrow source break vs. the pre-R2A surface is
/// the pending field type: it holds a `SemanticEventEnvelope` (event +
/// factual pointer/gesture metadata) instead of a bare `SemanticEvent`,
/// so enclosing metadata is never discarded at the arena boundary.
///
/// Pending events are drained by the arena/router after every dispatch.
/// Recognizers must attach the factual pointer metadata they captured
/// from the triggering `PointerEvent` — never reconstruct it later.
pub struct ArenaHandle<'a> {
    pub pending_semantic: &'a mut Option<SemanticEventEnvelope>,
}

impl<'a> ArenaHandle<'a> {
    /// Source-compatible one-argument emit: a plain envelope with no
    /// pointer or gesture facts (non-pointer events / hosts that do not
    /// track facts). Internal recognizers attach facts via
    /// [`Self::emit_with_facts`] / [`Self::emit_with`].
    pub fn emit(&mut self, event: SemanticEvent) {
        *self.pending_semantic = Some(SemanticEventEnvelope::plain(event));
    }

    /// Internal factual emit: attach the pointer metadata captured from
    /// the triggering `PointerEvent`.
    pub fn emit_with_facts(&mut self, event: SemanticEvent, facts: PointerFacts) {
        *self.pending_semantic = Some(SemanticEventEnvelope {
            event,
            pointer_facts: Some(facts),
            gesture: Default::default(),
        });
    }

    /// Emit an event carrying gesture facts alongside pointer facts.
    pub fn emit_with(
        &mut self,
        event: SemanticEvent,
        facts: PointerFacts,
        gesture: super::semantic::GestureFacts,
    ) {
        *self.pending_semantic = Some(SemanticEventEnvelope {
            event,
            pointer_facts: Some(facts),
            gesture,
        });
    }
}

/// PointerFacts helper: initiating-button continuity.
///
/// A gesture started by a provable single-button Down keeps that button
/// across its whole envelope stream, while every other fact stays from
/// the event that actually triggered the envelope (phase/position/
/// timestamp/buttons). Live in `recognizer.rs` beside the recognizers
/// that use it — `semantic.rs` only hosts the data.
impl PointerFacts {
    /// Re-attach the initiating Down's provable button to `self`.
    ///
    /// `initiating` must be the `button` captured from the gesture's
    /// initiating `Down` (`PointerFacts::from_event(down).button`): `Some`
    /// exactly when that Down's bitmask had one bit, so an ambiguous
    /// multi-button Down contributes `None` and the key stays absent.
    /// A triggering event that is itself a factual Down keeps its own
    /// value.
    pub fn with_initiating_button(mut self, initiating: Option<MouseButtons>) -> Self {
        self.button = self.button.or(initiating);
        self
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

    /// Called by the arena when this recognizer wins. Claim-time events
    /// (Tap/PanStart/LongPress) are emitted here — AFTER losers were
    /// rejected — so a cancellation produced by the win is emitted before
    /// the winner's semantic event.
    fn accept(&mut self, arena: &mut ArenaHandle<'_>);

    /// Legacy rejection hook: mark this recognizer as out of the arena.
    /// The arena calls the handle-aware [`Self::reject_with_handle`],
    /// whose default implementation delegates here; recognizers with
    /// active output (Press) override the handle-aware method to emit
    /// their cancellation event instead of silently dropping it.
    fn reject(&mut self);

    /// Handle-aware rejection, called by the arena. Default: delegate to
    /// the legacy [`Self::reject`].
    fn reject_with_handle(&mut self, _arena: &mut ArenaHandle<'_>) {
        self.reject();
    }

    /// Witness-only feed used by cross-arena coordination: update factual
    /// state (e.g. the Press recognizer's last-observed `PointerFacts`)
    /// WITHOUT emitting or claiming. Default: no-op.
    fn witness_pointer(&mut self, _event: &PointerEvent) {}

    /// Whether this recognizer still accepts another participant pointer.
    /// Cross-pointer transforms own at most two fingers; the router
    /// consults this before appending a third so a stray finger stays
    /// independent (its Up can never fire a spurious transform End).
    /// Default: unlimited. R2B2 third-finger contract.
    fn has_participant_capacity(&self) -> bool {
        true
    }

    /// Router refresh hook: called immediately before each pointer event
    /// is fed to this recognizer, with the CURRENT state-aware
    /// `gestures.disabled` predicate. Recognizers that captured a
    /// handler-owner node + config at Down time invalidate their session
    /// here when that owner became dynamically disabled mid-gesture.
    /// Default: no-op.
    fn refresh_node_disabled(&mut self, _node_disabled: &dyn Fn(NodeKey) -> bool) {}

    /// Called once per frame by the host adapter; enables time-based
    /// recognizers (LongPress, double-tap timeout) to wake up.
    fn tick(&mut self, _now_ms: u64, _arena: &mut ArenaHandle<'_>) {}

    fn next_wake_ms(&self) -> Option<u64> {
        None
    }
}
