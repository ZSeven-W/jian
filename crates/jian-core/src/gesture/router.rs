//! PointerRouter — top-level dispatcher. Creates arenas per pointer id,
//! collects applicable recognizers from the hit path, and feeds events in.
//!
//! Recognizer discovery rule (MVP, preserved): every node on the hit path
//! gets a Tap, Pan, and (chain-decision) LongPress recognizer — unused
//! handlers are inert because `EventDispatcher` bubbles to a declared
//! handler or drops the event. The **Press** witness recognizer is
//! installed once per pointer, only when the hit chain declares an enabled
//! press handler (`onPressStart`/`onPressEnd`/`onPressCancel`) — a press
//! session is one gesture, and unnamed presses must stay silent.
//!
//! Handler-aware Tap/DoubleTap: when the hit chain declares `onDoubleTap`,
//! the first Tap is buffered through the authored/default
//! `doubleTapTimeout`/`doubleTapSlop` and flushed exactly once from
//! `PointerRouter::tick` at its deadline even with no further input. A
//! matching second Tap yields only `DoubleTap` (no first- or second-Tap).
//! Chains without `onDoubleTap` deliver Taps immediately (legacy single-
//! tap behavior, which built-in widget activation depends on). The
//! pending-Tap state machine lives in [`router_tap`]; the runtime flushes
//! due actions via the internal [`Self::dispatch_current`] path, so a due
//! Tap is delivered BEFORE the current event's slider side effects,
//! disabled-predicate evaluation, hover semantics and arena routing.
//!
//! # Multi-pointer recognizers (Scale / Rotate)
//!
//! Per Plan 5 §B.2 these live OUTSIDE the per-pointer arenas. A second
//! pointer Down on the same scale-target appends its id to an existing
//! recognizer instance instead of spawning a fresh arena that loses the
//! first finger. The router fans each pointer event out to every multi
//! recognizer the pointer participates in. When a multi recognizer
//! claims, the router broadcasts a cancellation to every per-pointer
//! arena that fed it — an unresolved Tap/Pan/LongPress/Press on those
//! pointers loses to the cross-arena gesture. If a per-pointer arena
//! is already resolved (Tap won on Up before the multi recognizer
//! crossed threshold), the multi claim is rejected (too late).

use super::arena::Arena;
use super::config;
use super::hit::{hit_test, HitPath};
use super::pointer::{MouseButtons, PointerEvent, PointerKind, PointerPhase};
use super::raw::find_raw_root;
use super::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use super::recognizers::{
    HoverRecognizer, LongPressRecognizer, PanRecognizer, PressRecognizer, RotateRecognizer,
    ScaleRecognizer, TapRecognizer,
};
use super::router_tap::{apply_tap_deferral, PendingTap};
use super::semantic::{PointerFacts, SemanticEvent, SemanticEventEnvelope};
use crate::document::{NodeKey, RuntimeDocument};
use crate::spatial::SpatialIndex;
use std::collections::HashMap;

pub struct PointerRouter {
    arenas: HashMap<u32, Arena>,
    /// Pointers whose Down was inside a `rawPointer` subtree. For these we
    /// bypass arena arbitration and emit `RawPointer` events straight to the
    /// declared root node.
    raw_roots: HashMap<u32, NodeKey>,
    /// Pointers whose Down was a provable right-button press (exactly
    /// RIGHT): the press sequence is closed. A chain that declares an
    /// enabled `onContextMenu` emitted exactly ContextMenu; otherwise
    /// the sequence is swallowed. No arena/tap/press events may follow.
    context_menu_pids: HashMap<u32, ()>,
    next_id: RecognizerId,
    last_hover_target: Option<NodeKey>,
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
    /// Handler-aware DoubleTap deferral: one buffered Tap at most.
    pending_tap: Option<PendingTap>,
}

impl PointerRouter {
    pub fn new() -> Self {
        Self {
            arenas: HashMap::new(),
            raw_roots: HashMap::new(),
            context_menu_pids: HashMap::new(),
            next_id: 1,
            last_hover_target: None,
            multi: HashMap::new(),
            shared: HashMap::new(),
            multi_instances: HashMap::new(),
            pending_tap: None,
        }
    }

    fn alloc_id(&mut self) -> RecognizerId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Drop every per-pointer arena, cross-arena recognizer state, and the
    /// buffered pending Tap.
    ///
    /// Hot-reload (`Runtime::replace_document`) swaps the document's
    /// SlotMap underneath. SlotMap keys are *not* globally unique
    /// across SlotMaps — two trees can produce keys that compare
    /// equal — so any `NodeKey` cached in `raw_roots`,
    /// `last_hover_target`, `pending_tap`, or `multi_instances` from the
    /// pre-reload tree could silently dispatch the next hover or
    /// pointer event to an unrelated node in the new tree (notably,
    /// `handle_hover` would emit `HoverLeave` against a stale-but-
    /// equal key that now points elsewhere). Resetting unconditionally
    /// on doc swap is the safe default; in-flight gestures are torn
    /// down (a pending Tap window is discarded), which matches the
    /// user-visible behaviour that a `.op` edit mid-gesture cancels
    /// what was tracking the old tree.
    ///
    /// `next_id` deliberately keeps counting — recognizer ids are
    /// monotonic and we don't want a new doc's first allocation to
    /// alias any id that any external state may still be holding from
    /// before the swap.
    pub fn reset(&mut self) {
        self.arenas.clear();
        self.raw_roots.clear();
        self.context_menu_pids.clear();
        self.last_hover_target = None;
        self.multi.clear();
        self.shared.clear();
        self.multi_instances.clear();
        self.pending_tap = None;
    }

    /// Public, source-compatible dispatch: feeds the pointer event through
    /// the (envelope) pipeline and returns the semantic events.
    pub fn dispatch(
        &mut self,
        event: PointerEvent,
        doc: &RuntimeDocument,
        spatial: &SpatialIndex,
    ) -> Vec<SemanticEvent> {
        self.dispatch_enveloped(event, doc, spatial)
            .into_iter()
            .map(|envelope| envelope.event)
            .collect()
    }

    /// Envelope-returning dispatch used by the runtime. Recognizers attach
    /// factual pointer/gesture metadata here; nothing downstream
    /// reconstructs it. Uses the static (no state) disabled predicate;
    /// the runtime pointer path calls [`Self::dispatch_enveloped_with`].
    /// A due pending Tap is flushed BEFORE Hover/current semantics.
    pub fn dispatch_enveloped(
        &mut self,
        event: PointerEvent,
        doc: &RuntimeDocument,
        spatial: &SpatialIndex,
    ) -> Vec<SemanticEventEnvelope> {
        self.dispatch_enveloped_with(event, doc, spatial, &|_| false)
    }

    /// Envelope-returning dispatch with a state-aware `gestures.disabled`
    /// predicate. Every arbitration/config decision (recognizer
    /// installation, DoubleTap owner/deferral, ContextMenu owner,
    /// Scale/Rotate handler detection) consults it.
    ///
    /// Static/public path: a due pending Tap is flushed BEFORE both Hover
    /// semantics and the current event are processed — a deferred Tap must
    /// never observe the current event's side effects, and input at the
    /// exact deadline must never pair into a DoubleTap. The runtime pointer
    /// path uses [`Self::dispatch_current`] after its own flush + due
    /// delivery, so the Tap action runs before any current slider,
    /// disabled-predicate, hover or arena decision (and the Router never
    /// flushes the same pending Tap twice).
    pub(crate) fn dispatch_enveloped_with(
        &mut self,
        event: PointerEvent,
        doc: &RuntimeDocument,
        spatial: &SpatialIndex,
        node_disabled: &dyn Fn(NodeKey) -> bool,
    ) -> Vec<SemanticEventEnvelope> {
        let mut due = self.flush_pending_tap(event.t_ms);
        let mut current = self.dispatch_current(event, doc, spatial, node_disabled);
        due.append(&mut current);
        due
    }

    /// Internal current-event path WITHOUT any pending-Tap flush — the
    /// caller owns the flush: the runtime flushes + delivers due actions
    /// BEFORE calling this, and the public entry points flush above, so a
    /// due Tap is never collected twice and never reorders behind the
    /// current semantics. Hover is handled here too: because the caller
    /// flushed first, hover delivery always follows a due Tap.
    pub(crate) fn dispatch_current(
        &mut self,
        event: PointerEvent,
        doc: &RuntimeDocument,
        spatial: &SpatialIndex,
        node_disabled: &dyn Fn(NodeKey) -> bool,
    ) -> Vec<SemanticEventEnvelope> {
        // Hover handled separately: no arena, no claiming. Touch never
        // participates — see `handle_hover`.
        if matches!(event.phase, PointerPhase::Hover) {
            return self.handle_hover(&event, doc, spatial);
        }

        let pid = event.id.0;
        let mut out = Vec::new();

        // A pointer whose Down already resolved as a ContextMenu press (or
        // a right-only press with no context-menu handler, which is closed
        // without side effects): the sequence is closed. No Tap, no
        // PressEnd, no arena.
        if self.context_menu_pids.contains_key(&pid) {
            if matches!(event.phase, PointerPhase::Up | PointerPhase::Cancel) {
                self.context_menu_pids.remove(&pid);
            }
            return out;
        }

        if matches!(event.phase, PointerPhase::Down) {
            let path = hit_test(spatial, doc, event.position);
            if let Some(root) = find_raw_root(&path, doc) {
                self.raw_roots.insert(pid, root);
            } else if let Some(top) = path.topmost() {
                if Self::is_factual_right_press(&event) {
                    // Provable right-button press (exactly RIGHT). A closed
                    // sequence: when the chain declares an enabled
                    // `onContextMenu` it produces exactly ContextMenu;
                    // otherwise it is swallowed entirely — a right-button
                    // press never synthesizes Tap/Press/drag side effects.
                    self.context_menu_pids.insert(pid, ());
                    if config::chain_declares_enabled_with(doc, top, "onContextMenu", node_disabled)
                    {
                        out.push(SemanticEventEnvelope {
                            event: SemanticEvent::ContextMenu {
                                node: top,
                                position: event.position,
                            },
                            pointer_facts: Some(PointerFacts::from_event(&event)),
                            gesture: Default::default(),
                        });
                    }
                } else {
                    let arena = self.build_arena(&path, doc, event.kind, node_disabled);
                    self.arenas.insert(pid, arena);
                    // Multi-pointer recognizer registration (Plan 5 §B.2).
                    // Walk the hit path; for every node that declares an
                    // enabled `events.onScale*` / `events.onRotate*`, attach
                    // this pointer to the (possibly-new) recognizer instance.
                    self.register_multi_pointers(&path, doc, pid, node_disabled);
                }
            }
        }

        if let Some(&root) = self.raw_roots.get(&pid) {
            out.push(SemanticEventEnvelope {
                event: SemanticEvent::RawPointer {
                    node: root,
                    phase: event.phase,
                    position: event.position,
                },
                pointer_facts: Some(PointerFacts::from_event(&event)),
                gesture: Default::default(),
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
                out.extend(arena.drain_envelopes());
            }
        }

        // Handler-aware Tap/DoubleTap deferral: only a chain declaring
        // an enabled `onDoubleTap` buffers; Tap-only chains dispatch
        // immediately. Applies ONLY to taps produced by the current
        // event — a due pending tap was flushed by the caller already.
        apply_tap_deferral(&mut self.pending_tap, &mut out, doc, node_disabled);

        if matches!(event.phase, PointerPhase::Up | PointerPhase::Cancel) {
            self.arenas.remove(&pid);
            self.raw_roots.remove(&pid);
            self.context_menu_pids.remove(&pid);
            self.unregister_multi_pointer(pid);
        }

        out
    }

    /// Factual right-button press: a Mouse/Pen Down whose button bitmask is
    /// EXACTLY RIGHT. LEFT|RIGHT (or any multi-button Down) is ambiguous —
    /// no context is provable, so it is neither a ContextMenu press nor a
    /// closed sequence.
    fn is_factual_right_press(event: &PointerEvent) -> bool {
        matches!(event.kind, PointerKind::Mouse | PointerKind::Pen)
            && event.buttons == MouseButtons::RIGHT
    }

    /// Walk `path` from topmost to root; for each node that declares an
    /// enabled `events.onScale*` / `events.onRotate*` handler (empty/
    /// null lists, `disabledEvents` and state-disabled declarations do not
    /// count), find the existing recognizer instance for that (node, kind)
    /// pair (or allocate one) and append `pid` to its participant list.
    fn register_multi_pointers(
        &mut self,
        path: &HitPath,
        doc: &RuntimeDocument,
        pid: u32,
        node_disabled: &dyn Fn(NodeKey) -> bool,
    ) {
        for &node in &path.0 {
            let handlers = handler_kinds(doc, node, node_disabled);
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
    ///
    /// Cancellations (an active Press emits `PressCancel`) are pushed
    /// BEFORE the multi recognizer's claim event, so the stream is
    /// `[PressCancel, ScaleStart]` — cancel first, then the winner's
    /// semantic event.
    fn dispatch_multi(&mut self, event: &PointerEvent, out: &mut Vec<SemanticEventEnvelope>) {
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
            // Snapshot the pre-state so we can detect the
            // Possible→Claimed transition (vs. already-Claimed
            // sending Update events). Without this, the too_late
            // arbitration would re-fire on every Update, where it
            // *always* sees per-pointer arenas as resolved (they
            // were cancelled at the original claim) and would reject
            // a perfectly-valid in-flight gesture.
            let prev_state = recog.state();
            let mut pending = None;
            let mut handle = ArenaHandle {
                pending_semantic: &mut pending,
            };
            let new_state = recog.handle_pointer(event, &mut handle);
            // Re-borrow-free: clone the participant list before we
            // mutate per-pointer arenas.
            let participants: Vec<u32> = self.shared.get(&rid).cloned().unwrap_or_default();
            let claim_transition = matches!(new_state, RecognizerState::Claimed)
                && !matches!(prev_state, RecognizerState::Claimed);
            if claim_transition {
                // Plan 5 §B.2: any already-resolved arena means the
                // multi claim is too late. Reject the recognizer and
                // SUPPRESS the pending Start event — without this, an
                // observer would see ScaleStart / RotateStart for a
                // gesture that immediately rejected, with no matching
                // End. Codex round 26 Q1.
                let too_late = participants
                    .iter()
                    .any(|p| self.arenas.get(p).map(Arena::is_resolved).unwrap_or(false));
                if too_late {
                    let mut none = None;
                    let mut reject_handle = ArenaHandle {
                        pending_semantic: &mut none,
                    };
                    self.multi
                        .get_mut(&rid)
                        .unwrap()
                        .reject_with_handle(&mut reject_handle);
                    continue;
                }
                // Cancel each unresolved per-pointer arena: the multi
                // gesture wins, single-pointer Tap / Pan / LongPress /
                // Press on these pointers lose. Collect the
                // cancellations so they precede the claim event. The
                // TRIGGERING pointer's arena is witness-fed the current
                // event first (factual metadata only — no recognizer
                // may claim off it), so its PressCancel carries the
                // current Move; other participants keep their latest
                // factual event.
                let mut cancels = Vec::new();
                for p in &participants {
                    if let Some(arena) = self.arenas.get_mut(p) {
                        if !arena.is_resolved() {
                            if *p == pid {
                                arena.witness_press(event);
                            }
                            cancels.extend(arena.cancel_all());
                        }
                    }
                }
                out.extend(cancels);
            }
            // Emit AFTER the too_late check so a rejected claim
            // doesn't leak its Start payload onto the wire, and after
            // the cancellations so PressCancel precedes the winner.
            if let Some(ev) = pending {
                out.push(ev);
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

    fn build_arena(
        &mut self,
        path: &HitPath,
        doc: &RuntimeDocument,
        kind: PointerKind,
        node_disabled: &dyn Fn(NodeKey) -> bool,
    ) -> Arena {
        let mut members: Vec<Box<dyn Recognizer>> = Vec::with_capacity(4);
        let Some(top) = path.topmost() else {
            return Arena::new(members);
        };
        // Press witness: ONE per pointer (a press session is one gesture),
        // targeted at the Down's captured hit. Installed only when the
        // chain declares an enabled press handler — unnamed presses stay
        // silent.
        let press = ["onPressStart", "onPressEnd", "onPressCancel"]
            .iter()
            .any(|handler| config::chain_declares_enabled_with(doc, top, handler, node_disabled));
        if press {
            members.push(Box::new(PressRecognizer::new(self.alloc_id(), top)));
        }

        // At most ONE Tap per pointer path, targeted at the topmost hit:
        // a handler elsewhere on the chain is reached by bubbling (and
        // the built-in widget activation runs on the hit node), so
        // per-ancestor Tap recognizers only shadow each other.
        members.push(Box::new(TapRecognizer::new(self.alloc_id(), top)));

        // Handler/owner-aware Pan: the single Pan recognizer targets the
        // NEAREST node on the hit chain owning any enabled pan handler
        // (onPanStart/onPanUpdate/onPanEnd — nonempty, not
        // `disabledEvents`-listed, `gestures.disabled` not truthy), with
        // THAT owner's authored `dragThreshold` — a nearer child owning
        // only onPanUpdate must win over a farther ancestor owning
        // onPanStart, so the child's threshold governs and its node is
        // the semantic target (delivery bubbles the phases). With no
        // enabled pan handler, the legacy eager semantic recognizer is
        // installed at the topmost hit with default thresholds (its
        // semantic is dropped by the dispatcher).
        let pan_owner = config::chain_pan_owner_with(doc, top, node_disabled);
        let pan_node = pan_owner.unwrap_or(top);
        let pan_cfg = config::gesture_config(doc, pan_node);
        members.push(Box::new(
            PanRecognizer::new(self.alloc_id(), pan_node)
                .with_threshold(pan_cfg.effective_drag_threshold()),
        ));

        // Owner-aware LongPress / ContextMenu fallback (at most one):
        // an explicit enabled onLongPress wins; otherwise a touch
        // long-press on a chain declaring enabled onContextMenu falls
        // back to ContextMenu at the same deadline (never both). Any
        // other chain keeps the legacy unconditional LongPress
        // recognizer at the topmost hit — the semantic fires and the
        // dispatcher drops it when no handler exists, preserving the
        // pre-existing semantic stream for hosts that inspect it.
        let long_owner = config::chain_owner_with(doc, top, "onLongPress", node_disabled);
        let context_owner = config::chain_owner_with(doc, top, "onContextMenu", node_disabled);
        let (lp_node, lp_mode) = if let Some(owner) = long_owner {
            (owner, LongPressMode::LongPress)
        } else if let Some(owner) = context_owner {
            if matches!(kind, PointerKind::Touch) {
                (owner, LongPressMode::ContextMenu)
            } else {
                (top, LongPressMode::LongPress)
            }
        } else {
            (top, LongPressMode::LongPress)
        };
        let lp_cfg = config::gesture_config(doc, lp_node);
        let lp = match lp_mode {
            LongPressMode::LongPress => LongPressRecognizer::new(self.alloc_id(), lp_node),
            LongPressMode::ContextMenu => {
                LongPressRecognizer::for_context_menu(self.alloc_id(), lp_node)
            }
        };
        members.push(Box::new(
            lp.with_duration(lp_cfg.effective_long_press_duration()),
        ));
        Arena::new(members)
    }

    /// Public, source-compatible tick: drive timer-based recognizers and
    /// flush a due pending Tap.
    pub fn tick(&mut self, now_ms: u64) -> Vec<SemanticEvent> {
        self.tick_enveloped(now_ms)
            .into_iter()
            .map(|envelope| envelope.event)
            .collect()
    }

    /// Envelope-returning tick. A buffered Tap whose deadline `now_ms` has
    /// passed is emitted exactly once here, even with no new input.
    pub fn tick_enveloped(&mut self, now_ms: u64) -> Vec<SemanticEventEnvelope> {
        let mut out = Vec::new();
        for arena in self.arenas.values_mut() {
            arena.tick(now_ms);
            out.extend(arena.drain_envelopes());
        }
        out.extend(self.flush_pending_tap(now_ms));
        out
    }

    /// Flush a due pending Tap WITHOUT driving arena timers. Used while
    /// input is frozen (parked variant swap): timer-driven recognizers do
    /// not claim inside the freeze, but a deferred Tap whose deadline
    /// passed must not be consumed without delivery.
    pub fn flush_pending_tap(&mut self, now_ms: u64) -> Vec<SemanticEventEnvelope> {
        super::router_tap::flush_pending_tap(&mut self.pending_tap, now_ms)
    }

    pub fn next_wake_ms(&self) -> Option<u64> {
        let arena_wake = self.arenas.values().filter_map(Arena::next_wake_ms).min();
        let tap_wake = self.pending_tap.as_ref().map(|p| p.deadline_ms);
        match (arena_wake, tap_wake) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn handle_hover(
        &mut self,
        event: &PointerEvent,
        doc: &RuntimeDocument,
        spatial: &SpatialIndex,
    ) -> Vec<SemanticEventEnvelope> {
        // Touch must never emit hover actions or mutate the hover cache —
        // a touch host that (incorrectly) emits a Hover phase is ignored
        // so a later real mouse Enter is not turned into a Leave/Enter
        // pair against a poisoned target.
        if matches!(event.kind, PointerKind::Touch) {
            return Vec::new();
        }
        let path = hit_test(spatial, doc, event.position);
        let target = path.topmost();
        let mut out = Vec::new();
        if target != self.last_hover_target {
            if let Some(prev) = self.last_hover_target {
                out.push(SemanticEventEnvelope {
                    event: SemanticEvent::HoverLeave {
                        node: prev,
                        position: event.position,
                    },
                    pointer_facts: Some(PointerFacts::from_event(event)),
                    gesture: Default::default(),
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
}

/// Which LongPress semantic a chain's recognizers emit (computed once per
/// hit path in `build_arena`).
#[derive(Clone, Copy)]
enum LongPressMode {
    LongPress,
    ContextMenu,
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
/// recognizer kinds it declares ACTIVE handlers for (a non-empty list
/// that is not `disabledEvents`-listed and whose `gestures.disabled`
/// expression is not truthy). Round-trip via JSON for parity with
/// `dispatch_event`'s `extract_handler` — the schema types are
/// per-variant so direct field access would need a match arm per
/// `PenNode` variant. Scale/Rotate arbitration is untouched; only
/// handler DETECTION honors the disabled declarations.
fn handler_kinds(
    doc: &RuntimeDocument,
    key: NodeKey,
    node_disabled: &dyn Fn(NodeKey) -> bool,
) -> HandlerKinds {
    let scale = ["onScaleStart", "onScaleUpdate", "onScaleEnd"]
        .iter()
        .any(|handler| {
            config::node_declares_handler(doc, key, handler)
                && !config::node_disables_handler(doc, key, handler)
                && !node_disabled(key)
        });
    let rotate = ["onRotateStart", "onRotateUpdate", "onRotateEnd"]
        .iter()
        .any(|handler| {
            config::node_declares_handler(doc, key, handler)
                && !config::node_disables_handler(doc, key, handler)
                && !node_disabled(key)
        });
    HandlerKinds { scale, rotate }
}
