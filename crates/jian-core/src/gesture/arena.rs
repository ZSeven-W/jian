//! Per-pointer Arena — runs the recognizer arbitration state machine.
//!
//! Flutter-style semantics:
//! - On pointer-down, collect all applicable recognizers on the hit path.
//! - Each subsequent event is routed to every still-Possible member.
//! - The first recognizer to return `Claimed` wins; all others are Rejected.
//! - If pointer-up arrives with no winner, pick by priority (depth, kind).

use super::pointer::{PointerEvent, PointerPhase};
use super::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use super::semantic::SemanticEvent;
use crate::document::RuntimeDocument;

pub struct Arena {
    members: Vec<Box<dyn Recognizer>>,
    resolved: Option<RecognizerId>,
    emitted: Vec<SemanticEvent>,
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

    pub fn drain_emitted(&mut self) -> Vec<SemanticEvent> {
        std::mem::take(&mut self.emitted)
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
        // Accept winner + reject the rest.
        for (idx, r) in self.members.iter_mut().enumerate() {
            let mut pending = None;
            let mut handle = ArenaHandle {
                pending_semantic: &mut pending,
            };
            if idx == winner_idx {
                r.accept(&mut handle);
            } else {
                r.reject();
            }
            if let Some(ev) = pending {
                self.emitted.push(ev);
            }
        }
    }

    /// Visit all members (mutable) — used by router for cross-arena coordination.
    pub fn members_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Recognizer>> {
        self.members.iter_mut()
    }

    /// Drive `tick()` on every still-Possible member. If one of them claims
    /// as a side effect (LongPress is the canonical case), resolve the
    /// arena — accept the winner, reject everyone else — so that the
    /// next pointer event doesn't let a competing recognizer also claim.
    pub fn tick(&mut self, now: std::time::Instant) {
        if self.resolved.is_some() {
            // Still route ticks to the winner in case it wants to emit
            // follow-up events (e.g. pan velocity). No resolution needed.
            if let Some(winner_id) = self.resolved {
                for r in &mut self.members {
                    if r.id() == winner_id {
                        let mut pending = None;
                        let mut handle = ArenaHandle {
                            pending_semantic: &mut pending,
                        };
                        r.tick(now, &mut handle);
                        if let Some(ev) = pending {
                            self.emitted.push(ev);
                        }
                        break;
                    }
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
            r.tick(now, &mut handle);
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
