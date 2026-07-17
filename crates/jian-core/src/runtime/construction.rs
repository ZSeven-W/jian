use super::{Runtime, AUDIT_LOG_CAPACITY};
use crate::action::services::{
    NullClipboard, NullFeedback, NullNetworkClient, NullPlatform, NullRouter, NullStorageBackend,
};
use crate::action::{default_registry, TaskClock, TaskQueue};
use crate::binding::DeferredBindingQueue;
use crate::capability::{
    from_schema_capability, AuditLog, DeclaredCapabilityGate, DummyCapabilityGate,
    NullPermissionBroker,
};
use crate::document::loader;
use crate::effect::EffectRegistry;
use crate::error::CoreResult;
use crate::expression::ExpressionCache;
use crate::geometry::size;
use crate::gesture::{collect_focus_chain, FocusManager, PointerRouter};
use crate::layout::LayoutEngine;
use crate::signal::scheduler::Scheduler;
use crate::spatial::SpatialIndex;
use crate::state::StateGraph;
use crate::viewport::Viewport;
use jian_ops_schema::document::PenDocument;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

impl Runtime {
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new() -> Self {
        let scheduler = Rc::new(Scheduler::new());
        let mutation_counter = Rc::new(Cell::new(0));
        let widget_states =
            crate::widget_state::WidgetStateStore::with_counter(mutation_counter.clone());
        let effects = EffectRegistry::new();
        effects.install_on(&scheduler);
        let runtime = Self {
            state: Rc::new(StateGraph::new_with_counter(
                scheduler.clone(),
                mutation_counter.clone(),
            )),
            scheduler,
            effects,
            document: None,
            layout: LayoutEngine::new(),
            spatial: SpatialIndex::new(),
            image_store: Default::default(),
            image_resolver: Rc::new(crate::render::image_store::NullImageResolver),
            text_geometry: None,
            text_geometry_ready: false,
            image_completions: Rc::new(RefCell::new(Vec::new())),
            image_requests: BTreeMap::new(),
            image_request_sources: BTreeMap::new(),
            image_document_dir: PathBuf::new(),
            viewport: Viewport::new(size(800.0, 600.0)),
            load_warnings: Vec::new(),
            layout_errors: Vec::new(),
            variant_table: Default::default(),
            active_screen_path: None,
            active_variant_page_id: None,
            active_page_key: String::new(),
            ime_registry: Default::default(),
            swap_state: Default::default(),
            variant_source: None,
            mutation_counter,
            layout_mutation_seen: 0,
            font_generation_seen: 0,
            last_variant_build_count: 0,
            action_surface_inputs: Vec::new(),
            action_surface_generation: 0,
            dirty: true,
            task_queue: TaskQueue::default(),
            action_outcomes: Vec::new(),
            action_reporting_enabled: false,
            task_clock: Arc::new(TaskClock::default()),
            document_generation: 0,

            gestures: PointerRouter::new(),
            focus: FocusManager::new(),
            widget_states,
            now_ms: 0,
            actions: default_registry(),
            expr_cache: Rc::new(ExpressionCache::new()),
            deferred_bindings: DeferredBindingQueue::new(),
            network: Rc::new(NullNetworkClient),
            ws_sessions: Rc::new(RefCell::new(HashMap::new())),
            ws_receive_active: BTreeSet::new(),
            ws_messages: Rc::new(RefCell::new(Vec::new())),
            storage: Rc::new(NullStorageBackend),
            nav: Rc::new(NullRouter),
            feedback: Rc::new(NullFeedback),
            async_feedback: Rc::new(NullFeedback),
            clipboard: Rc::new(NullClipboard),
            platform: Rc::new(NullPlatform),
            capabilities: Rc::new(DummyCapabilityGate),
            audit: None,
            permissions: Rc::new(NullPermissionBroker),
            logic: Rc::new(crate::logic::NullLogicProvider),
            #[cfg(test)]
            fail_next_loader: false,
            #[cfg(debug_assertions)]
            fail_next_variant_build: Cell::new(false),
        };
        runtime.state.set_viewport(800.0, 600.0, 1.0);
        runtime
    }

    /// Build a runtime whose `CapabilityGate` is derived from the
    /// document's `app.capabilities` declaration. Checks are recorded in
    /// an `AuditLog` attached to `self.audit`.
    ///
    /// An undeclared `app.capabilities` field means "no capabilities" —
    /// every IO action will be denied. Ship the `.op` with an explicit
    /// declaration to unlock network/storage/etc.
    ///
    /// Responsive breakpoint variants are selected against the default
    /// 800x600 viewport. A host that already knows its first-frame size must
    /// use [`Self::new_from_document_with_viewport`], otherwise the initially
    /// mounted variant can be wrong for the real window and nothing corrects
    /// it until a resize-driven swap.
    pub fn new_from_document(schema: PenDocument) -> CoreResult<Self> {
        Self::new_from_document_with_viewport(schema, (800.0, 600.0))
    }

    /// [`Self::new_from_document`], but the initial responsive breakpoint
    /// variant is selected for the host's real first-frame viewport.
    ///
    /// Non-responsive documents ignore `viewport` entirely and construct
    /// byte-for-byte like `new_from_document` (their layout viewport stays at
    /// the 800x600 default so later occlusion-driven relayouts are
    /// unaffected).
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new_from_document_with_viewport(
        schema: PenDocument,
        viewport: (f32, f32),
    ) -> CoreResult<Self> {
        let scheduler = Rc::new(Scheduler::new());
        let effects = EffectRegistry::new();
        effects.install_on(&scheduler);

        let viewport = if schema.is_responsive() {
            viewport
        } else {
            (800.0, 600.0)
        };
        let prepared = super::document_prepare::prepare_document(schema, viewport, None);
        let schema = prepared.mounted;
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

        let responsive = schema.is_responsive();
        let active_screen_path = prepared.path;
        let active_variant_page_id = prepared.selected_page_id;
        let active_page_key = if responsive {
            active_variant_page_id.clone().unwrap_or_default()
        } else {
            String::new()
        };
        let mutation_counter = Rc::new(Cell::new(0));
        let widget_states =
            crate::widget_state::WidgetStateStore::with_counter(mutation_counter.clone());
        let state = Rc::new(StateGraph::new_with_counter(
            scheduler.clone(),
            mutation_counter.clone(),
        ));
        state.set_responsive(responsive);
        let doc = loader::build(schema, &state)?;
        let action_surface_inputs =
            crate::action_surface::derive_actions(&doc.schema, &crate::action_surface::BUILD_SALT);
        let focus_chain = collect_focus_chain(&doc);
        let mut focus = FocusManager::new();
        focus.set_chain(focus_chain);

        let mut runtime = Self {
            state,
            scheduler,
            effects,
            document: Some(doc),
            layout: LayoutEngine::new(),
            spatial: SpatialIndex::new(),
            image_store: Default::default(),
            image_resolver: Rc::new(crate::render::image_store::NullImageResolver),
            text_geometry: None,
            text_geometry_ready: false,
            image_completions: Rc::new(RefCell::new(Vec::new())),
            image_requests: BTreeMap::new(),
            image_request_sources: BTreeMap::new(),
            image_document_dir: PathBuf::new(),
            viewport: Viewport::new(size(viewport.0, viewport.1)),
            load_warnings: prepared.warnings,
            layout_errors: Vec::new(),
            variant_table: prepared.variants,
            active_screen_path,
            active_variant_page_id,
            active_page_key: active_page_key.clone(),
            ime_registry: Default::default(),
            swap_state: Default::default(),
            variant_source: prepared.source,
            layout_mutation_seen: mutation_counter.get(),
            font_generation_seen: 0,
            mutation_counter,
            last_variant_build_count: 0,
            action_surface_inputs,
            action_surface_generation: 1,
            dirty: true,
            task_queue: TaskQueue::default(),
            action_outcomes: Vec::new(),
            action_reporting_enabled: false,
            task_clock: Arc::new(TaskClock::default()),
            document_generation: 1,

            gestures: PointerRouter::new(),
            focus,
            widget_states,
            now_ms: 0,
            actions: default_registry(),
            expr_cache: Rc::new(ExpressionCache::new()),
            deferred_bindings: DeferredBindingQueue::new(),
            network: Rc::new(NullNetworkClient),
            ws_sessions: Rc::new(RefCell::new(HashMap::new())),
            ws_receive_active: BTreeSet::new(),
            ws_messages: Rc::new(RefCell::new(Vec::new())),
            storage: Rc::new(NullStorageBackend),
            nav: Rc::new(NullRouter),
            feedback: Rc::new(NullFeedback),
            async_feedback: Rc::new(NullFeedback),
            clipboard: Rc::new(NullClipboard),
            platform: Rc::new(NullPlatform),
            capabilities: gate,
            audit: Some(audit),
            permissions: Rc::new(NullPermissionBroker),
            logic: Rc::new(crate::logic::NullLogicProvider),
            #[cfg(test)]
            fail_next_loader: false,
            #[cfg(debug_assertions)]
            fail_next_variant_build: Cell::new(false),
        };
        runtime.widget_states.set_page_key(active_page_key);
        runtime.state.set_viewport(viewport.0, viewport.1, 1.0);
        runtime.admit_document_images();
        Ok(runtime)
    }
}
