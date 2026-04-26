//! PointerRouter — top-level dispatcher. Creates arenas per pointer id,
//! collects applicable recognizers from the hit path, and feeds events in.
//!
//! Recognizer discovery rule (MVP): every node on the hit path gets a
//! Tap, Pan, LongPress, and Hover recognizer attached. A real
//! implementation would consult the node's `events` map to only build
//! recognizers that have at least one declared handler, but the extra
//! arena members are harmless — unused handlers simply have empty
//! ActionLists.
//!
//! # Multi-pointer recognizers (Scale / Rotate)
//!
//! Per Plan 5 §B.2 these live OUTSIDE the per-pointer arenas. A second
//! pointer Down on the same scale-target appends its id to an existing
//! recognizer instance instead of spawning a fresh arena that loses the
//! first finger. The router fans each pointer event out to every multi
//! recognizer the pointer participates in. When a multi recognizer
//! claims, the router broadcasts a cancellation to every per-pointer
//! arena that fed it — an unresolved Tap/Pan/LongPress on those
//! pointers loses to the cross-arena gesture. If a per-pointer arena
//! is already resolved (Tap won on Up before the multi recognizer
//! crossed threshold), the multi claim is rejected (too late).

use super::arena::Arena;
use super::hit::{hit_test, HitPath};
use super::pointer::{PointerEvent, PointerPhase};
use super::raw::find_raw_root;
use super::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use super::recognizers::{
    HoverRecognizer, LongPressRecognizer, PanRecognizer, RotateRecognizer, ScaleRecognizer,
    TapRecognizer,
};
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
    /// Cross-arena recognizer pool. Plan 5 §B.2's `multi`. Owns each
    /// multi-pointer recognizer (Scale / Rotate) by id; values are
    /// boxed `dyn Recognizer` so future kinds drop in without churning
    /// the storage layout.
    multi: HashMap<RecognizerId, Box<dyn Recognizer>>,
    /// Plan 5 §B.2's `shared`. RecognizerId → list of pointer ids
    /// currently feeding it. Updated on Down/Up.
    shared: HashMap<RecognizerId, Vec<u32>>,
    /// Plan 5 §B.2's `multi_instances`. (NodeKey, recognizer kind) →
    /// instance id, so a second pointer landing on the same scale
    /// target finds the existing recognizer instead of spawning a
    /// duplicate.
    multi_instances: HashMap<(NodeKey, &'static str), RecognizerId>,
}

impl PointerRouter {
    pub fn new() -> Self {
        Self {
            arenas: HashMap::new(),
            raw_roots: HashMap::new(),
            next_id: 1,
            last_hover_target: None,
            last_tap: None,
            multi: HashMap::new(),
            shared: HashMap::new(),
            multi_instances: HashMap::new(),
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
                // Multi-pointer recognizer registration (Plan 5 §B.2).
                // Walk the hit path; for every node that declares any
                // `events.onScale*` / `events.onRotate*`, attach this
                // pointer to the (possibly-new) recognizer instance.
                self.register_multi_pointers(&path, doc, pid);
            }
        }

        let mut out = Vec::new();
        if let Some(&root) = self.raw_roots.get(&pid) {
            out.push(SemanticEvent::RawPointer {
                node: root,
                phase: event.phase,
                position: event.position,
            });
        } else {
            // Multi-pointer dispatch FIRST so a two-finger pinch wins
            // over the per-pointer Pan threshold. A 100px Move that
            // satisfies Pan's 8px slop is also the same input that
            // crosses Scale's 5% threshold — running per-pointer
            // first lets Pan claim, after which Scale rejects as
            // "too late". With multi-first ordering, Scale claims and
            // cancels the per-pointer arenas BEFORE they get to see
            // the move.
            self.dispatch_multi(&event, &mut out);
            if let Some(arena) = self.arenas.get_mut(&pid) {
                arena.dispatch(&event, doc);
                out.extend(arena.drain_emitted());
            }
        }

        // Synthesize DoubleTap at the router level: in-arena recognizers can't
        // track it because a Down always starts a fresh arena.
        let now = event.timestamp;
        self.synthesize_double_tap(&mut out, now);

        if matches!(event.phase, PointerPhase::Up | PointerPhase::Cancel) {
            self.arenas.remove(&pid);
            self.raw_roots.remove(&pid);
            self.unregister_multi_pointer(pid);
        }

        out
    }

    /// Walk `path` from topmost to root; for each node that declares
    /// any `events.onScale*` / `events.onRotate*` handler, find the
    /// existing recognizer instance for that (node, kind) pair (or
    /// allocate one) and append `pid` to its participant list.
    fn register_multi_pointers(&mut self, path: &HitPath, doc: &RuntimeDocument, pid: u32) {
        for &node in &path.0 {
            let handlers = handler_kinds(doc, node);
            if handlers.scale {
                let id = self.find_or_create_multi(node, "Scale", |id| {
                    Box::new(ScaleRecognizer::new(id, node))
                });
                self.shared.entry(id).or_default().push(pid);
            }
            if handlers.rotate {
                let id = self.find_or_create_multi(node, "Rotate", |id| {
                    Box::new(RotateRecognizer::new(id, node))
                });
                self.shared.entry(id).or_default().push(pid);
            }
        }
    }

    fn find_or_create_multi(
        &mut self,
        node: NodeKey,
        kind: &'static str,
        build: impl FnOnce(RecognizerId) -> Box<dyn Recognizer>,
    ) -> RecognizerId {
        if let Some(&id) = self.multi_instances.get(&(node, kind)) {
            return id;
        }
        let id = self.alloc_id();
        self.multi.insert(id, build(id));
        self.multi_instances.insert((node, kind), id);
        id
    }

    /// Feed `event` to every multi-pointer recognizer this pointer
    /// participates in. If a recognizer claims, broadcast cancellation
    /// to all per-pointer arenas in its `shared` set — except
    /// already-resolved arenas, which would mean the multi claim
    /// arrived too late (Tap / Pan already won that pointer).
    fn dispatch_multi(&mut self, event: &PointerEvent, out: &mut Vec<SemanticEvent>) {
        let pid = event.id.0;
        // Snapshot the recognizer ids this pointer feeds so we can
        // mutate `self.multi` without holding a borrow on `shared`.
        let rids: Vec<RecognizerId> = self
            .shared
            .iter()
            .filter_map(|(rid, pids)| pids.contains(&pid).then_some(*rid))
            .collect();
        for rid in rids {
            let Some(recog) = self.multi.get_mut(&rid) else {
                continue;
            };
            if matches!(recog.state(), RecognizerState::Rejected) {
                continue;
            }
            let mut pending = None;
            let mut handle = ArenaHandle {
                pending_semantic: &mut pending,
            };
            let new_state = recog.handle_pointer(event, &mut handle);
            if let Some(ev) = pending {
                out.push(ev);
            }
            // Re-borrow-free: clone the participant list before we
            // mutate per-pointer arenas.
            let participants: Vec<u32> = self.shared.get(&rid).cloned().unwrap_or_default();
            if matches!(new_state, RecognizerState::Claimed) {
                // Plan 5 §B.2: any already-resolved arena means the
                // multi claim is too late. Reject the recognizer; no
                // event emitted (the Start payload above is followed
                // by a quiet Reject — recognizer impls only emit
                // Start once, so no orphan End).
                let too_late = participants
                    .iter()
                    .any(|p| self.arenas.get(p).map(Arena::is_resolved).unwrap_or(false));
                if too_late {
                    self.multi.get_mut(&rid).unwrap().reject();
                    continue;
                }
                // Cancel each unresolved per-pointer arena: the multi
                // gesture wins, single-pointer Tap / Pan / LongPress
                // on these pointers lose.
                for p in &participants {
                    if let Some(arena) = self.arenas.get_mut(p) {
                        if !arena.is_resolved() {
                            arena.cancel_all();
                        }
                    }
                }
            }
        }
    }

    /// Drop `pid` from every `shared[id]` it appears in. Empty
    /// recognizer instances are removed entirely (and from
    /// `multi_instances`) so a future Down on a different scale
    /// target re-derives without stale state.
    fn unregister_multi_pointer(&mut self, pid: u32) {
        let mut to_drop: Vec<RecognizerId> = Vec::new();
        for (rid, pids) in self.shared.iter_mut() {
            pids.retain(|p| *p != pid);
            if pids.is_empty() {
                to_drop.push(*rid);
            }
        }
        for rid in &to_drop {
            // Give the recognizer a chance to emit ScaleEnd / RotateEnd
            // before we drop it. Spec: pointer Up that drops the
            // participant count below 2 ends the gesture.
            self.shared.remove(rid);
            self.multi.remove(rid);
        }
        if !to_drop.is_empty() {
            self.multi_instances.retain(|_, v| !to_drop.contains(v));
        }
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

/// Which multi-pointer recognizer kinds a node opts into.
#[derive(Default, Clone, Copy)]
struct HandlerKinds {
    scale: bool,
    rotate: bool,
}

/// Inspect the node's `events` map and return which multi-pointer
/// recognizer kinds it declares handlers for. Round-trip via JSON
/// for parity with `dispatch_event`'s `extract_handler` — the schema
/// types are per-variant so direct field access would need a match
/// arm per `PenNode` variant.
fn handler_kinds(doc: &RuntimeDocument, key: NodeKey) -> HandlerKinds {
    let Some(data) = doc.tree.nodes.get(key) else {
        return HandlerKinds::default();
    };
    let Ok(v) = serde_json::to_value(&data.schema) else {
        return HandlerKinds::default();
    };
    let Some(events) = v.as_object().and_then(|o| o.get("events")) else {
        return HandlerKinds::default();
    };
    let Some(events) = events.as_object() else {
        return HandlerKinds::default();
    };
    let scale = events.contains_key("onScaleStart")
        || events.contains_key("onScaleUpdate")
        || events.contains_key("onScaleEnd");
    let rotate = events.contains_key("onRotateStart")
        || events.contains_key("onRotateUpdate")
        || events.contains_key("onRotateEnd");
    HandlerKinds { scale, rotate }
}
