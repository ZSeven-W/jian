//! PressRecognizer — the passive press-witness recognizer.
//!
//! One instance per pointer (not per node), attached to the Down's
//! captured target:
//!
//! - `Down` emits `PressStart` (with the Down's factual pointer facts).
//! - A host `Cancel` emits `PressCancel` exactly once.
//! - An **unclaimed** `Up` emits `PressEnd` (the arena still resolved to
//!   a winner afterwards — conventionally Tap — but the press was not
//!   canceled, so it ends normally).
//! - Any other claim (Pan/LongPress/Scale/Rotate) rejects this
//!   recognizer, which emits `PressCancel` exactly once via `reject` —
//!   the arena rejection plumbing hands the recognizer an `ArenaHandle`
//!   precisely so cancellation events are never silently lost.
//!
//! `Press*` events keep the captured target (`self.node`) even when the
//! release moves outside it; positions are the factual event positions.

use crate::document::NodeKey;
use crate::gesture::pointer::{MouseButtons, PointerEvent, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::{PointerFacts, SemanticEvent};

pub struct PressRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    /// Whether `PressStart` was emitted (Down was seen).
    started: bool,
    /// Whether `PressEnd` or `PressCancel` was delivered — the press
    /// session ends exactly once, whichever way it ends.
    finished: bool,
    /// The initiating Down's provable single button — retained on
    /// PressEnd/PressCancel facts while phase/position/timestamp/buttons
    /// stay from the triggering event. `None` when the Down was
    /// button-less or ambiguous (keeps the key absent).
    down_button: Option<MouseButtons>,
    /// Most recent facts; `reject`/`Cancel` use the last observed event.
    last_facts: Option<PointerFacts>,
}

impl PressRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            started: false,
            finished: false,
            down_button: None,
            last_facts: None,
        }
    }

    fn finish(&mut self, event: SemanticEvent, arena: &mut ArenaHandle<'_>) {
        if self.started && !self.finished {
            if let Some(facts) = self.last_facts.clone() {
                arena.emit_with_facts(event, facts.with_initiating_button(self.down_button));
            }
            self.finished = true;
        }
    }
}

impl Recognizer for PressRecognizer {
    fn id(&self) -> RecognizerId {
        self.id
    }
    fn kind(&self) -> &'static str {
        "Press"
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
        let facts = PointerFacts::from_event(event);
        match event.phase {
            PointerPhase::Down => {
                if !self.started {
                    arena.emit_with_facts(
                        SemanticEvent::PressStart {
                            node: self.node,
                            position: event.position,
                        },
                        facts.clone(),
                    );
                    self.started = true;
                }
                self.down_button = facts.button;
                self.last_facts = Some(facts);
            }
            PointerPhase::Move => {
                self.last_facts = Some(facts);
            }
            PointerPhase::Up => {
                // Unclaimed Up: the press ends normally. The arena may
                // still resolve to a winner (e.g. Tap) — this recognizer
                // stays Possible so the resolution rejection does not
                // re-classify the end as a cancel.
                if self.started && !self.finished {
                    arena.emit_with_facts(
                        SemanticEvent::PressEnd {
                            node: self.node,
                            position: event.position,
                        },
                        facts.clone().with_initiating_button(self.down_button),
                    );
                    self.finished = true;
                }
                self.last_facts = Some(facts);
            }
            PointerPhase::Cancel => {
                if self.started && !self.finished {
                    arena.emit_with_facts(
                        SemanticEvent::PressCancel {
                            node: self.node,
                            position: event.position,
                        },
                        facts.clone().with_initiating_button(self.down_button),
                    );
                    self.finished = true;
                }
                self.state = RecognizerState::Rejected;
            }
            PointerPhase::Hover => {
                self.last_facts = Some(facts);
            }
        }
        self.state
    }

    fn accept(&mut self, _arena: &mut ArenaHandle<'_>) {
        self.state = RecognizerState::Claimed;
    }

    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
    }

    fn reject_with_handle(&mut self, arena: &mut ArenaHandle<'_>) {
        if self.started && !self.finished {
            // Another member won; the active press is canceled exactly once.
            let position = self
                .last_facts
                .as_ref()
                .map(|f| f.position)
                .unwrap_or_default();
            self.finish(
                SemanticEvent::PressCancel {
                    node: self.node,
                    position,
                },
                arena,
            );
        }
        self.state = RecognizerState::Rejected;
    }

    fn witness_pointer(&mut self, event: &PointerEvent) {
        // Witness-only feed: refresh the factual metadata (used by a
        // PressCancel that a cross-arena claim triggers on the NEXT
        // dispatch) without emitting or claiming. The same `last_facts`
        // are what `reject_with_handle` reports.
        self.last_facts = Some(PointerFacts::from_event(event));
    }
}
