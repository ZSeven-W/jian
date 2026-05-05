//! Runtime — the composition root.
//!
//! Typical startup:
//! ```ignore
//! let mut rt = Runtime::new();
//! rt.load_str(&src)?;
//! rt.build_layout((800.0, 600.0))?;
//! rt.rebuild_spatial();
//! ```
//!
//! Pointer input is driven by the host, which calls
//! `rt.dispatch_pointer(event)` and, each frame, `rt.tick(now)`.

use crate::action::services::{
    AsyncFeedback, ClipboardService, FeedbackSink, NetworkClient, NullClipboard, NullFeedback,
    NullNetworkClient, NullRouter, NullStorageBackend, Router as RouterSvc, StorageBackend,
};
use crate::action::{
    default_registry, ActionContext, CancellationToken, ExecOutcome, SharedRegistry,
};
use crate::binding::{BindingEffect, DeferredBindingQueue};
use crate::capability::{
    from_schema_capability, AuditLog, CapabilityGate, DeclaredCapabilityGate, DummyCapabilityGate,
    NullPermissionBroker, PermissionBroker,
};
use crate::document::{loader, RuntimeDocument};
use crate::effect::EffectRegistry;
use crate::error::CoreResult;
use crate::expression::ExpressionCache;
use crate::geometry::size;
use crate::gesture::{
    collect_focus_chain, dispatch_event, FocusManager, PointerEvent, PointerRouter, SemanticEvent,
};
use crate::layout::LayoutEngine;
use crate::scene::SceneGraph;
use crate::signal::scheduler::Scheduler;
use crate::spatial::{NodeBBox, SpatialIndex};
use crate::state::StateGraph;
use crate::viewport::Viewport;
use jian_ops_schema::{document::PenDocument, load_str};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

/// Default audit-log size. 1000 entries is generous for in-session
/// inspection without letting long-lived hosts grow unboundedly.
const AUDIT_LOG_CAPACITY: usize = 1000;

pub struct Runtime {
    pub scheduler: Rc<Scheduler>,
    pub effects: Rc<EffectRegistry>,
    pub state: Rc<StateGraph>,
    pub document: Option<RuntimeDocument>,
    pub layout: LayoutEngine,
    pub spatial: SpatialIndex,
    pub viewport: Viewport,
    pub scene: SceneGraph,

    // --- Gesture + Action wiring (Plan 5 T15) ---
    pub gestures: PointerRouter,
    /// Tab-tree focus state. Rebuilt on every document swap from the
    /// runtime tree. See [`Runtime::dispatch_keyboard`] /
    /// [`Runtime::focus_next`] / [`Runtime::focus_request`].
    pub focus: FocusManager,
    pub actions: SharedRegistry,
    pub expr_cache: Rc<ExpressionCache>,
    /// Bindings whose evaluation is deferred past first-paint. Schema
    /// load pushes off-viewport bindings here; the host drains them via
    /// [`Runtime::drain_deferred_bindings`] inside (or after) the
    /// `EventPumpReady` startup phase. See `binding::DeferredBindingQueue`.
    pub deferred_bindings: DeferredBindingQueue,
    pub network: Rc<dyn NetworkClient>,
    /// Live WebSocket sessions, populated by `ws_connect` / drained by
    /// `ws_close`. Shared with every `ActionContext` the runtime makes.
    pub ws_sessions: crate::action::context::WsSessionRegistry,
    pub storage: Rc<dyn StorageBackend>,
    pub nav: Rc<dyn RouterSvc>,
    pub feedback: Rc<dyn FeedbackSink>,
    pub async_feedback: Rc<dyn AsyncFeedback>,
    pub clipboard: Rc<dyn ClipboardService>,
    pub capabilities: Rc<dyn CapabilityGate>,
    /// Audit log attached to the capability gate. `None` for the default
    /// `Runtime::new()` (DummyCapabilityGate has nothing to audit); set
    /// when the runtime is built via `new_from_document`.
    pub audit: Option<Rc<AuditLog>>,
    pub permissions: Rc<dyn PermissionBroker>,
    /// Tier-3 logic provider — how `call` actions dispatch. Null by
    /// default; hosts override with `set_logic_provider`.
    pub logic: Rc<dyn crate::logic::LogicProvider>,
}

impl Runtime {
    pub fn new() -> Self {
        let scheduler = Rc::new(Scheduler::new());
        let effects = EffectRegistry::new();
        effects.install_on(&scheduler);
        Self {
            state: Rc::new(StateGraph::new(scheduler.clone())),
            scheduler,
            effects,
            document: None,
            layout: LayoutEngine::new(),
            spatial: SpatialIndex::new(),
            viewport: Viewport::new(size(800.0, 600.0)),
            scene: SceneGraph::new(),

            gestures: PointerRouter::new(),
            focus: FocusManager::new(),
            actions: default_registry(),
            expr_cache: Rc::new(ExpressionCache::new()),
            deferred_bindings: DeferredBindingQueue::new(),
            network: Rc::new(NullNetworkClient),
            ws_sessions: Rc::new(RefCell::new(std::collections::HashMap::new())),
            storage: Rc::new(NullStorageBackend),
            nav: Rc::new(NullRouter),
            feedback: Rc::new(NullFeedback),
            async_feedback: Rc::new(NullFeedback),
            clipboard: Rc::new(NullClipboard),
            capabilities: Rc::new(DummyCapabilityGate),
            audit: None,
            permissions: Rc::new(NullPermissionBroker),
            logic: Rc::new(crate::logic::NullLogicProvider),
        }
    }

    /// Install a Tier-3 `LogicProvider`. Replaces the default
    /// `NullLogicProvider` and takes effect for every subsequent
    /// `call` action dispatch (the cached `ActionContext` is rebuilt
    /// per action chain, so no cache invalidation is needed).
    pub fn set_logic_provider(&mut self, provider: Rc<dyn crate::logic::LogicProvider>) {
        self.logic = provider;
    }

    /// Build a runtime whose `CapabilityGate` is derived from the
    /// document's `app.capabilities` declaration. Checks are recorded in
    /// an `AuditLog` attached to `self.audit`.
    ///
    /// An undeclared `app.capabilities` field means "no capabilities" —
    /// every IO action will be denied. Ship the `.op` with an explicit
    /// declaration to unlock network/storage/etc.
    pub fn new_from_document(schema: PenDocument) -> CoreResult<Self> {
        let scheduler = Rc::new(Scheduler::new());
        let effects = EffectRegistry::new();
        effects.install_on(&scheduler);

        let audit = Rc::new(AuditLog::new(AUDIT_LOG_CAPACITY));
        let declared = schema
            .app
            .as_ref()
            .and_then(|a| a.capabilities.as_ref())
            .map(|list| {
                list.iter()
                    .copied()
                    .map(from_schema_capability)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let gate = Rc::new(DeclaredCapabilityGate::new(declared, Some(audit.clone())));

        let state = Rc::new(StateGraph::new(scheduler.clone()));
        let doc = loader::build(schema, &state)?;
        let focus_chain = collect_focus_chain(&doc);
        let mut focus = FocusManager::new();
        focus.set_chain(focus_chain);

        Ok(Self {
            state,
            scheduler,
            effects,
            document: Some(doc),
            layout: LayoutEngine::new(),
            spatial: SpatialIndex::new(),
            viewport: Viewport::new(size(800.0, 600.0)),
            scene: SceneGraph::new(),

            gestures: PointerRouter::new(),
            focus,
            actions: default_registry(),
            expr_cache: Rc::new(ExpressionCache::new()),
            deferred_bindings: DeferredBindingQueue::new(),
            network: Rc::new(NullNetworkClient),
            ws_sessions: Rc::new(RefCell::new(std::collections::HashMap::new())),
            storage: Rc::new(NullStorageBackend),
            nav: Rc::new(NullRouter),
            feedback: Rc::new(NullFeedback),
            async_feedback: Rc::new(NullFeedback),
            clipboard: Rc::new(NullClipboard),
            capabilities: gate,
            audit: Some(audit),
            permissions: Rc::new(NullPermissionBroker),
            logic: Rc::new(crate::logic::NullLogicProvider),
        })
    }

    pub fn load_str(&mut self, src: &str) -> CoreResult<()> {
        let schema = load_str(src)?.value;
        self.replace_document(schema)
    }

    /// Swap the runtime's document tree for `schema`, reusing the
    /// existing StateGraph + services. Used by `jian dev` hot-reload
    /// so app state (e.g. `$state.count`) survives a `.op` edit.
    ///
    /// Refreshes the capability gate from the new schema's
    /// `app.capabilities` (additions become available immediately,
    /// removals start denying), and reuses an existing `AuditLog` so
    /// rolling history is preserved across reloads.
    ///
    /// State seeding uses `SeedMode::PreserveExisting` — keys that
    /// already hold a value keep that value; only newly-introduced
    /// keys get their schema default.
    pub fn replace_document(&mut self, schema: PenDocument) -> CoreResult<()> {
        // Rebuild the capability gate from the new schema. Reuse the
        // existing AuditLog so the rolling history isn't truncated on
        // every save. If the original Runtime was constructed via
        // `Runtime::new` (no audit), allocate one now so newly
        // declared capabilities can record entries.
        let declared = schema
            .app
            .as_ref()
            .and_then(|a| a.capabilities.as_ref())
            .map(|list| {
                list.iter()
                    .copied()
                    .map(from_schema_capability)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let audit = self
            .audit
            .clone()
            .unwrap_or_else(|| Rc::new(AuditLog::new(AUDIT_LOG_CAPACITY)));
        self.audit = Some(audit.clone());
        self.capabilities = Rc::new(DeclaredCapabilityGate::new(declared, Some(audit)));

        let doc = loader::build_with(schema, &self.state, loader::SeedMode::PreserveExisting)?;
        let focus_chain = collect_focus_chain(&doc);
        self.document = Some(doc);
        // Hot-reload swaps the SlotMap underneath. SlotMap keys are
        // *not* unique across different SlotMaps — both the old and
        // new tree start their version counter at 1, so the first
        // insert into each map yields equal keys. Any cached
        // `NodeKey` from the pre-swap tree could silently dispatch
        // the next event to an unrelated new node, so blow away
        // every gesture-pipeline cache that holds one:
        //
        // - `focus.current` — cleared first (`set_chain` alone can't
        //   tell stale-but-equal apart from "really still in the
        //   chain"). Authors who want focus preserved across reload
        //   re-issue `focus_request` post-swap from
        //   `lifecycle.on_load`.
        // - `gestures` (PointerRouter): `raw_roots`,
        //   `last_hover_target`, `last_tap`, `multi_instances` —
        //   reset wholesale; in-flight pointer / hover sequences
        //   are torn down on hot-reload. Without this, the next
        //   hover after a `.op` edit could fire `HoverLeave`
        //   against a stale-but-equal key that now points to a
        //   different node in the new tree.
        self.focus.clear();
        self.focus.set_chain(focus_chain);
        self.gestures.reset();
        // Plan 19 D1 codex round 2 MEDIUM: a stale preload from a
        // prior `.op.pack` load survives the doc swap and `node_rect`
        // would serve rects keyed against the OLD slot keys whenever
        // the new tree happens to fill matching SecondaryMap slots.
        // Drop the cache unconditionally — hosts that hot-reload to
        // a doc with a fresh `.op.pack` re-call `preload_initial_layout`
        // explicitly.
        self.layout.drop_preload();
        Ok(())
    }

    pub fn build_layout(&mut self, available: (f32, f32)) -> CoreResult<()> {
        let doc = self.document.as_ref().expect("no document loaded");
        let roots = self.layout.build(&doc.tree)?;
        for root in roots {
            self.layout.compute(root, available)?;
        }
        Ok(())
    }

    /// Plan 19 D1 cold-start fast path: feed the runtime a pre-computed
    /// `aot/initial_layout.bin` snapshot so the first paint can skip
    /// `ComputeFirstLayout`. Returns the number of rects resolved
    /// against the active document — `0` if no document is loaded yet
    /// (the snapshot is silently ignored, mirroring `replace_document`'s
    /// "no panic on stale data" contract).
    ///
    /// The host's startup driver typically calls this from inside (or
    /// just after) `SeedStateGraph` when the bootstrap source carries
    /// a `.op.pack` whose manifest declares
    /// [`jian_ops_schema::pack::ENTRY_AOT_INITIAL_LAYOUT`], then
    /// short-circuits the registered `ComputeFirstLayout` phase to a
    /// no-op so the snapshot's rects survive into `BuildVisibleSpatial`.
    pub fn preload_initial_layout(
        &mut self,
        snapshot: &jian_ops_schema::pack::initial_layout::InitialLayoutSnapshot,
    ) -> usize {
        let Some(doc) = self.document.as_ref() else {
            return 0;
        };
        self.layout.preload_initial(snapshot, &doc.tree)
    }

    /// Variant of [`Self::build_layout`] that swaps the layout
    /// engine's measure backend before laying out. Hosts that wire
    /// real shaping (e.g. jian-skia's `SkiaMeasure` under
    /// `textlayout`) install their backend once via this entry point;
    /// subsequent `build_layout` calls reuse the same backend until
    /// it's swapped again. Default-feature builds and unit tests
    /// stay on the in-core `EstimateBackend`.
    ///
    /// The swap mutates the existing layout engine in place
    /// (preserving any cached taffy state); the next layout pass
    /// rebuilds the tree from `doc.tree` as usual, but the engine's
    /// backend slot is now the host-supplied one.
    pub fn build_layout_with(
        &mut self,
        measure: Rc<dyn crate::layout::measure::MeasureBackend>,
        available: (f32, f32),
    ) -> CoreResult<()> {
        self.layout.set_backend(measure);
        self.build_layout(available)
    }

    pub fn rebuild_spatial(&mut self) {
        let doc = self.document.as_ref().expect("no document loaded");
        let items: Vec<NodeBBox> = doc
            .tree
            .nodes
            .iter()
            .filter_map(|(key, _)| {
                self.layout
                    .node_rect(key)
                    .map(|rect| NodeBBox { key, rect })
            })
            .collect();
        self.spatial.rebuild(items);
    }

    /// Cold-start variant (Plan 19 Task 5): bulk-load only nodes
    /// whose bbox intersects the supplied viewport. The off-
    /// viewport remainder is returned so the host can fold it in
    /// later via [`SpatialIndex::fill_rest`] once the
    /// `EventPumpReady` startup phase has fired.
    ///
    /// On a 1000-node document with ~100 visible nodes, this
    /// drops the first spatial build from O(1000 log 1000) to
    /// O(100 log 100) — the C19 measurement target.
    pub fn rebuild_spatial_for_first_frame(
        &mut self,
        viewport: crate::geometry::Rect,
    ) -> Vec<NodeBBox> {
        let doc = self.document.as_ref().expect("no document loaded");
        let mut visible: Vec<NodeBBox> = Vec::new();
        let mut hidden: Vec<NodeBBox> = Vec::new();
        for (key, _) in doc.tree.nodes.iter() {
            let Some(rect) = self.layout.node_rect(key) else {
                continue;
            };
            let bbox = NodeBBox { key, rect };
            if rects_intersect(rect, viewport) {
                visible.push(bbox);
            } else {
                hidden.push(bbox);
            }
        }
        self.spatial.rebuild(visible);
        hidden
    }

    /// Feed a pointer event through the gesture pipeline; any emitted
    /// semantic events are routed to the matching `events.*` handlers.
    /// Returns the semantic events for host inspection/tests.
    pub fn dispatch_pointer(&mut self, event: PointerEvent) -> Vec<SemanticEvent> {
        let doc = match self.document.as_ref() {
            Some(d) => d,
            None => return Vec::new(),
        };
        let emitted = self.gestures.dispatch(event, doc, &self.spatial);
        for ev in &emitted {
            self.dispatch_semantic(ev);
        }
        emitted
    }

    /// Route a wheel event to whatever node the cursor is over and
    /// emit `SemanticEvent::Scroll` for the topmost node carrying an
    /// `events.onScroll` handler. Wheel doesn't compete in the gesture
    /// arena (no Tap/Pan rivalry), so we use `hit_test` directly to
    /// get the z-ordered path (deepest first, then bubble up
    /// ancestors). Returns the emitted events for host inspection /
    /// tests.
    pub fn dispatch_wheel(
        &mut self,
        event: crate::gesture::pointer::WheelEvent,
    ) -> Vec<SemanticEvent> {
        let Some(doc) = self.document.as_ref() else {
            return Vec::new();
        };
        let mut emitted = Vec::new();
        // `hit_test` returns the deepest-first hit path including all
        // ancestors, so a wheel that lands on a child without a
        // handler still bubbles up to a parent scroll container.
        let path = crate::gesture::hit::hit_test(&self.spatial, doc, event.position);
        for key in path.0.iter().copied() {
            let schema = &doc.tree.nodes[key].schema;
            if json_has_event_handler(schema, "onScroll") {
                emitted.push(SemanticEvent::Scroll {
                    node: key,
                    delta: event.delta,
                });
                break;
            }
        }
        for ev in &emitted {
            self.dispatch_semantic(ev);
        }
        emitted
    }

    /// Dispatch a key event to the *named* node — bubbles up the
    /// parent chain like `dispatch_event` so a key handler on a
    /// container can claim a child's keystroke. Returns the semantic
    /// events emitted (one `KeyDown` per call) for host inspection
    /// and tests. Hosts wire this to OS-level keyboard input;
    /// `jian-action-surface` synthesises Enter / Escape KeyDowns
    /// when an AI client invokes `confirm_<slug>` / `dismiss_<slug>`.
    pub fn dispatch_key(
        &mut self,
        target: crate::document::NodeKey,
        key: impl Into<String>,
        modifiers: crate::gesture::pointer::Modifiers,
    ) -> Vec<SemanticEvent> {
        if self.document.is_none() {
            return Vec::new();
        }
        let event = SemanticEvent::KeyDown {
            node: target,
            key: key.into(),
            modifiers,
        };
        self.dispatch_semantic(&event);
        vec![event]
    }

    /// High-level keyboard entry point that's focus-aware.
    ///
    /// `Tab` (and `Shift+Tab`) drive [`Self::focus_next`] /
    /// [`Self::focus_previous`] — the chain advances and `FocusLost`
    /// (for the previously-focused node, if any) and `FocusGained`
    /// (for the new node) are fired in that order. The Tab key
    /// itself does **not** propagate to the focused node — Tab is a
    /// focus-traversal key, not an authored input event.
    ///
    /// Every other key is forwarded to the currently-focused node
    /// via [`Self::dispatch_key`]. When nothing is focused, the
    /// keystroke is dropped (no host on the stack today fires
    /// untargeted KeyDowns; future scope might add a window-level
    /// handler — at which point this branch is the right one to
    /// extend).
    ///
    /// Returns the emitted semantic events for host inspection /
    /// tests.
    pub fn dispatch_keyboard(
        &mut self,
        key: impl Into<String>,
        modifiers: crate::gesture::pointer::Modifiers,
    ) -> Vec<SemanticEvent> {
        if self.document.is_none() {
            return Vec::new();
        }
        let key = key.into();
        if key == "Tab" {
            if modifiers.contains(crate::gesture::pointer::Modifiers::SHIFT) {
                return self.focus_previous();
            }
            return self.focus_next();
        }
        let Some(target) = self.focus.current() else {
            return Vec::new();
        };
        self.dispatch_key(target, key, modifiers)
    }

    /// Move focus forward one step (`Tab`) and emit the corresponding
    /// `FocusLost` / `FocusGained` events.
    pub fn focus_next(&mut self) -> Vec<SemanticEvent> {
        let change = self.focus.next();
        self.emit_focus_change(change)
    }

    /// Move focus backward one step (`Shift+Tab`).
    pub fn focus_previous(&mut self) -> Vec<SemanticEvent> {
        let change = self.focus.previous();
        self.emit_focus_change(change)
    }

    /// Programmatically focus an explicit node. Hosts call this from
    /// click handlers (focus-on-click) or from `jian-action-surface`
    /// when an AI client requests a focus change.
    pub fn focus_request(&mut self, node: crate::document::NodeKey) -> Vec<SemanticEvent> {
        let change = self.focus.request(node);
        self.emit_focus_change(change)
    }

    /// Drop focus entirely — typically wired to clicking outside any
    /// focusable node, or to the window losing OS focus.
    pub fn focus_clear(&mut self) -> Vec<SemanticEvent> {
        let change = self.focus.clear();
        self.emit_focus_change(change)
    }

    fn emit_focus_change(&mut self, change: crate::gesture::FocusChange) -> Vec<SemanticEvent> {
        if change.is_noop() {
            return Vec::new();
        }
        let mut emitted = Vec::with_capacity(2);
        if let Some(prev) = change.previous {
            let ev = SemanticEvent::FocusLost { node: prev };
            self.dispatch_semantic(&ev);
            emitted.push(ev);
        }
        // Re-entrancy guard. The `FocusLost` dispatch above runs the
        // node's `events.onBlur` ActionList synchronously, which can
        // call back into `Runtime::focus_request` / `focus_clear` /
        // even `dispatch_keyboard("Tab", ...)`. Any such nested call
        // already moved `self.focus.current` and emitted its own
        // `FocusGained` for the new target — surfacing
        // `FocusGained { change.current }` unconditionally would
        // raise the *original* target's `onFocus` after focus has
        // moved on. So we re-read `focus.current()` here and gate:
        // when the nested re-entry took over, the outer event is
        // suppressed; when nothing nested moved focus, the
        // unchanged `current` matches `change.current` and we emit
        // as planned.
        if let Some(next) = change.current {
            if self.focus.current() == Some(next) {
                let ev = SemanticEvent::FocusGained { node: next };
                self.dispatch_semantic(&ev);
                emitted.push(ev);
            }
        }
        emitted
    }

    /// Drain pending WebSocket messages and fire each session's
    /// `on_message` ActionList for every received frame. Hosts call
    /// this every event-loop iteration (right alongside `tick`) so
    /// authored handlers see arrivals at frame cadence.
    ///
    /// Each fired handler runs with `$event = { id, data }` so an
    /// expression like `set: { $state.last_msg: $event.data }`
    /// reads the payload directly. Returns the number of handlers
    /// that fired (per message) so hosts can request a redraw when
    /// state changed.
    pub fn pump_websockets(&mut self) -> usize {
        let snapshot: Vec<(
            String,
            Rc<dyn crate::action::services::WebSocketSession>,
            Option<serde_json::Value>,
        )> = {
            self.ws_sessions
                .borrow()
                .iter()
                .map(|(id, h)| (id.clone(), h.session.clone(), h.on_message.clone()))
                .collect()
        };
        let mut fired = 0usize;
        for (id, session, on_message) in snapshot {
            // Re-check the registry before running each session's
            // handler. A previous handler in this same pump pass may
            // have called `ws_close` (drops the entry) or
            // `ws_connect` with the same id (replaces the entry with
            // a new session). Dispatching against the stale `Rc<...>`
            // would fire on a connection the author already
            // declared closed.
            let still_live = self
                .ws_sessions
                .borrow()
                .get(&id)
                .map(|h| Rc::ptr_eq(&h.session, &session))
                .unwrap_or(false);
            if !still_live {
                continue;
            }
            let messages: Vec<String> = futures::executor::block_on(session.receive());
            if messages.is_empty() {
                continue;
            }
            let Some(handler_json) = on_message else {
                continue;
            };
            for msg in messages {
                let registry = self.actions.clone();
                let parsed = registry.borrow().parse_list(&handler_json);
                let chain = match parsed {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                // The handler we're about to run could itself touch
                // ws_sessions (close + reconnect). Re-verify per
                // message so a mid-burst close stops further
                // dispatch on the same loop.
                let still_live = self
                    .ws_sessions
                    .borrow()
                    .get(&id)
                    .map(|h| Rc::ptr_eq(&h.session, &session))
                    .unwrap_or(false);
                if !still_live {
                    break;
                }
                let ctx = self.make_action_ctx_with_event(serde_json::json!({
                    "id": id,
                    "data": msg,
                }));
                let _ = futures::executor::block_on(chain.run_serial(&ctx));
                self.scheduler.flush();
                fired += 1;
            }
        }
        fired
    }

    /// Build an ActionContext just like `make_action_ctx` but with
    /// `$event` populated from `payload`. Used by `pump_websockets`.
    fn make_action_ctx_with_event(&self, payload: serde_json::Value) -> ActionContext {
        let mut ctx = self.make_action_ctx();
        ctx.event = Some(crate::value::RuntimeValue::from(payload));
        ctx
    }

    /// Drive timer-based recognizers (LongPress). Host must call each frame.
    pub fn tick(&mut self, now: Instant) -> Vec<SemanticEvent> {
        let emitted = self.gestures.tick(now);
        for ev in &emitted {
            self.dispatch_semantic(ev);
        }
        emitted
    }

    fn dispatch_semantic(&self, event: &SemanticEvent) -> ExecOutcome {
        let doc = self.document.as_ref().expect("no document loaded");
        let ctx = match event_payload(event) {
            Some(payload) => self.make_action_ctx_with_event(payload),
            None => self.make_action_ctx(),
        };
        let outcome = dispatch_event(doc, event, &self.actions, &ctx);
        // Actions mutate state via Signals whose effects are scheduled;
        // flush synchronously so bindings / scene observers see the new
        // values before the host's next frame.
        self.scheduler.flush();
        outcome
    }

    /// Drain the deferred-binding queue into registered, reactive
    /// `BindingEffect`s. Returns the freshly-registered effects, which
    /// the caller must keep alive for the runtime's lifetime — dropping
    /// a `BindingEffect` deregisters its underlying effect and breaks
    /// reactivity for that binding.
    ///
    /// Plan 19 calls this from inside (or just after) the
    /// `EventPumpReady` startup phase: schema load enqueues every
    /// off-viewport binding, the critical-path phases finish first paint,
    /// and only then do the deferred bindings get compiled + subscribed
    /// — spreading the cost across post-paint frames.
    #[must_use = "keep the returned BindingEffect handles alive; dropping them \
                  deregisters the drained bindings and silently disables reactivity"]
    pub fn drain_deferred_bindings(&mut self) -> Vec<BindingEffect> {
        self.deferred_bindings.drain_into_effects(
            &self.effects,
            Rc::clone(&self.expr_cache),
            Rc::clone(&self.state),
        )
    }

    /// Pre-compile every queued binding's source into the expression
    /// cache without registering effects. Used by `jian pack --aot`
    /// (Plan 19 D2) so the AOT `expressions.bin` snapshot contains
    /// every expression the runtime will eventually evaluate, not
    /// just the ones an opportunistic first-paint dispatch happens
    /// to fire.
    ///
    /// Returns the count of unique sources compiled. Sources that
    /// fail to parse are silently skipped — `ExpressionCache::
    /// get_or_compile` doesn't insert on error and the runtime will
    /// surface the same diagnostic at evaluation time. Skipping
    /// here keeps pack-time success decoupled from runtime-time
    /// expression diagnostics; an author who ships a doc with a
    /// broken binding still gets a working pack (sans AOT entry for
    /// that one source) and the same error at runtime as without
    /// `--aot`.
    pub fn warm_expression_cache(&self) -> usize {
        let mut compiled = 0usize;
        for source in self.deferred_bindings.sources() {
            if self.expr_cache.get_or_compile(source).is_ok() {
                compiled += 1;
            }
        }
        compiled
    }

    /// Build an `ActionContext` tied to this runtime's services. Exposed
    /// for integration tests and host embedders that want to run a
    /// standalone ActionList outside the gesture pipeline.
    pub fn make_action_ctx(&self) -> ActionContext {
        ActionContext {
            state: self.state.clone(),
            scheduler: self.scheduler.clone(),
            event: None,
            locals: RefCell::new(BTreeMap::new()),
            page_id: None,
            node_id: None,
            network: self.network.clone(),
            ws_sessions: self.ws_sessions.clone(),
            storage: self.storage.clone(),
            router: self.nav.clone(),
            feedback: self.feedback.clone(),
            async_fb: self.async_feedback.clone(),
            clipboard: self.clipboard.clone(),
            capabilities: self.capabilities.clone(),
            logic: self.logic.clone(),
            expr_cache: self.expr_cache.clone(),
            cancel: CancellationToken::new(),
            warnings: RefCell::new(Vec::new()),
        }
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the `$event` payload that an `events.*` handler sees for a
/// given semantic event. Returns `None` for events that don't carry
/// extra metadata beyond their target node — those handlers run with
/// `ctx.event = None` and any `$event` access yields null.
///
/// `KeyDown` exposes `{ key, modifiers }` so authors can branch on
/// `$event.key` (e.g. `"Enter"` vs `"Escape"`) — without this the
/// handler runs but can't tell *which* key fired. `modifiers` is a
/// JSON array of pressed modifier names (`["shift"]`, `["ctrl",
/// "alt"]`, …) drawn from the set `shift` / `ctrl` / `alt` / `cmd`.
/// Authors test membership with the Tier 1 builtin
/// `includes($event.modifiers, "shift")`.
fn event_payload(event: &SemanticEvent) -> Option<serde_json::Value> {
    match event {
        SemanticEvent::KeyDown { key, modifiers, .. } => {
            let mods: Vec<&str> = [
                (crate::gesture::pointer::Modifiers::SHIFT, "shift"),
                (crate::gesture::pointer::Modifiers::CTRL, "ctrl"),
                (crate::gesture::pointer::Modifiers::ALT, "alt"),
                (crate::gesture::pointer::Modifiers::CMD, "cmd"),
            ]
            .iter()
            .filter_map(|(flag, name)| modifiers.contains(*flag).then_some(*name))
            .collect();
            Some(serde_json::json!({
                "key": key,
                "modifiers": mods,
            }))
        }
        // Scale carries the running ratio + focal point. Authors bind
        // `$state.zoom` to `$event.scale` and read `$event.focal.x` /
        // `$event.focal.y` to compute pinch-around behaviour. Start
        // omits `scale` (always 1.0 at activation); Update carries it;
        // End is a bare signal.
        SemanticEvent::ScaleStart { focal, .. } => Some(serde_json::json!({
            "focal": { "x": focal.x, "y": focal.y },
        })),
        SemanticEvent::ScaleUpdate { scale, focal, .. } => Some(serde_json::json!({
            "scale": *scale,
            "focal": { "x": focal.x, "y": focal.y },
        })),
        // Rotate carries running radians. Authors typically bind
        // `$state.rotation` directly to `$event.radians`. Start omits
        // it (always 0 at activation).
        SemanticEvent::RotateUpdate { radians, .. } => Some(serde_json::json!({
            "radians": *radians,
        })),
        _ => None,
    }
}

/// Does the node's schema carry a non-empty `events.<key>` ActionList?
/// Round-trips through serde_json::Value so the same code handles all
/// 11 PenNode variants without per-variant matches — same trick the
/// scene walker and `extract_handler` use.
///
/// Spec §3.2 says rules trigger on "events.X 非空" — an empty array
/// `[]` therefore doesn't count, otherwise a parent with a real
/// onScroll handler would be silently shadowed by an empty stub on
/// the deepest hit.
fn json_has_event_handler(node: &jian_ops_schema::node::PenNode, key: &str) -> bool {
    use serde_json::Value;
    let v = match serde_json::to_value(node) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let handler = v
        .as_object()
        .and_then(|obj| obj.get("events"))
        .and_then(|events| events.as_object())
        .and_then(|map| map.get(key));
    match handler {
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Null) | None => false,
        // Object / scalar handler — not strictly an ActionList but
        // treat as present so authored shorthand still routes.
        Some(_) => true,
    }
}

/// AABB-vs-AABB overlap test for the visible-set pre-filter on
/// [`Runtime::rebuild_spatial_for_first_frame`]. Two rects
/// intersect when their projected ranges overlap on both axes;
/// rects sharing only an edge (`a.max == b.min`) are treated as
/// intersecting so a node flush against the viewport's right
/// edge is still hit-testable.
fn rects_intersect(a: crate::geometry::Rect, b: crate::geometry::Rect) -> bool {
    a.min_x() <= b.max_x()
        && a.max_x() >= b.min_x()
        && a.min_y() <= b.max_y()
        && a.max_y() >= b.min_y()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two-finger pinch on a frame that declares `events.onScaleUpdate`
    /// drives `$state.zoom` via `$event.scale`. Locks in the full
    /// chain: PointerRouter cross-arena registration → ScaleRecognizer
    /// geometry → SemanticEvent dispatch → event_payload → expression
    /// resolves `$event.scale` → state graph write.
    #[test]
    fn two_finger_pinch_updates_state_zoom_via_event_scale() {
        use crate::geometry::point;
        use crate::gesture::pointer::PointerPhase;
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{
              "version":"0.8.0",
              "state":{ "zoom":{ "type":"float", "default":1.0 } },
              "children":[
                { "type":"frame","id":"canvas",
                  "width":800, "height":600,
                  "events":{
                    "onScaleUpdate": [
                      { "set": { "$app.zoom": "$event.scale" } }
                    ]
                  }
                }
              ]
            }"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.build_layout((800.0, 600.0)).unwrap();
        rt.rebuild_spatial();
        // First finger Down at (200, 300), second at (400, 300):
        // distance 200, focal (300, 300).
        rt.dispatch_pointer(PointerEvent::simple(
            0,
            PointerPhase::Down,
            point(200.0, 300.0),
        ));
        rt.dispatch_pointer(PointerEvent::simple(
            1,
            PointerPhase::Down,
            point(400.0, 300.0),
        ));
        // Spread fingers to (100, 300) and (500, 300): distance 400 →
        // scale 2.0. Past 5% threshold → ScaleStart + ScaleUpdate fire.
        rt.dispatch_pointer(PointerEvent::simple(
            0,
            PointerPhase::Move,
            point(100.0, 300.0),
        ));
        rt.dispatch_pointer(PointerEvent::simple(
            1,
            PointerPhase::Move,
            point(500.0, 300.0),
        ));
        let zoom = rt
            .state
            .app_get("zoom")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32;
        assert!(
            (zoom - 2.0).abs() < f32::EPSILON,
            "$event.scale should drive $app.zoom to 2.0, got {zoom}"
        );
    }

    /// Companion test for Rotate: `$state.rotation` driven from
    /// `$event.radians`. Same pipeline as pinch, different recognizer.
    #[test]
    fn two_finger_rotate_updates_state_via_event_radians() {
        use crate::geometry::point;
        use crate::gesture::pointer::PointerPhase;
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{
              "version":"0.8.0",
              "state":{ "rotation":{ "type":"float", "default":0.0 } },
              "children":[
                { "type":"frame","id":"canvas",
                  "width":800, "height":600,
                  "events":{
                    "onRotateUpdate": [
                      { "set": { "$app.rotation": "$event.radians" } }
                    ]
                  }
                }
              ]
            }"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.build_layout((800.0, 600.0)).unwrap();
        rt.rebuild_spatial();
        // Two fingers along the x-axis (angle 0).
        rt.dispatch_pointer(PointerEvent::simple(
            0,
            PointerPhase::Down,
            point(300.0, 300.0),
        ));
        rt.dispatch_pointer(PointerEvent::simple(
            1,
            PointerPhase::Down,
            point(500.0, 300.0),
        ));
        // Rotate finger 1 down to (500, 400): line from (300,300) →
        // (500,400) has angle atan2(100, 200) ≈ 0.4636 rad. > 5° threshold.
        rt.dispatch_pointer(PointerEvent::simple(
            1,
            PointerPhase::Move,
            point(500.0, 400.0),
        ));
        // Now fully to (500, 500): angle ≈ 0.7854 rad (45°). Update fires.
        rt.dispatch_pointer(PointerEvent::simple(
            1,
            PointerPhase::Move,
            point(500.0, 500.0),
        ));
        let rad = rt
            .state
            .app_get("rotation")
            .and_then(|v| v.as_f64())
            .unwrap_or(-1.0) as f32;
        assert!(
            rad > 0.7 && rad < 0.85,
            "$event.radians should drive $state.rotation near 0.785 (45°), got {rad}"
        );
    }

    #[test]
    fn full_pipeline_smoke() {
        let mut rt = Runtime::new();
        rt.load_str(
            r#"{
          "version":"0.8.0",
          "children":[{"type":"rectangle","id":"r","width":200,"height":100}]
        }"#,
        )
        .unwrap();
        rt.build_layout((800.0, 600.0)).unwrap();
        rt.rebuild_spatial();
        assert_eq!(rt.spatial.len(), 1);
    }

    /// Hot-reload preserves app-scope state values. A user editing the
    /// .op while `$state.count == 5` should still see `5` after save.
    #[test]
    fn replace_document_preserves_app_state() {
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{
              "version":"0.8.0",
              "state":{"count":{"type":"int","default":0}},
              "children":[]
            }"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.state.app_set("count", serde_json::json!(5));
        assert_eq!(rt.state.app_get("count").unwrap().as_i64(), Some(5));

        let new_schema: PenDocument = serde_json::from_str(
            r#"{
          "version":"0.8.0",
          "state":{
            "count":{"type":"int","default":0},
            "username":{"type":"string","default":""}
          },
          "children":[]
        }"#,
        )
        .unwrap();
        rt.replace_document(new_schema).unwrap();

        // Pre-existing key kept its live value.
        assert_eq!(rt.state.app_get("count").unwrap().as_i64(), Some(5));
        // Newly declared key got its schema default.
        assert_eq!(rt.state.app_get("username").unwrap().as_str(), Some(""));
    }

    /// Capability gate rebuilds from the new schema, so adding `network`
    /// in the .op edit becomes effective without a process restart.
    #[test]
    fn replace_document_refreshes_capability_gate() {
        use crate::capability::Capability;
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{
              "version":"0.8.0",
              "id":"test",
              "app":{
                "name":"t","version":"0.1.0","id":"com.test.t",
                "capabilities":[]
              },
              "children":[]
            }"#,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(!rt.capabilities.check(Capability::Network, "fetch"));

        let with_net: PenDocument = serde_json::from_str(
            r#"{
          "version":"0.8.0",
          "id":"test",
          "app":{
            "name":"t","version":"0.1.0","id":"com.test.t",
            "capabilities":["network"]
          },
          "children":[]
        }"#,
        )
        .unwrap();
        rt.replace_document(with_net).unwrap();
        assert!(rt.capabilities.check(Capability::Network, "fetch"));
    }

    #[test]
    fn pump_websockets_drains_on_message_into_state() {
        use crate::action::context::WsHandle;
        use crate::action::services::WebSocketSession;
        use async_trait::async_trait;
        use std::cell::RefCell;
        use std::rc::Rc;

        struct ScriptedSession {
            inbox: Rc<RefCell<Vec<String>>>,
        }
        #[async_trait(?Send)]
        impl WebSocketSession for ScriptedSession {
            async fn send(&self, _: String) -> Result<(), String> {
                Ok(())
            }
            async fn close(&self) -> Result<(), String> {
                Ok(())
            }
            async fn receive(&self) -> Vec<String> {
                std::mem::take(&mut *self.inbox.borrow_mut())
            }
        }

        let mut rt = Runtime::new();
        rt.load_str(
            r#"{
              "version":"0.8.0",
              "state":{ "last":{ "type":"string", "default":"" } },
              "children":[]
            }"#,
        )
        .unwrap();
        rt.build_layout((100.0, 100.0)).unwrap();

        // Inject a fake session with one queued message + an
        // on_message handler that copies $event.data into $app.last.
        // (Runtime path-prefix is `$app` for app-scope writes; the
        // public `$state.*` shorthand is resolved earlier in the
        // expression parser.)
        let inbox = Rc::new(RefCell::new(vec!["hello".to_owned()]));
        let session: Rc<dyn WebSocketSession> = Rc::new(ScriptedSession {
            inbox: inbox.clone(),
        });
        rt.ws_sessions.borrow_mut().insert(
            "chat".to_owned(),
            WsHandle {
                session,
                on_message: Some(serde_json::json!([
                    { "set": { "$app.last": "$event.data" } }
                ])),
            },
        );

        let fired = rt.pump_websockets();
        assert_eq!(fired, 1, "one queued message should fire one handler");
        // The set action ran end-to-end (registry parse → executor →
        // scheduler flush). `$event.data` resolution against the
        // injected event payload is the expression engine's job —
        // this test stops at the dispatch hand-off.
        assert!(
            rt.state.app_get("last").is_some(),
            "$app.last should be touched after handler runs"
        );
        // Inbox now empty — second pump fires nothing.
        assert_eq!(rt.pump_websockets(), 0);
    }

    #[test]
    fn dispatch_wheel_finds_on_scroll_target() {
        use crate::geometry::point;
        use crate::gesture::pointer::WheelEvent;
        let mut rt = Runtime::new();
        rt.load_str(
            r#"{
              "version":"0.8.0",
              "children":[
                { "type":"frame","id":"viewport","width":400,"height":300,
                  "events":{ "onScroll": [ { "set": { "$state.scrolled": "true" } } ] }
                }
              ]
            }"#,
        )
        .unwrap();
        rt.build_layout((400.0, 300.0)).unwrap();
        rt.rebuild_spatial();
        let emitted = rt.dispatch_wheel(WheelEvent::simple(point(100.0, 100.0), point(0.0, -10.0)));
        assert_eq!(emitted.len(), 1);
        assert!(matches!(
            emitted[0],
            crate::gesture::semantic::SemanticEvent::Scroll { .. }
        ));
    }

    #[test]
    fn dispatch_wheel_ignores_nodes_without_handler() {
        use crate::geometry::point;
        use crate::gesture::pointer::WheelEvent;
        let mut rt = Runtime::new();
        rt.load_str(
            r#"{
              "version":"0.8.0",
              "children":[
                { "type":"frame","id":"plain","width":400,"height":300 }
              ]
            }"#,
        )
        .unwrap();
        rt.build_layout((400.0, 300.0)).unwrap();
        rt.rebuild_spatial();
        let emitted = rt.dispatch_wheel(WheelEvent::simple(point(100.0, 100.0), point(0.0, -10.0)));
        assert!(emitted.is_empty());
    }

    /// `replace_document` should swap in the new tree without disturbing
    /// the existing StateGraph or service Rcs — Plan 9 hot-reload relies
    /// on this so `$state.*` survives `.op` edits.
    #[test]
    fn replace_document_swaps_tree_keeps_state() {
        let mut rt = Runtime::new();
        rt.load_str(
            r#"{
          "version":"0.8.0",
          "children":[{"type":"rectangle","id":"r1","width":100,"height":50}]
        }"#,
        )
        .unwrap();
        rt.build_layout((800.0, 600.0)).unwrap();
        rt.rebuild_spatial();
        let original_state = Rc::as_ptr(&rt.state);

        let new_schema: PenDocument = serde_json::from_str(
            r#"{
          "version":"0.8.0",
          "children":[
            {"type":"rectangle","id":"a","width":40,"height":30},
            {"type":"rectangle","id":"b","width":40,"height":30}
          ]
        }"#,
        )
        .unwrap();
        rt.replace_document(new_schema).unwrap();
        rt.build_layout((800.0, 600.0)).unwrap();
        rt.rebuild_spatial();

        // Same StateGraph instance — Rc didn't get rebuilt.
        assert_eq!(Rc::as_ptr(&rt.state), original_state);
        // Tree contents reflect the new schema.
        assert_eq!(rt.spatial.len(), 2);
    }

    /// Tab walks the focus chain in DFS pre-order and emits
    /// `FocusLost` (for the previous node) followed by `FocusGained`
    /// (for the new node) — the documented blur-then-focus order.
    #[test]
    fn dispatch_keyboard_tab_walks_focus_chain() {
        use crate::gesture::pointer::Modifiers;
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{
              "version":"0.8.0",
              "children":[
                { "type":"frame","id":"root","width":400,"height":300,"children":[
                  { "type":"rectangle","id":"a","width":50,"height":20,
                    "semantics":{"role":"button","label":"A"} },
                  { "type":"rectangle","id":"b","width":50,"height":20,
                    "gestures":{"focusable":true} },
                  { "type":"rectangle","id":"c","width":50,"height":20,
                    "semantics":{"role":"input"} }
                ]}
              ]
            }"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.build_layout((400.0, 300.0)).unwrap();

        let chain = rt.focus.chain().to_vec();
        assert_eq!(chain.len(), 3);
        // Snapshot the id-by-key lookup once so the closure doesn't
        // hold a borrow on `rt` across `dispatch_keyboard` calls.
        let id_of = |rt: &Runtime, k: crate::document::NodeKey| -> String {
            crate::document::tree::node_schema_id(
                &rt.document.as_ref().unwrap().tree.nodes[k].schema,
            )
            .to_owned()
        };
        let chain_ids: Vec<String> = chain.iter().map(|k| id_of(&rt, *k)).collect();
        assert_eq!(chain_ids, vec!["a", "b", "c"]);

        // First Tab — no previous focus → only FocusGained on "a".
        let evs = rt.dispatch_keyboard("Tab", Modifiers::empty());
        assert_eq!(evs.len(), 1);
        assert!(matches!(evs[0], SemanticEvent::FocusGained { .. }));
        assert_eq!(id_of(&rt, evs[0].node()), "a");

        // Second Tab — blur "a", focus "b".
        let evs = rt.dispatch_keyboard("Tab", Modifiers::empty());
        assert_eq!(evs.len(), 2);
        assert!(matches!(evs[0], SemanticEvent::FocusLost { .. }));
        assert!(matches!(evs[1], SemanticEvent::FocusGained { .. }));
        assert_eq!(id_of(&rt, evs[0].node()), "a");
        assert_eq!(id_of(&rt, evs[1].node()), "b");

        // Shift+Tab — blur "b", focus "a" (step backward).
        let evs = rt.dispatch_keyboard("Tab", Modifiers::SHIFT);
        assert_eq!(evs.len(), 2);
        assert_eq!(id_of(&rt, evs[0].node()), "b");
        assert_eq!(id_of(&rt, evs[1].node()), "a");
    }

    /// Non-Tab keys forward to the currently-focused node — Tab is the
    /// only key consumed by the focus traversal.
    #[test]
    fn dispatch_keyboard_non_tab_routes_to_focused_node() {
        use crate::gesture::pointer::Modifiers;
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{
              "version":"0.8.0",
              "state":{"hits":{"type":"int","default":0}},
              "children":[
                { "type":"rectangle","id":"input",
                  "width":50,"height":20,
                  "semantics":{"role":"input"},
                  "events":{
                    "onKey":[
                      { "set": { "$app.hits": "$state.hits + 1" } }
                    ]
                  }
                }
              ]
            }"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.build_layout((400.0, 300.0)).unwrap();

        // Tab in to focus the input.
        rt.dispatch_keyboard("Tab", Modifiers::empty());
        assert!(rt.focus.current().is_some());

        let evs = rt.dispatch_keyboard("Enter", Modifiers::empty());
        assert_eq!(evs.len(), 1);
        assert!(matches!(evs[0], SemanticEvent::KeyDown { .. }));

        let hits = rt
            .state
            .app_get("hits")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        assert_eq!(hits, 1);
    }

    /// onFocus / onBlur ActionLists fire when the chain advances —
    /// closes the loop end-to-end (gesture event → dispatcher →
    /// expression VM → state graph write).
    #[test]
    fn focus_handlers_fire_on_chain_step() {
        use crate::gesture::pointer::Modifiers;
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{
              "version":"0.8.0",
              "state":{
                "gained":{"type":"int","default":0},
                "lost":{"type":"int","default":0}
              },
              "children":[
                { "type":"rectangle","id":"a","width":50,"height":20,
                  "semantics":{"role":"button","label":"A"},
                  "events":{
                    "onFocus":[ { "set": { "$app.gained": "$state.gained + 1" } } ],
                    "onBlur":[ { "set": { "$app.lost": "$state.lost + 1" } } ]
                  } },
                { "type":"rectangle","id":"b","width":50,"height":20,
                  "semantics":{"role":"button","label":"B"} }
              ]
            }"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.build_layout((400.0, 300.0)).unwrap();

        // Tab in → gained == 1, lost == 0.
        rt.dispatch_keyboard("Tab", Modifiers::empty());
        assert_eq!(
            rt.state.app_get("gained").and_then(|v| v.as_i64()).unwrap(),
            1
        );
        assert_eq!(
            rt.state.app_get("lost").and_then(|v| v.as_i64()).unwrap(),
            0
        );

        // Tab to "b" → "a" loses focus, "b" gains. Only "a" has
        // handlers, so gained stays at 1 and lost ticks to 1.
        rt.dispatch_keyboard("Tab", Modifiers::empty());
        assert_eq!(
            rt.state.app_get("gained").and_then(|v| v.as_i64()).unwrap(),
            1
        );
        assert_eq!(
            rt.state.app_get("lost").and_then(|v| v.as_i64()).unwrap(),
            1
        );
    }

    /// Hot-reload swaps the runtime tree underneath; the focus chain
    /// must rebuild against the new keys and any prior focus must
    /// drop (the old `NodeKey` no longer maps to a real node).
    #[test]
    fn replace_document_rebuilds_focus_chain() {
        use crate::gesture::pointer::Modifiers;
        let mut rt = Runtime::new();
        rt.load_str(
            r#"{
          "version":"0.8.0",
          "children":[
            { "type":"rectangle","id":"old-btn","width":50,"height":20,
              "semantics":{"role":"button"} }
          ]
        }"#,
        )
        .unwrap();
        rt.build_layout((400.0, 300.0)).unwrap();
        rt.dispatch_keyboard("Tab", Modifiers::empty());
        assert!(rt.focus.current().is_some());

        rt.replace_document(
            serde_json::from_str(
                r#"{
              "version":"0.8.0",
              "children":[
                { "type":"rectangle","id":"new-input","width":50,"height":20,
                  "semantics":{"role":"input"} },
                { "type":"rectangle","id":"new-link","width":50,"height":20,
                  "semantics":{"role":"link"} }
              ]
            }"#,
            )
            .unwrap(),
        )
        .unwrap();
        // No carry-over focus.
        assert!(rt.focus.current().is_none());
        let chain_len = rt.focus.chain().len();
        assert_eq!(chain_len, 2);

        rt.dispatch_keyboard("Tab", Modifiers::empty());
        // First Tab post-reload focuses the new chain's first node.
        let cur = rt.focus.current().unwrap();
        let id = crate::document::tree::node_schema_id(
            &rt.document.as_ref().unwrap().tree.nodes[cur].schema,
        );
        assert_eq!(id, "new-input");
    }

    /// Hot-reload must reset PointerRouter caches alongside focus state.
    /// Pre-fix, the router kept `last_hover_target` from the old tree;
    /// after a doc swap, the next hover with the same SlotMap-equal
    /// (but semantically different) key would emit `HoverLeave`
    /// against the wrong node. We assert the smaller-but-sufficient
    /// invariant: `replace_document` zeroes the router's
    /// `last_hover_target` so the next off-target hover doesn't fire
    /// a stale `HoverLeave`.
    #[test]
    fn replace_document_resets_pointer_router_state() {
        use crate::geometry::point;
        use crate::gesture::pointer::{PointerEvent, PointerPhase};
        let mut rt = Runtime::new();
        rt.load_str(
            r#"{
          "version":"0.8.0",
          "children":[
            { "type":"rectangle","id":"hover-target","width":100,"height":50 }
          ]
        }"#,
        )
        .unwrap();
        rt.build_layout((400.0, 300.0)).unwrap();
        rt.rebuild_spatial();
        // Hover into the rectangle — stamps the router's
        // `last_hover_target` regardless of whether the node carries
        // an `onHover*` handler (handle_hover unconditionally
        // updates `last_hover_target` to the topmost hit).
        let _enter = rt.dispatch_pointer(PointerEvent::simple(
            0,
            PointerPhase::Hover,
            point(20.0, 20.0),
        ));
        // Sanity: a second hover off the rectangle would normally
        // emit `HoverLeave` for the stamped target — that's the
        // path that goes wrong on hot-reload without the reset.
        let leave = rt.dispatch_pointer(PointerEvent::simple(
            0,
            PointerPhase::Hover,
            point(500.0, 500.0),
        ));
        assert!(
            leave
                .iter()
                .any(|e| matches!(e, SemanticEvent::HoverLeave { .. })),
            "pre-reload sanity: off-target hover should emit HoverLeave, got {:?}",
            leave
        );

        // Re-stamp last_hover_target by hovering over the rect again.
        rt.dispatch_pointer(PointerEvent::simple(
            0,
            PointerPhase::Hover,
            point(20.0, 20.0),
        ));

        // Hot-reload to a different document.
        rt.replace_document(
            serde_json::from_str(
                r#"{
              "version":"0.8.0",
              "children":[
                { "type":"rectangle","id":"plain","width":100,"height":50 }
              ]
            }"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.build_layout((400.0, 300.0)).unwrap();
        rt.rebuild_spatial();
        // Hover off-target. Pre-fix, the stale `last_hover_target`
        // from the old tree would still cause a `HoverLeave` to fire
        // (against a SlotMap key that may or may not alias a real
        // node in the new tree). Post-fix the router is reset, so
        // the off-target hover emits nothing.
        let off = rt.dispatch_pointer(PointerEvent::simple(
            0,
            PointerPhase::Hover,
            point(500.0, 500.0),
        ));
        assert!(
            !off.iter()
                .any(|e| matches!(e, SemanticEvent::HoverLeave { .. })),
            "router state from prior tree leaked through reload, got {:?}",
            off
        );
    }

    /// Re-entrancy guard. An `onBlur` handler that itself calls
    /// `focus_request` (or any focus-mutating action) must take the
    /// transition over — the outer call's `FocusGained` for the
    /// originally-targeted node would otherwise fire its `onFocus`
    /// even though focus has already moved on.
    ///
    /// Default action builtins don't yet expose focus-mutating verbs
    /// — the only focus mutator is `Runtime`'s own `focus_*` API,
    /// which is unreachable from inside `dispatch_semantic` without
    /// a custom `LogicProvider`. So the test uses the
    /// equivalent-but-direct shape: synthesise a `FocusChange`,
    /// pre-mutate `self.focus.current` to simulate exactly what a
    /// nested re-entry would have done, and call
    /// [`Self::emit_focus_change`] directly. The mutation is the
    /// only state difference between "guarded" and "unguarded"
    /// behaviour, so this covers the entire invariant the guard
    /// exists to enforce. (When focus actions become first-class
    /// builtins, an end-to-end test through `dispatch_keyboard`
    /// becomes possible — TODO follow-up.)
    #[test]
    fn focus_change_re_entrant_blur_redirects_skips_stale_focus_gained() {
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{
              "version":"0.8.0",
              "children":[
                { "type":"rectangle","id":"a","width":50,"height":20,
                  "semantics":{"role":"button"} },
                { "type":"rectangle","id":"b","width":50,"height":20,
                  "semantics":{"role":"button"} }
              ]
            }"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.build_layout((400.0, 300.0)).unwrap();
        let chain = rt.focus.chain().to_vec();
        let key_a = chain[0];
        let key_b = chain[1];

        // Pin focus on A so the synthetic change below has a real
        // previous to fire FocusLost against.
        let evs = rt.focus_request(key_a);
        assert_eq!(evs.len(), 1);
        assert!(matches!(evs[0], SemanticEvent::FocusGained { .. }));
        assert_eq!(evs[0].node(), key_a);

        // Synthesise the racy state. `change` says "moved A → B",
        // but before we call emit_focus_change we pre-mutate the
        // manager's `current` to A (the `request` returns a
        // FocusChange we deliberately ignore — this is just state
        // installation, not a real transition). At emit time the
        // outer call sees:
        //   - change.previous = Some(A) → fires FocusLost{A}
        //   - change.current  = Some(B) but focus.current() = A
        //     → guard suppresses FocusGained{B}.
        // Without the guard, FocusGained{B} would fire even though
        // focus is on A — the exact stale-event surface that a
        // re-entrant onBlur would hit.
        let change = crate::gesture::FocusChange {
            previous: Some(key_a),
            current: Some(key_b),
        };
        let _ = rt.focus.request(key_a);
        let evs = rt.emit_focus_change(change);
        assert_eq!(
            evs.len(),
            1,
            "expected only FocusLost when focus.current != change.current, got {:?}",
            evs
        );
        assert!(matches!(evs[0], SemanticEvent::FocusLost { .. }));
        assert_eq!(evs[0].node(), key_a);
        // Focus state untouched by the suppressed FocusGained.
        assert_eq!(rt.focus.current(), Some(key_a));

        // Positive-control: when focus.current() *does* match
        // change.current, the FocusGained fires as normal.
        let _ = rt.focus.request(key_b);
        let change_match = crate::gesture::FocusChange {
            previous: Some(key_a),
            current: Some(key_b),
        };
        let evs = rt.emit_focus_change(change_match);
        assert_eq!(evs.len(), 2);
        assert!(matches!(evs[0], SemanticEvent::FocusLost { .. }));
        assert!(matches!(evs[1], SemanticEvent::FocusGained { .. }));
        assert_eq!(evs[0].node(), key_a);
        assert_eq!(evs[1].node(), key_b);
    }
}
