//! PointerRouter — top-level dispatcher. Creates arenas per pointer id,
//! collects applicable recognizers from the hit path, and feeds events in.
//!
//! Recognizer discovery rule (MVP): every node on the hit path gets a
//! Tap, Pan, LongPress, and Hover recognizer attached. A real
//! implementation would consult the node's `events` map to only build
//! recognizers that have at least one declared handler, but the extra
//! arena members are harmless — unused handlers simply have empty
//! ActionLists.

use super::arena::Arena;
use super::hit::{hit_test, HitPath};
use super::pointer::{PointerEvent, PointerPhase};
use super::raw::find_raw_root;
use super::recognizer::{Recognizer, RecognizerId};
use super::recognizers::{HoverRecognizer, LongPressRecognizer, PanRecognizer, TapRecognizer};
use super::semantic::SemanticEvent;
use crate::document::{NodeKey, RuntimeDocument};
use crate::spatial::SpatialIndex;
use std::collections::HashMap;
use std::time::Instant;

pub struct PointerRouter {
    arenas: HashMap<u32, Arena>,
    /// Pointers whose Down was inside a `rawPointer` subtree. For these we
    /// bypass arena arbitration and emit `RawPointer` events straight to the
    /// declared root node.
    raw_roots: HashMap<u32, NodeKey>,
    next_id: RecognizerId,
    last_hover_target: Option<NodeKey>,
}

impl PointerRouter {
    pub fn new() -> Self {
        Self {
            arenas: HashMap::new(),
            raw_roots: HashMap::new(),
            next_id: 1,
            last_hover_target: None,
        }
    }

    fn alloc_id(&mut self) -> RecognizerId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn dispatch(
        &mut self,
        event: PointerEvent,
        doc: &RuntimeDocument,
        spatial: &SpatialIndex,
    ) -> Vec<SemanticEvent> {
        // Hover handled separately: no arena, no claiming.
        if matches!(event.phase, PointerPhase::Hover) {
            return self.handle_hover(&event, doc, spatial);
        }

        let pid = event.id.0;

        if matches!(event.phase, PointerPhase::Down) {
            let path = hit_test(spatial, doc, event.position);
            if let Some(root) = find_raw_root(&path, doc) {
                self.raw_roots.insert(pid, root);
            } else {
                let arena = self.build_arena(&path);
                self.arenas.insert(pid, arena);
            }
        }

        let mut out = Vec::new();
        if let Some(&root) = self.raw_roots.get(&pid) {
            out.push(SemanticEvent::RawPointer {
                node: root,
                phase: event.phase,
                position: event.position,
            });
        } else if let Some(arena) = self.arenas.get_mut(&pid) {
            arena.dispatch(&event, doc);
            out.extend(arena.drain_emitted());
        }

        if matches!(event.phase, PointerPhase::Up | PointerPhase::Cancel) {
            self.arenas.remove(&pid);
            self.raw_roots.remove(&pid);
        }

        out
    }

    fn build_arena(&mut self, path: &HitPath) -> Arena {
        let mut members: Vec<Box<dyn Recognizer>> = Vec::with_capacity(path.0.len() * 3);
        for &node in &path.0 {
            members.push(Box::new(TapRecognizer::new(self.alloc_id(), node)));
            members.push(Box::new(PanRecognizer::new(self.alloc_id(), node)));
            members.push(Box::new(LongPressRecognizer::new(self.alloc_id(), node)));
        }
        Arena::new(members)
    }

    fn handle_hover(
        &mut self,
        event: &PointerEvent,
        doc: &RuntimeDocument,
        spatial: &SpatialIndex,
    ) -> Vec<SemanticEvent> {
        let path = hit_test(spatial, doc, event.position);
        let target = path.topmost();
        let mut out = Vec::new();
        if target != self.last_hover_target {
            if let Some(prev) = self.last_hover_target {
                out.push(SemanticEvent::HoverLeave {
                    node: prev,
                    position: event.position,
                });
            }
            if let Some(cur) = target {
                let mut r = HoverRecognizer::new(self.alloc_id(), cur);
                let mut pending = None;
                let mut h = super::recognizer::ArenaHandle {
                    pending_semantic: &mut pending,
                };
                let _ = r.handle_pointer(event, &mut h);
                if let Some(ev) = pending {
                    out.push(ev);
                }
            }
            self.last_hover_target = target;
        }
        out
    }

    /// Drive LongPress and other timer-based recognizers. Host should call
    /// every frame (or at ≥ 100 Hz for responsive long-press).
    pub fn tick(&mut self, now: Instant) -> Vec<SemanticEvent> {
        let mut out = Vec::new();
        for arena in self.arenas.values_mut() {
            for r in arena.members_mut() {
                let mut pending = None;
                let mut h = super::recognizer::ArenaHandle {
                    pending_semantic: &mut pending,
                };
                r.tick(now, &mut h);
                if let Some(ev) = pending {
                    out.push(ev);
                }
            }
        }
        out
    }
}

impl Default for PointerRouter {
    fn default() -> Self {
        Self::new()
    }
}
