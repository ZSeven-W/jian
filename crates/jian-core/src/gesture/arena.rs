//! Per-pointer Arena — runs the recognizer arbitration state machine.
//!
//! Flutter-style semantics:
//! - On pointer-down, collect all applicable recognizers on the hit path.
//! - Each subsequent event is routed to every still-Possible member.
//! - The first recognizer to return `Claimed` wins; all others are Rejected.
//! - If pointer-up arrives with no winner, pick by priority (depth, kind).
//!
//! # Cancellation plumbing
//!
//! `reject` receives an `ArenaHandle` so an active recognizer (Press) can
//! emit its cancellation instead of silently dropping it. Ordering is
//! deterministic: losers are rejected before the winner is accepted, and
//! claim-time events are emitted from `accept` — so for a Pan claim the
//! stream is `[PressCancel, PanStart]`, and for an unclaimed Up
//! `[PressEnd, Tap]`.

use super::pointer::{PointerEvent, PointerPhase};
use super::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use super::semantic::{SemanticEvent, SemanticEventEnvelope};
use crate::document::RuntimeDocument;

pub struct Arena {
    members: Vec<Box<dyn Recognizer>>,
    resolved: Option<RecognizerId>,
    emitted: Vec<SemanticEventEnvelope>,
}

impl Arena {
    pub fn new(members: Vec<Box<dyn Recognizer>>) -> Self {
        Self {
            members,
            resolved: None,
            emitted: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
    pub fn len(&self) -> usize {
        self.members.len()
    }
    pub fn is_resolved(&self) -> bool {
        self.resolved.is_some()
    }

    /// Envelope-returning drain — used by the router so factual pointer
    /// metadata stays attached end-to-end.
    pub fn drain_envelopes(&mut self) -> Vec<SemanticEventEnvelope> {
        std::mem::take(&mut self.emitted)
    }

    /// Source-compatible drain of the semantic events (facts dropped) —
    /// kept for external arena users (jian-gallery).
    pub fn drain_emitted(&mut self) -> Vec<SemanticEvent> {
        self.drain_envelopes()
            .into_iter()
            .map(|envelope| envelope.event)
            .collect()
    }

    /// Feed a pointer event to every still-Possible recognizer. Returns any
    /// semantic events produced as a side-effect of this dispatch.
    pub fn dispatch(&mut self, event: &PointerEvent, doc: &RuntimeDocument) {
        // Fast path: already resolved → only the winner sees further events.
        if let Some(winner_id) = self.resolved {
            for r in &mut self.members {
                if r.id() == winner_id {
                    let mut pending = None;
                    let mut handle = ArenaHandle {
                        pending_semantic: &mut pending,
                    };
                    let _ = r.handle_pointer(event, &mut handle);
                    if let Some(ev) = pending {
                        self.emitted.push(ev);
                    }
                    break;
                }
            }
            return;
        }

        // Unresolved: feed all still-Possible members.
        let mut winner_idx: Option<usize> = None;
        for (idx, r) in self.members.iter_mut().enumerate() {
            if matches!(r.state(), RecognizerState::Rejected) {
                continue;
            }
            let mut pending = None;
            let mut handle = ArenaHandle {
                pending_semantic: &mut pending,
            };
            let s = r.handle_pointer(event, &mut handle);
            if let Some(ev) = pending {
                self.emitted.push(ev);
            }
            if matches!(s, RecognizerState::Claimed) {
                winner_idx = Some(idx);
                break;
            }
        }

        if let Some(idx) = winner_idx {
            self.resolve(idx);
            return;
        }

        // Pointer-up with no winner: pick by priority.
        if matches!(event.phase, PointerPhase::Up) {
            self.resolve_by_priority(doc);
        }
    }

    /// Select the highest-priority still-Possible member and resolve.
    fn resolve_by_priority(&mut self, doc: &RuntimeDocument) {
        let mut best: Option<(usize, (u32, u32, RecognizerId))> = None;
        for (idx, r) in self.members.iter().enumerate() {
            if matches!(r.state(), RecognizerState::Rejected) {
                continue;
            }
            let (depth, kind_p) = super::priority::rank(r.as_ref(), doc);
            let key = (depth, kind_p, r.id());
            match best {
                None => best = Some((idx, key)),
                Some((_, ref cur)) => {
                    // Higher depth wins; then higher kind-priority;
                    // then lower id (deterministic).
                    if key.0 > cur.0
                        || (key.0 == cur.0 && key.1 > cur.1)
                        || (key.0 == cur.0 && key.1 == cur.1 && key.2 < cur.2)
                    {
                        best = Some((idx, key));
                    }
                }
            }
        }
        if let Some((idx, _)) = best {
            self.resolve(idx);
        }
    }

    fn resolve(&mut self, winner_idx: usize) {
        let winner_id = self.members[winner_idx].id();
        self.resolved = Some(winner_id);
        // Reject losers FIRST (each via the handle-aware bridge so active
        // recognizers emit cancellations), then accept the winner, whose
        // claim-time event is emitted from `accept`. This yields
        // `[…cancellations, <winner event>]` in `emitted`.
        let mut winner_pending = None;
        for (idx, r) in self.members.iter_mut().enumerate() {
            let mut pending = None;
            let mut handle = ArenaHandle {
                pending_semantic: &mut pending,
            };
            if idx == winner_idx {
                r.accept(&mut handle);
                winner_pending = pending;
            } else if !matches!(r.state(), RecognizerState::Rejected) {
                r.reject_with_handle(&mut handle);
                if let Some(ev) = pending {
                    self.emitted.push(ev);
                }
            }
        }
        if let Some(ev) = winner_pending {
            self.emitted.push(ev);
        }
    }

    /// Visit all members (mutable) — used by router for cross-arena coordination.
    pub fn members_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Recognizer>> {
        self.members.iter_mut()
    }

    pub fn next_wake_ms(&self) -> Option<u64> {
        self.members
            .iter()
            .filter(|recognizer| !matches!(recognizer.state(), RecognizerState::Rejected))
            .filter_map(|recognizer| recognizer.next_wake_ms())
            .min()
    }

    /// Reject every still-Possible member and mark the arena as
    /// resolved. Used when a cross-arena recognizer (Scale / Rotate)
    /// claims its multi-pointer gesture — the per-pointer arenas it
    /// participated in are pre-empted, so a single-finger Tap / Pan
    /// / LongPress on those pointers loses to the multi gesture.
    /// `resolved` gets a synthetic id (u64::MAX) so subsequent
    /// `dispatch` calls take the fast path and feed nothing further.
    ///
    /// Returns the cancellation events produced by the rejection (an
    /// active Press emits `PressCancel`) so the router can order them
    /// BEFORE the multi recognizer's claim event.
    pub fn cancel_all(&mut self) -> Vec<SemanticEventEnvelope> {
        let mut cancels = Vec::new();
        if self.resolved.is_some() {
            return cancels;
        }
        for r in &mut self.members {
            if !matches!(r.state(), RecognizerState::Rejected) {
                let mut pending = None;
                let mut handle = ArenaHandle {
                    pending_semantic: &mut pending,
                };
                r.reject_with_handle(&mut handle);
                if let Some(ev) = pending {
                    cancels.push(ev);
                }
            }
        }
        self.resolved = Some(u64::MAX);
        cancels
    }

    /// Witness-only feed: refresh factual state (the Press recognizer's
    /// last-observed `PointerFacts`) with `event` WITHOUT letting any
    /// recognizer claim. Called by the router before a cross-arena
    /// multi-pointer claim cancels this arena, so the triggering
    /// pointer's `PressCancel` carries the current event's facts instead
    /// of stale ones — and a Pan/Tap cannot win the arena off the back
    /// of the cancelling event.
    pub fn witness_press(&mut self, event: &PointerEvent) {
        for r in &mut self.members {
            r.witness_pointer(event);
        }
    }

    /// Drive `tick()` on every still-Possible member. If one of them claims
    /// as a side effect (LongPress is the canonical case), resolve the
    /// arena — accept the winner, reject everyone else — so that the
    /// next pointer event doesn't let a competing recognizer also claim.
    pub fn tick(&mut self, now_ms: u64) {
        if let Some(winner_id) = self.resolved {
            // Still route ticks to the winner in case it wants to emit
            // follow-up events (e.g. pan velocity). No resolution needed.
            for r in &mut self.members {
                if r.id() == winner_id {
                    let mut pending = None;
                    let mut handle = ArenaHandle {
                        pending_semantic: &mut pending,
                    };
                    r.tick(now_ms, &mut handle);
                    if let Some(ev) = pending {
                        self.emitted.push(ev);
                    }
                    break;
                }
            }
            return;
        }

        let mut winner_idx: Option<usize> = None;
        for (idx, r) in self.members.iter_mut().enumerate() {
            if matches!(r.state(), RecognizerState::Rejected) {
                continue;
            }
            let mut pending = None;
            let mut handle = ArenaHandle {
                pending_semantic: &mut pending,
            };
            r.tick(now_ms, &mut handle);
            if let Some(ev) = pending {
                self.emitted.push(ev);
            }
            if matches!(r.state(), RecognizerState::Claimed) && winner_idx.is_none() {
                winner_idx = Some(idx);
            }
        }
        if let Some(idx) = winner_idx {
            self.resolve(idx);
        }
    }
}
