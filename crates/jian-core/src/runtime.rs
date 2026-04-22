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
        let doc = loader::build(schema, &self.state)?;
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
}
