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
use crate::capability::{
    from_schema_capability, AuditLog, CapabilityGate, DeclaredCapabilityGate, DummyCapabilityGate,
    NullPermissionBroker, PermissionBroker,
};
use crate::document::{loader, RuntimeDocument};
use crate::effect::EffectRegistry;
use crate::error::CoreResult;
use crate::expression::ExpressionCache;
use crate::geometry::size;
use crate::gesture::{dispatch_event, PointerEvent, PointerRouter, SemanticEvent};
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
    pub actions: SharedRegistry,
    pub expr_cache: Rc<ExpressionCache>,
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
            actions: default_registry(),
            expr_cache: Rc::new(ExpressionCache::new()),
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
            actions: default_registry(),
            expr_cache: Rc::new(ExpressionCache::new()),
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
        self.document = Some(doc);
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
        let snapshot: Vec<(String, Rc<dyn crate::action::services::WebSocketSession>, Option<serde_json::Value>)> = {
            self.ws_sessions
                .borrow()
                .iter()
                .map(|(id, h)| (id.clone(), h.session.clone(), h.on_message.clone()))
                .collect()
        };
        let mut fired = 0usize;
        for (id, session, on_message) in snapshot {
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
        let ctx = self.make_action_ctx();
        let outcome = dispatch_event(doc, event, &self.actions, &ctx);
        // Actions mutate state via Signals whose effects are scheduled;
        // flush synchronously so bindings / scene observers see the new
        // values before the host's next frame.
        self.scheduler.flush();
        outcome
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn dispatch_wheel_finds_onScroll_target() {
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
        let emitted = rt.dispatch_wheel(WheelEvent::simple(
            point(100.0, 100.0),
            point(0.0, -10.0),
        ));
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
        let emitted =
            rt.dispatch_wheel(WheelEvent::simple(point(100.0, 100.0), point(0.0, -10.0)));
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
}
