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
use crate::geometry::Point;
use crate::spatial::SpatialIndex;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Max gap between the two Taps of a DoubleTap.
const DOUBLE_TAP_GAP: Duration = Duration::from_millis(300);
/// Max pixel distance between the two Taps of a DoubleTap.
const DOUBLE_TAP_SLOP_PX: f32 = 16.0;

pub struct PointerRouter {
    arenas: HashMap<u32, Arena>,
    /// Pointers whose Down was inside a `rawPointer` subtree. For these we
    /// bypass arena arbitration and emit `RawPointer` events straight to the
    /// declared root node.
    raw_roots: HashMap<u32, NodeKey>,
    next_id: RecognizerId,
    last_hover_target: Option<NodeKey>,
    /// Most recent Tap per node — used by the router-level DoubleTap
    /// synthesizer. Tap state crosses pointer-sequences, which is why the
    /// in-arena DoubleTapRecognizer alone can't detect it: each Down opens
    /// a fresh arena.
    last_tap: Option<(NodeKey, Instant, Point)>,
}

impl PointerRouter {
    pub fn new() -> Self {
        Self {
            arenas: HashMap::new(),
            raw_roots: HashMap::new(),
            next_id: 1,
            last_hover_target: None,
            last_tap: None,
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

        // Synthesize DoubleTap at the router level: in-arena recognizers can't
        // track it because a Down always starts a fresh arena.
        let now = event.timestamp;
        self.synthesize_double_tap(&mut out, now);

        if matches!(event.phase, PointerPhase::Up | PointerPhase::Cancel) {
            self.arenas.remove(&pid);
            self.raw_roots.remove(&pid);
        }

        out
    }

    /// Walk the emitted semantic events; for each `Tap`, check whether it
    /// matches the cached previous Tap (same node, within `DOUBLE_TAP_GAP`
    /// and `DOUBLE_TAP_SLOP_PX`). If so, append a `DoubleTap`.
    fn synthesize_double_tap(&mut self, out: &mut Vec<SemanticEvent>, now: Instant) {
        // Collect the indices where a DoubleTap should be inserted right
        // after a matching Tap, to avoid iterator-invalidation.
        let mut insertions: Vec<(usize, SemanticEvent)> = Vec::new();
        for (i, ev) in out.iter().enumerate() {
            let SemanticEvent::Tap { node, position } = ev else {
                continue;
            };
            if let Some((prev_node, prev_t, prev_pos)) = self.last_tap {
                let dt = now.saturating_duration_since(prev_t);
                let dx = position.x - prev_pos.x;
                let dy = position.y - prev_pos.y;
                if prev_node == *node
                    && dt <= DOUBLE_TAP_GAP
                    && (dx * dx + dy * dy).sqrt() <= DOUBLE_TAP_SLOP_PX
                {
                    insertions.push((
                        i + 1,
                        SemanticEvent::DoubleTap {
                            node: *node,
                            position: *position,
                        },
                    ));
                    self.last_tap = None; // Consume; a triple tap is 1 + double.
                    continue;
                }
            }
            self.last_tap = Some((*node, now, *position));
        }
        for (offset, (idx, ev)) in insertions.into_iter().enumerate() {
            out.insert(idx + offset, ev);
        }
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
    ///
    /// Delegates to `Arena::tick`, which resolves the arena if a timer-driven
    /// claim fires — important for LongPress so a subsequent Up doesn't also
    /// let Tap claim on the same pointer sequence.
    pub fn tick(&mut self, now: Instant) -> Vec<SemanticEvent> {
        let mut out = Vec::new();
        for arena in self.arenas.values_mut() {
            arena.tick(now);
            out.extend(arena.drain_emitted());
        }
        out
    }
}

impl Default for PointerRouter {
    fn default() -> Self {
        Self::new()
    }
}
