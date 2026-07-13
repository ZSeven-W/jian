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
    NullNetworkClient, NullPlatform, NullRouter, NullStorageBackend, PlatformService, RouteState,
    Router as RouterSvc, StorageBackend,
};
use crate::action::{
    default_registry, ActionContext, CancellationToken, ExecOutcome, SharedRegistry, TaskClock,
    TaskQueue,
};
use crate::binding::{BindingEffect, DeferredBindingQueue};
use crate::capability::{
    from_schema_capability, AuditLog, CapabilityGate, DeclaredCapabilityGate, DummyCapabilityGate,
    NullPermissionBroker, PermissionBroker,
};
use crate::document::{loader, RuntimeDocument};
use crate::effect::EffectRegistry;
use crate::error::{CoreError, CoreResult};
use crate::expression::ExpressionCache;
use crate::geometry::size;
use crate::gesture::{
    collect_focus_chain, FocusManager, PointerEvent, PointerRouter, SemanticEvent,
};
use crate::layout::{LayoutEngine, StagedLayout};
use crate::signal::scheduler::Scheduler;
use crate::spatial::{NodeBBox, SpatialIndex};
use crate::state::StateGraph;
use crate::viewport::Viewport;
use jian_ops_schema::{document::PenDocument, load_str};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

struct ImageRequest {
    task_id: u64,
    owner_generation: Rc<Cell<u64>>,
}

struct ImageCompletion {
    key: String,
    owner_generation: Rc<Cell<u64>>,
    result: Result<Vec<u8>, String>,
}

type ImageCompletionQueue = Rc<RefCell<Vec<ImageCompletion>>>;
type WebSocketMessageQueue = Rc<RefCell<Vec<(String, u64, Vec<String>)>>>;

mod ime_handshake;
mod pump;
mod variant_swap;
pub use ime_handshake::{ImeConfirmOutcome, ImeControlOp, ImeHost, ImeSnapshot};
pub use pump::FrameDirective;
pub use variant_swap::{ParkedBuild, SwapState};

/// Default audit-log size. 1000 entries is generous for in-session
/// inspection without letting long-lived hosts grow unboundedly.
const AUDIT_LOG_CAPACITY: usize = 1000;

struct PreparedDocument {
    mounted: PenDocument,
    source: Option<PenDocument>,
    variants: jian_ops_schema::screen_projection::ScreenVariantTable,
    path: Option<String>,
    selected_page_id: Option<String>,
    warnings: Vec<String>,
}

fn prepare_document(
    mut schema: PenDocument,
    viewport: (f32, f32),
    preferred_path: Option<&str>,
) -> PreparedDocument {
    if !schema.is_responsive() {
        return PreparedDocument {
            mounted: schema,
            source: None,
            variants: Default::default(),
            path: None,
            selected_page_id: None,
            warnings: Vec::new(),
        };
    }
    let (projected, projection_warnings) =
        jian_ops_schema::screen_projection::project_screens(&schema);
    let mut warnings: Vec<String> = projection_warnings
        .into_iter()
        .map(|warning| warning.to_string())
        .collect();
    if let Some((source, variants)) = projected {
        let path = preferred_path
            .filter(|path| variants.0.contains_key(*path))
            .map(str::to_owned)
            .or_else(|| source.routes.as_ref().map(|routes| routes.entry.clone()))
            .unwrap_or_else(|| "/".to_owned());
        let selected_page_id = select_variant_page(&variants, &path, viewport.0);
        let mut mounted = source.clone();
        if let Some(selected) = selected_page_id.as_deref() {
            mounted.pages = source
                .pages
                .as_ref()
                .and_then(|pages| pages.iter().find(|page| page.id == selected))
                .cloned()
                .map(|page| vec![page]);
        }
        return PreparedDocument {
            mounted,
            source: Some(source),
            variants,
            path: Some(path),
            selected_page_id,
            warnings,
        };
    }
    if let Some(pages) = schema.pages.as_mut() {
        let had_routes = schema.routes.is_some();
        let mut routes = schema
            .routes
            .take()
            .unwrap_or(jian_ops_schema::routes::RoutesConfig {
                entry: String::new(),
                routes: Default::default(),
                transitions: None,
            });
        let mut id_warnings = Vec::new();
        jian_ops_schema::page_ids::normalize_page_ids(pages, &mut routes, &mut id_warnings);
        warnings.extend(id_warnings.into_iter().map(|warning| warning.to_string()));
        if had_routes {
            schema.routes = Some(routes);
        }
    }
    let selected_page_id = schema
        .pages
        .as_ref()
        .and_then(|pages| pages.first())
        .map(|page| page.id.clone());
    PreparedDocument {
        mounted: schema,
        source: None,
        variants: Default::default(),
        path: None,
        selected_page_id,
        warnings,
    }
}

fn select_variant_page(
    variants: &jian_ops_schema::screen_projection::ScreenVariantTable,
    path: &str,
    width: f32,
) -> Option<String> {
    let set = variants.0.get(path)?;
    Some(
        set.ranged
            .iter()
            .find(|entry| {
                entry.range.min_width.unwrap_or(0.0) as f32 <= width
                    && width <= entry.range.max_width.unwrap_or(f64::INFINITY) as f32
            })
            .map_or_else(
                || set.default_page_id.clone(),
                |entry| entry.page_id.clone(),
            ),
    )
}

fn copy_layout_scopes(source: &StateGraph, target: &StateGraph, storage_allowed: bool) {
    target.replace_app(&source.app_snapshot());
    target.replace_vars(&source.vars_snapshot());
    target.replace_route(&source.route_snapshot());
    target.replace_viewport(&source.viewport_snapshot());
    if storage_allowed {
        target.replace_storage(&source.storage_snapshot());
        if let serde_json::Value::Object(values) = source.storage_cache.snapshot() {
            for (key, value) in values {
                target.storage_cache.set_local(&key, value);
            }
        }
    }
    for page_key in source.page_keys() {
        target.replace_page(&page_key, &source.page_snapshot(&page_key));
    }
    for (page_key, node_id) in source.self_keys() {
        target.replace_self(
            &page_key,
            &node_id,
            &source.self_snapshot(&page_key, &node_id),
        );
    }
}

fn route_values(route: &RouteState) -> BTreeMap<String, serde_json::Value> {
    BTreeMap::from([
        ("path".to_owned(), serde_json::json!(route.path)),
        ("params".to_owned(), serde_json::json!(route.params)),
        ("query".to_owned(), serde_json::json!(route.query)),
        ("stack".to_owned(), serde_json::json!(route.stack)),
    ])
}

fn normalized_route_values(
    route: &RouteState,
    valid_paths: &[String],
) -> BTreeMap<String, serde_json::Value> {
    let valid: BTreeSet<&str> = valid_paths.iter().map(String::as_str).collect();
    let survives = valid.contains(route.path.as_str());
    let (path, params, query, mut stack) = if survives {
        (
            route.path.clone(),
            route.params.clone(),
            route.query.clone(),
            route
                .stack
                .iter()
                .filter(|path| valid.contains(path.as_str()))
                .cloned()
                .collect(),
        )
    } else {
        (
            valid_paths.first().cloned().unwrap_or_else(|| "/".into()),
            BTreeMap::new(),
            BTreeMap::new(),
            Vec::new(),
        )
    };
    if stack.last() != Some(&path) {
        stack.push(path.clone());
    }
    BTreeMap::from([
        ("path".to_owned(), serde_json::json!(path)),
        ("params".to_owned(), serde_json::json!(params)),
        ("query".to_owned(), serde_json::json!(query)),
        ("stack".to_owned(), serde_json::json!(stack)),
    ])
}

pub struct Runtime {
    pub scheduler: Rc<Scheduler>,
    pub effects: Rc<EffectRegistry>,
    pub state: Rc<StateGraph>,
    pub document: Option<RuntimeDocument>,
    pub layout: LayoutEngine,
    pub spatial: SpatialIndex,
    pub image_store: crate::render::image_store::ImageStore,
    pub image_resolver: Rc<dyn crate::render::image_store::ImageResolver>,
    image_completions: ImageCompletionQueue,
    image_requests: BTreeMap<String, ImageRequest>,
    image_request_sources: BTreeMap<String, String>,
    image_document_dir: PathBuf,
    pub viewport: Viewport,
    load_warnings: Vec<String>,
    layout_errors: Vec<String>,
    variant_table: jian_ops_schema::screen_projection::ScreenVariantTable,
    active_screen_path: Option<String>,
    active_variant_page_id: Option<String>,
    active_page_key: String,
    ime_registry: ime_handshake::ImeRegistry,
    swap_state: SwapState,
    variant_source: Option<PenDocument>,
    mutation_counter: Rc<Cell<u64>>,
    layout_mutation_seen: u64,
    last_variant_build_count: usize,
    action_surface_inputs: Vec<crate::action_surface::ActionDefinition>,
    action_surface_generation: u64,
    dirty: bool,
    task_queue: TaskQueue,
    action_outcomes: Vec<ReportedActionOutcome>,
    action_reporting_enabled: bool,
    task_clock: Arc<TaskClock>,
    document_generation: u64,

    // --- Gesture + Action wiring (Plan 5 T15) ---
    pub gestures: PointerRouter,
    /// Tab-tree focus state. Rebuilt on every document swap from the
    /// runtime tree. See [`Runtime::dispatch_keyboard`] /
    /// [`Runtime::focus_next`] / [`Runtime::focus_request`].
    pub focus: FocusManager,
    /// Per-node runtime state for interactive widget nodes (text input,
    /// toggle, select, slider, radio, tabs). Seeded lazily from node
    /// props; preserved across `replace_document` for surviving ids.
    pub widget_states: crate::widget_state::WidgetStateStore,
    /// Host-updated monotonic clock in milliseconds. Drives widget
    /// caret-blink phase; hosts call [`Runtime::set_now_ms`] each frame
    /// (the gesture pipeline keeps its own `Instant` via [`Runtime::tick`]).
    pub now_ms: u64,
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
    ws_receive_active: BTreeSet<String>,
    ws_messages: WebSocketMessageQueue,
    pub storage: Rc<dyn StorageBackend>,
    pub nav: Rc<dyn RouterSvc>,
    pub feedback: Rc<dyn FeedbackSink>,
    pub async_feedback: Rc<dyn AsyncFeedback>,
    pub clipboard: Rc<dyn ClipboardService>,
    pub platform: Rc<dyn PlatformService>,
    pub capabilities: Rc<dyn CapabilityGate>,
    /// Audit log attached to the capability gate. `None` for the default
    /// `Runtime::new()` (DummyCapabilityGate has nothing to audit); set
    /// when the runtime is built via `new_from_document`.
    pub audit: Option<Rc<AuditLog>>,
    pub permissions: Rc<dyn PermissionBroker>,
    /// Tier-3 logic provider — how `call` actions dispatch. Null by
    /// default; hosts override with `set_logic_provider`.
    pub logic: Rc<dyn crate::logic::LogicProvider>,
    #[cfg(test)]
    fail_next_loader: bool,
    #[cfg(debug_assertions)]
    fail_next_variant_build: Cell<bool>,
}

/// A host-reportable authored action outcome with the source that scheduled it.
pub struct ReportedActionOutcome {
    pub outcome: ExecOutcome,
    pub source: Option<String>,
}

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
            ws_sessions: Rc::new(RefCell::new(std::collections::HashMap::new())),
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
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new_from_document(schema: PenDocument) -> CoreResult<Self> {
        let scheduler = Rc::new(Scheduler::new());
        let effects = EffectRegistry::new();
        effects.install_on(&scheduler);

        let prepared = prepare_document(schema, (800.0, 600.0), None);
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
            image_completions: Rc::new(RefCell::new(Vec::new())),
            image_requests: BTreeMap::new(),
            image_request_sources: BTreeMap::new(),
            image_document_dir: PathBuf::new(),
            viewport: Viewport::new(size(800.0, 600.0)),
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
            ws_sessions: Rc::new(RefCell::new(std::collections::HashMap::new())),
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
        runtime.state.set_viewport(800.0, 600.0, 1.0);
        runtime.admit_document_images();
        Ok(runtime)
    }

    pub fn load_str(&mut self, src: &str) -> CoreResult<()> {
        let schema = load_str(src)?.value;
        self.replace_document(schema)
    }

    /// Atomically hot-reload a document together with the layout/spatial data
    /// the host will immediately consume. All fallible parsing, loading,
    /// measurement, and constraint work completes against detached state
    /// before the live document, tasks, or image ownership are changed.
    pub fn load_str_and_relayout(&mut self, src: &str) -> CoreResult<()> {
        let schema = load_str(src)?.value;
        let preferred_path = self.active_screen_path.clone();
        self.replace_document_for_path_mode(schema, preferred_path.as_deref(), None, true, true)
    }

    pub fn configure_variants(
        &mut self,
        path: impl Into<String>,
        table: jian_ops_schema::screen_projection::ScreenVariantTable,
    ) {
        self.active_screen_path = Some(path.into());
        self.variant_table = table;
    }

    pub fn configure_variant_source(
        &mut self,
        source: PenDocument,
        path: impl Into<String>,
        table: jian_ops_schema::screen_projection::ScreenVariantTable,
    ) {
        self.variant_source = Some(source);
        self.configure_variants(path, table);
        if let Some(page_id) = self
            .document
            .as_ref()
            .and_then(|document| document.active_page.clone())
        {
            self.active_variant_page_id = Some(page_id.clone());
            self.active_page_key = page_id.clone();
            self.widget_states.set_page_key(page_id);
        }
    }

    pub fn selected_variant(&self) -> Option<&str> {
        self.active_variant_page_id.as_deref()
    }

    pub fn active_page_key(&self) -> &str {
        &self.active_page_key
    }

    /// Current projected screen path, when the document defines screens.
    pub fn active_screen_path(&self) -> Option<&str> {
        self.active_screen_path.as_deref()
    }

    /// Clone the projected route table for a host-owned router.
    pub fn screen_table(&self) -> Option<crate::screens::ScreenTable> {
        if let Some(source) = self.variant_source.clone() {
            crate::screens::ScreenTable::from_projected(source, self.variant_table.clone())
        } else {
            crate::screens::ScreenTable::from_document(self.document.as_ref()?.schema.clone())
        }
    }

    /// Changes whenever the mounted document's derived action set changes.
    pub fn action_surface_generation(&self) -> u64 {
        self.action_surface_generation
    }

    pub fn begin_ime_handshake(&mut self, snapshot: ImeSnapshot) -> u64 {
        self.ime_registry.issue(snapshot)
    }

    pub fn confirm_ime_commit(&mut self, request_id: u64, text: &str) -> ImeConfirmOutcome {
        let outcome = self
            .ime_registry
            .confirm_commit(request_id, text, &mut self.widget_states);
        if outcome != ImeConfirmOutcome::NoOp {
            self.complete_parked_after_ime(request_id);
        }
        outcome
    }

    pub fn confirm_ime_cancel(&mut self, request_id: u64) -> ImeConfirmOutcome {
        let outcome = self
            .ime_registry
            .confirm_cancel(request_id, &mut self.widget_states);
        if outcome != ImeConfirmOutcome::NoOp {
            self.complete_parked_after_ime(request_id);
        }
        outcome
    }

    fn active_ime_snapshot(&self) -> Option<ImeSnapshot> {
        self.widget_states.iter().find_map(|(node_id, state)| {
            let crate::widget_state::WidgetState::TextInput(field) = state else {
                return None;
            };
            let composition = field.composition()?;
            let region = composition
                .region
                .unwrap_or_else(|| (field.caret(), field.caret()));
            Some(ImeSnapshot {
                field_key: (self.active_page_key.clone(), node_id.to_owned()),
                region,
                text: composition.text.clone(),
                durable_text: field.text().to_owned(),
            })
        })
    }

    pub fn needs_variant_swap(&self, new_width: f32) -> Option<String> {
        let path = self.active_screen_path.as_deref()?;
        let variants = self.variant_table.0.get(path)?;
        let selected = variants
            .ranged
            .iter()
            .find(|entry| {
                entry.range.min_width.unwrap_or(0.0) as f32 <= new_width
                    && new_width <= entry.range.max_width.unwrap_or(f64::INFINITY) as f32
            })
            .map_or(variants.default_page_id.as_str(), |entry| {
                entry.page_id.as_str()
            });
        (self.active_variant_page_id.as_deref() != Some(selected)).then(|| selected.to_owned())
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
        let preferred_path = self.active_screen_path.clone();
        self.replace_document_for_path_mode(schema, preferred_path.as_deref(), None, true, false)
    }

    pub fn cancel_all_tasks(&mut self) {
        self.task_queue.cancel_all();
        self.ws_receive_active.clear();
        self.ws_messages.borrow_mut().clear();
        self.image_requests.clear();
        self.image_completions.borrow_mut().clear();
        self.state.storage_cache.cancel_hydrations();
        self.document_generation = self.document_generation.wrapping_add(1);
    }

    fn cancel_non_image_tasks_for_reload(&mut self) {
        let retained = self.reload_retained_task_ids();
        self.task_queue.cancel_all_except(&retained);
        self.ws_receive_active.clear();
        self.ws_messages.borrow_mut().clear();
        self.state.storage_cache.cancel_hydrations();
        self.document_generation = self.document_generation.wrapping_add(1);
    }

    fn reload_retained_task_ids(&self) -> BTreeSet<u64> {
        self.image_requests
            .values()
            .map(|request| request.task_id)
            .collect()
    }

    fn transfer_reload_image_requests(&mut self) {
        let generation = self.document_generation;
        for (key, request) in &self.image_requests {
            if self.image_store.state(key) == Some(crate::render::image_store::ImageState::Pending)
            {
                request.owner_generation.set(generation);
                // A ready resolver may already have left TaskQueue and queued
                // its completion. The shared owner cell still safely
                // transfers that completion even when there is no task left
                // to retag.
                self.task_queue.retag_task(request.task_id, generation);
            }
        }
        let stale: Vec<String> = self
            .image_requests
            .iter()
            .filter(|(key, _)| {
                self.image_store.state(key) != Some(crate::render::image_store::ImageState::Pending)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in stale {
            if let Some(request) = self.image_requests.remove(&key) {
                self.task_queue.cancel_task(request.task_id);
            }
        }
    }

    pub(crate) fn replace_document_for_path(
        &mut self,
        schema: PenDocument,
        preferred_path: Option<&str>,
        route: &RouteState,
    ) -> CoreResult<()> {
        self.replace_document_for_path_mode(schema, preferred_path, Some(route), false, false)
    }

    pub(crate) fn replace_document_for_path_and_relayout(
        &mut self,
        schema: PenDocument,
        preferred_path: Option<&str>,
        route: &RouteState,
    ) -> CoreResult<()> {
        self.replace_document_for_path_mode(schema, preferred_path, Some(route), false, true)
    }

    fn replace_document_for_path_mode(
        &mut self,
        mut schema: PenDocument,
        preferred_path: Option<&str>,
        candidate_route: Option<&RouteState>,
        conform_reload: bool,
        install_layout: bool,
    ) -> CoreResult<()> {
        let route_snapshot = conform_reload.then(|| self.nav.current());
        let reload_declaration_schema = schema.clone();
        let prepared = prepare_document(
            schema,
            (self.viewport.size.width, self.viewport.size.height),
            preferred_path,
        );
        let declaration_source = &reload_declaration_schema;
        let page_declarations: BTreeMap<String, jian_ops_schema::state::StateSchema> =
            declaration_source
                .pages
                .as_ref()
                .into_iter()
                .flatten()
                .filter_map(|page| page.state.clone().map(|state| (page.id.clone(), state)))
                .collect();
        fn collect_self_declarations(
            value: &serde_json::Value,
            page_key: &str,
            output: &mut BTreeMap<(String, String), jian_ops_schema::state::StateSchema>,
        ) {
            match value {
                serde_json::Value::Object(map) => {
                    if map.get("type").and_then(|value| value.as_str()).is_some() {
                        if let (Some(id), Some(state)) =
                            (map.get("id").and_then(|v| v.as_str()), map.get("state"))
                        {
                            if let Ok(schema) = serde_json::from_value(state.clone()) {
                                output.insert((page_key.to_owned(), id.to_owned()), schema);
                            }
                        }
                    }
                    if let Some(children) = map.get("children") {
                        collect_self_declarations(children, page_key, output);
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        collect_self_declarations(value, page_key, output);
                    }
                }
                _ => {}
            }
        }
        let mut self_declarations = BTreeMap::new();
        if let Ok(children) = serde_json::to_value(&declaration_source.children) {
            collect_self_declarations(&children, "", &mut self_declarations);
        }
        if let Some(pages) = declaration_source.pages.as_ref() {
            for page in pages {
                if let Ok(children) = serde_json::to_value(&page.children) {
                    collect_self_declarations(&children, &page.id, &mut self_declarations);
                }
            }
        }
        schema = prepared.mounted;
        let responsive = schema.is_responsive();
        let valid_paths: Vec<String> = schema.routes.as_ref().map_or_else(Vec::new, |routes| {
            std::iter::once(routes.entry.clone())
                .chain(
                    routes
                        .routes
                        .keys()
                        .filter(|path| *path != &routes.entry)
                        .cloned(),
                )
                .collect()
        });
        let declared_state = schema.state.clone().unwrap_or_default();
        let staged_defaults: BTreeMap<String, serde_json::Value> = declared_state
            .iter()
            .map(|(key, entry)| {
                (
                    key.clone(),
                    entry.default.clone().unwrap_or(serde_json::Value::Null),
                )
            })
            .collect();
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
        let storage_allowed = declared.contains(&crate::action::Capability::Storage);
        let capabilities = Rc::new(DeclaredCapabilityGate::new(declared, Some(audit.clone())));

        // Loader seeding is fallible and mutating. Build against a detached
        // graph so failure cannot alter responsive mode or any live scope.
        let staged_state = Rc::new(StateGraph::new(Rc::new(Scheduler::new())));
        staged_state.set_responsive(responsive);
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_loader) {
            return Err(CoreError::Layout("injected loader failure".into()));
        }
        let doc = loader::build_with(schema, &staged_state, loader::SeedMode::Initial)?;
        let staged_vars = staged_state.vars_snapshot();
        let page_key = if responsive {
            prepared.selected_page_id.as_deref().unwrap_or_default()
        } else {
            ""
        };
        // Preview every registered cancellation compensation against a
        // detached copy. The one fallible geometry build then sees exactly
        // the state that task cancellation will commit, while failed reloads
        // leave live futures and their authored loading flags untouched.
        let cancellation_state = Rc::new(StateGraph::new(Rc::new(Scheduler::new())));
        cancellation_state.set_responsive(responsive);
        copy_layout_scopes(&self.state, &cancellation_state, storage_allowed);
        if conform_reload {
            let retained = self.reload_retained_task_ids();
            self.task_queue
                .preview_cancel_compensations_except(&retained, &cancellation_state);
        }
        let live_state = cancellation_state.app_snapshot();
        let (merged_state, mut conformance_warnings) =
            crate::state::conformance::merge_scope(&live_state, &staged_defaults, &declared_state);
        let mut page_merges = Vec::new();
        let mut self_merges = Vec::new();
        if conform_reload {
            // Union of newly declared keys and RETAINED live keys: a page
            // whose `state` declaration disappeared must still be merged —
            // against an empty declaration, which prunes its stale fields.
            let empty_page_schema = jian_ops_schema::state::StateSchema::default();
            let mut page_keys: Vec<String> = page_declarations.keys().cloned().collect();
            for key in self.state.page_keys() {
                if !page_declarations.contains_key(&key) {
                    page_keys.push(key);
                }
            }
            for page_key in &page_keys {
                let page_schema = page_declarations
                    .get(page_key)
                    .unwrap_or(&empty_page_schema);
                let staged: BTreeMap<String, serde_json::Value> = page_schema
                    .iter()
                    .map(|(name, entry)| {
                        (
                            name.clone(),
                            entry.default.clone().unwrap_or(serde_json::Value::Null),
                        )
                    })
                    .collect();
                let (merged, warnings) = crate::state::conformance::merge_scope(
                    &cancellation_state.page_snapshot(page_key),
                    &staged,
                    page_schema,
                );
                conformance_warnings.extend(
                    warnings
                        .into_iter()
                        .map(|warning| format!("$page[{page_key}]: {warning}")),
                );
                page_merges.push((page_key.clone(), merged));
            }
            let empty_self_schema = jian_ops_schema::state::StateSchema::default();
            let mut self_keys: Vec<(String, String)> = self_declarations.keys().cloned().collect();
            for key in self.state.self_keys() {
                if !self_declarations.contains_key(&key) {
                    self_keys.push(key);
                }
            }
            for (page_key, node_id) in &self_keys {
                let declared = self_declarations
                    .get(&(page_key.clone(), node_id.clone()))
                    .unwrap_or(&empty_self_schema);
                let staged: BTreeMap<String, serde_json::Value> = declared
                    .iter()
                    .map(|(name, entry)| {
                        (
                            name.clone(),
                            entry.default.clone().unwrap_or(serde_json::Value::Null),
                        )
                    })
                    .collect();
                let (merged, warnings) = crate::state::conformance::merge_scope(
                    &cancellation_state.self_snapshot(page_key, node_id),
                    &staged,
                    declared,
                );
                conformance_warnings.extend(
                    warnings
                        .into_iter()
                        .map(|warning| format!("$self[{page_key}/{node_id}]: {warning}")),
                );
                self_merges.push((page_key.clone(), node_id.clone(), merged));
            }
        }

        let committed_route = candidate_route.map_or_else(
            || {
                route_snapshot.as_ref().map_or_else(
                    || self.state.route_snapshot(),
                    |route| normalized_route_values(route, &valid_paths),
                )
            },
            route_values,
        );
        if conform_reload {
            copy_layout_scopes(&cancellation_state, &staged_state, storage_allowed);
            staged_state.replace_app(&merged_state);
            staged_state.replace_vars(&staged_vars);
            for (page_key, values) in &page_merges {
                staged_state.replace_page(page_key, values);
            }
            for (page_key, node_id, values) in &self_merges {
                staged_state.replace_self(page_key, node_id, values);
            }
            staged_state.replace_route(&committed_route);
        } else {
            copy_layout_scopes(&self.state, &staged_state, storage_allowed);
            staged_state.replace_route(&committed_route);
        }
        let staged_geometry = if install_layout {
            Some(self.stage_document_geometry(
                &doc,
                &staged_state,
                page_key,
                (self.viewport.size.width, self.viewport.size.height),
            )?)
        } else {
            None
        };

        if conform_reload {
            let closing_sessions: Vec<_> = self
                .ws_sessions
                .borrow_mut()
                .drain()
                .map(|(_, handle)| handle.session)
                .collect();
            self.cancel_non_image_tasks_for_reload();
            for session in closing_sessions {
                self.task_queue.spawn_future(
                    async move {
                        let result = session
                            .close()
                            .await
                            .map_err(crate::action::ActionError::Custom);
                        ExecOutcome {
                            result,
                            warnings: Vec::new(),
                        }
                    },
                    self.document_generation,
                    Some("websocket:reload-close".into()),
                );
            }
        }

        if conform_reload {
            self.state.replace_app(&merged_state);
            self.state.replace_vars(&staged_vars);
            for (page_key, values) in &page_merges {
                self.state.replace_page(page_key, values);
            }
            for (page_key, node_id, values) in &self_merges {
                self.state.replace_self(page_key, node_id, values);
            }
            if !storage_allowed {
                self.state.replace_storage(&BTreeMap::new());
                self.state.storage_cache.purge();
            }
        }
        self.state.replace_route(&committed_route);
        self.state.set_responsive(responsive);
        let action_surface_inputs =
            crate::action_surface::derive_actions(&doc.schema, &crate::action_surface::BUILD_SALT);
        let focus_chain = collect_focus_chain(&doc);
        self.audit = Some(audit);
        self.capabilities = capabilities;
        if let Some(route_snapshot) = route_snapshot {
            self.nav.restore(route_snapshot, &valid_paths);
        }
        self.action_surface_inputs = action_surface_inputs;
        self.action_surface_generation = self.action_surface_generation.wrapping_add(1);
        self.load_warnings = prepared.warnings;
        if conform_reload {
            self.load_warnings.extend(conformance_warnings);
        }
        self.variant_source = prepared.source;
        self.variant_table = prepared.variants;
        self.active_screen_path = prepared.path;
        self.active_variant_page_id = prepared.selected_page_id.clone();
        self.active_page_key = if doc.schema.is_responsive() {
            prepared.selected_page_id.unwrap_or_default()
        } else {
            String::new()
        };
        self.widget_states
            .set_page_key(self.active_page_key.clone());
        self.image_store.begin_reload_ownership();
        self.document = Some(doc);
        if let Some((layout, spatial, layout_warnings)) = staged_geometry {
            self.layout.install(layout);
            self.spatial = spatial;
            for warning in layout_warnings {
                if !self.load_warnings.contains(&warning) {
                    self.load_warnings.push(warning);
                }
            }
            self.layout_mutation_seen = self.mutation_counter.get();
            self.mark_dirty();
        }
        // Preserve widget runtime state for ids that still exist in the
        // swapped-in tree; drop state for nodes that vanished.
        if let Some(doc) = self.document.as_ref() {
            self.widget_states
                .retain_ids(&|id| doc.tree.get(id).is_some());
            self.widget_states.revalidate(doc, &self.state);
        }
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
        self.state.clear_image_keys();
        self.admit_document_images();
        self.image_store.finish_reload_ownership();
        if conform_reload {
            self.transfer_reload_image_requests();
        }
        self.image_request_sources
            .retain(|key, _| self.image_store.state(key).is_some());
        Ok(())
    }

    fn stage_document_geometry(
        &self,
        live_doc: &RuntimeDocument,
        state: &StateGraph,
        page_key: &str,
        available: (f32, f32),
    ) -> CoreResult<(StagedLayout, SpatialIndex, Vec<String>)> {
        let responsive = live_doc.schema.is_responsive();
        let mut materialized;
        let doc = if responsive {
            materialized = live_doc.clone();
            for (_, node) in materialized.tree.nodes.iter_mut() {
                crate::binding::materialize_layout_bindings(
                    &mut node.schema,
                    state,
                    Some(page_key),
                );
            }
            &materialized
        } else {
            live_doc
        };
        let mut staged = self.layout.build_staged(doc)?;
        let mut warnings = Vec::new();
        if responsive {
            warnings.extend(staged.engine.constraint_lints().iter().cloned());
            if let Some(root) = select_viewport_root(&doc.tree, &mut warnings) {
                if root_has_limits(&doc.tree.nodes[root].schema) {
                    warnings.push("responsive viewport root min/max bounds are ignored".to_owned());
                }
                staged.engine.override_root_for_viewport(root, available)?;
            }
            for root in staged.roots.iter().copied() {
                staged.engine.compute_responsive(root, available)?;
            }
        } else {
            for root in staged.roots.iter().copied() {
                staged.engine.compute(root, available)?;
            }
        }
        let items: Vec<NodeBBox> = doc
            .tree
            .nodes
            .iter()
            .filter(|(_, node)| {
                serde_json::to_value(&node.schema)
                    .ok()
                    .and_then(|json| json.get("visible").and_then(serde_json::Value::as_bool))
                    .unwrap_or(true)
            })
            .filter_map(|(key, _)| {
                staged
                    .engine
                    .node_scene_rect(doc, key)
                    .map(|rect| NodeBBox { key, rect })
            })
            .collect();
        let mut spatial = SpatialIndex::new();
        spatial.rebuild(items);
        Ok((staged, spatial, warnings))
    }

    pub fn build_layout(&mut self, available: (f32, f32)) -> CoreResult<()> {
        let responsive = self
            .document
            .as_ref()
            .expect("no document loaded")
            .schema
            .is_responsive();
        if responsive {
            self.viewport.size = size(available.0, available.1);
            self.state.set_viewport(available.0, available.1, 1.0);
        }
        let live_doc = self.document.as_ref().expect("no document loaded");
        let mut materialized;
        let doc = if responsive {
            materialized = live_doc.clone();
            for (_, node) in materialized.tree.nodes.iter_mut() {
                crate::binding::materialize_layout_bindings(
                    &mut node.schema,
                    &self.state,
                    Some(&self.active_page_key),
                );
            }
            &materialized
        } else {
            live_doc
        };
        let mut staged = self.layout.build_staged(doc)?;
        if !responsive {
            for root in staged.roots.iter().copied() {
                staged.engine.compute(root, available)?;
            }
        } else {
            for warning in staged.engine.constraint_lints().to_vec() {
                if !self.load_warnings.contains(&warning) {
                    self.load_warnings.push(warning);
                }
            }
            let viewport_root = select_viewport_root(&doc.tree, &mut self.load_warnings);
            if let Some(root_key) = viewport_root {
                if root_has_limits(&doc.tree.nodes[root_key].schema) {
                    let warning = "responsive viewport root min/max bounds are ignored".to_owned();
                    if !self.load_warnings.contains(&warning) {
                        self.load_warnings.push(warning);
                    }
                }
                staged
                    .engine
                    .override_root_for_viewport(root_key, available)?;
            }
            for root in staged.roots.iter().copied() {
                staged.engine.compute_responsive(root, available)?;
            }
        }

        let items: Vec<NodeBBox> = doc
            .tree
            .nodes
            .iter()
            .filter(|(_, node)| {
                serde_json::to_value(&node.schema)
                    .ok()
                    .and_then(|json| json.get("visible").and_then(|value| value.as_bool()))
                    .unwrap_or(true)
            })
            .filter_map(|(key, _)| {
                staged
                    .engine
                    .node_scene_rect(doc, key)
                    .map(|rect| NodeBBox { key, rect })
            })
            .collect();
        let focused_became_hidden = self.focus.current().is_some_and(|focused| {
            doc.tree.nodes.get(focused).is_some_and(|node| {
                serde_json::to_value(&node.schema)
                    .ok()
                    .and_then(|json| json.get("visible").and_then(|value| value.as_bool()))
                    == Some(false)
            })
        });
        let mut spatial = SpatialIndex::new();
        spatial.rebuild(items);
        self.layout.install(staged);
        self.spatial = spatial;
        if focused_became_hidden {
            self.focus.clear();
        }
        self.layout_mutation_seen = self.mutation_counter.get();
        self.mark_dirty();
        Ok(())
    }

    /// Rebuild geometry at the current host-truth viewport. Both layout and
    /// spatial indexes are committed together by [`Self::build_layout`].
    pub fn relayout(&mut self) -> CoreResult<()> {
        self.build_layout((self.viewport.size.width, self.viewport.size.height))
    }

    pub fn prepare_frame(
        &mut self,
        backend: &mut impl crate::render::RenderBackend,
        backend_generation: u64,
    ) {
        let changed = self.image_store.has_pending_work();
        for warning in self.image_store.prepare_frame(backend, backend_generation) {
            self.load_warnings.push(warning);
        }
        // Dirty only when the store actually transitioned something —
        // unconditional dirtying would defeat the pump idle contract
        // (needs_paint would be true on every frame forever).
        if changed {
            self.mark_dirty();
        }
    }

    pub fn set_image_document_dir(&mut self, directory: impl Into<PathBuf>) {
        self.image_document_dir = directory.into();
        self.state.clear_image_keys();
        self.admit_document_images();
    }

    pub fn admit_document_images(&mut self) {
        let Some(doc) = self.document.as_ref() else {
            return;
        };
        let mut found = Vec::new();
        for (_, node) in doc.tree.nodes.iter() {
            if let jian_ops_schema::node::PenNode::Image(image) = &node.schema {
                found.push(image.src.as_ref().to_owned());
            }
            if let Ok(json) = serde_json::to_value(&node.schema) {
                if let Some(fills) = json.get("fill").cloned().and_then(|value| {
                    serde_json::from_value::<Vec<jian_ops_schema::style::PenFill>>(value).ok()
                }) {
                    for fill in fills {
                        if let jian_ops_schema::style::PenFill::Image(image) = fill {
                            found.push(image.url.as_ref().to_owned());
                        }
                    }
                }
            }
        }
        for source in found {
            if !source.starts_with("data:") {
                match self.image_resolver.admission(&source) {
                    Ok(Some(admission)) => {
                        self.state.set_image_key(&source, &admission.key);
                        self.image_request_sources
                            .insert(admission.key.clone(), admission.request_source);
                        if !admission.requires_network
                            || self.capabilities.check(
                                crate::action::Capability::Network,
                                "image_resolve",
                                self.now_ms,
                            )
                        {
                            self.image_store
                                .admit_resolver(&admission.key, 64 * 1024 * 1024);
                        } else {
                            self.image_store.admit_resolver(&admission.key, 0);
                            if self.image_store.state(&admission.key)
                                != Some(crate::render::image_store::ImageState::Registered)
                            {
                                self.image_store
                                    .fail(&admission.key, "network capability denied");
                                self.load_warnings.push(format!(
                                    "image `{}`: network capability denied",
                                    admission.key
                                ));
                            }
                        }
                        continue;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.load_warnings
                            .push(format!("image `{source}`: {error}"));
                        continue;
                    }
                }
            }
            let key = match crate::render::image_store::canonical_url_key(
                &source,
                &self.image_document_dir,
            ) {
                Ok(key) => key,
                Err(error) => {
                    self.load_warnings.push(error);
                    continue;
                }
            };
            self.state.set_image_key(&source, &key);
            if source.starts_with("data:") {
                match crate::render::image_store::decode_data_url(&source) {
                    Ok(bytes) => self.image_store.admit_inline(&key, bytes),
                    Err(error) => {
                        self.image_store.admit_resolver(&key, 0);
                        self.image_store.fail(&key, &error);
                        self.load_warnings.push(format!("image `{key}`: {error}"));
                    }
                }
            } else if source.starts_with("http://") || source.starts_with("https://") {
                if self.capabilities.check(
                    crate::action::Capability::Network,
                    "image_resolve",
                    self.now_ms,
                ) {
                    self.image_store.admit_resolver(&key, 64 * 1024 * 1024);
                } else {
                    self.image_store.admit_resolver(&key, 0);
                    if self.image_store.state(&key)
                        != Some(crate::render::image_store::ImageState::Registered)
                    {
                        self.image_store.fail(&key, "network capability denied");
                        self.load_warnings
                            .push(format!("image `{key}`: network capability denied"));
                    }
                }
            } else {
                let path = Path::new(&key);
                match crate::render::image_store::read_confined_local(
                    path,
                    &self.image_document_dir,
                ) {
                    Ok(bytes) => self.image_store.admit_inline(&key, bytes),
                    Err(error) => {
                        self.image_store.admit_resolver(&key, 0);
                        self.image_store.fail(&key, &error);
                        self.load_warnings.push(format!("image `{key}`: {error}"));
                    }
                }
            }
        }
    }

    pub(crate) fn dispatch_image_requests(&mut self) {
        for key in self.image_store.pending_keys() {
            if self.image_requests.contains_key(&key) {
                continue;
            }
            let resolver = self.image_resolver.clone();
            let completions = self.image_completions.clone();
            let request_key = key.clone();
            let owner_generation = Rc::new(Cell::new(self.document_generation));
            let completion_owner = owner_generation.clone();
            let request_source = self
                .image_request_sources
                .get(&key)
                .cloned()
                .unwrap_or_else(|| key.clone());
            let task_id = self.task_queue.spawn_future(
                async move {
                    let result = resolver.resolve(&request_source).await;
                    completions.borrow_mut().push(ImageCompletion {
                        key: request_key,
                        owner_generation: completion_owner,
                        result,
                    });
                    ExecOutcome {
                        result: Ok(()),
                        warnings: Vec::new(),
                    }
                },
                self.document_generation,
                Some(format!("image:{key}")),
            );
            self.image_requests.insert(
                key,
                ImageRequest {
                    task_id,
                    owner_generation,
                },
            );
        }
    }

    pub fn load_warnings(&self) -> &[String] {
        &self.load_warnings
    }

    /// Drain authored action results completed since the previous call.
    ///
    /// Hosts use this to report every action diagnostic and failure without
    /// coupling JS/native callbacks to runtime dispatch borrows.
    pub fn take_action_outcomes(&mut self) -> Vec<ReportedActionOutcome> {
        std::mem::take(&mut self.action_outcomes)
    }

    /// Drain layout failures caught by asynchronous/host-driven runtime paths.
    /// The live layout remains installed because every layout build is staged.
    pub fn take_layout_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.layout_errors)
    }

    /// Enable retention of action warnings and failures for a reporting host.
    /// Disabled by default so native hosts that do not install callbacks keep
    /// the historical discard behavior and cannot accumulate diagnostics.
    pub fn enable_action_reporting(&mut self) {
        self.action_reporting_enabled = true;
    }

    /// Record a host-visible runtime warning for the normal warning stream.
    pub fn push_load_warning(&mut self, warning: impl Into<String>) {
        self.load_warnings.push(warning.into());
    }

    /// Queue a host-visible layout error without degrading it to a warning.
    pub fn push_layout_error(&mut self, error: impl Into<String>) {
        self.layout_errors.push(error.into());
    }

    fn collect_task_outcomes(&mut self) -> bool {
        let outcomes = self.task_queue.poll_all(self.now_ms);
        let completed = !outcomes.is_empty();
        if self.action_reporting_enabled {
            self.action_outcomes
                .extend(outcomes.into_iter().filter_map(|completed| {
                    (completed.outcome.result.is_err() || !completed.outcome.warnings.is_empty())
                        .then_some(ReportedActionOutcome {
                            outcome: completed.outcome,
                            source: completed.source,
                        })
                }));
        }
        completed
    }

    /// Canonical live logical viewport used by responsive selection/reload.
    pub fn set_viewport_size(&mut self, viewport: (f32, f32)) {
        if (self.viewport.size.width, self.viewport.size.height) != viewport {
            self.viewport.size = size(viewport.0, viewport.1);
            self.state.set_viewport(viewport.0, viewport.1, 1.0);
            self.scheduler.flush();
            self.mutation_counter
                .set(self.mutation_counter.get().wrapping_add(1));
            self.mark_dirty();
            if self
                .document
                .as_ref()
                .is_some_and(|document| document.schema.is_responsive())
            {
                if let Err(error) = self.relayout() {
                    self.push_layout_error(format!("viewport relayout failed: {error}"));
                }
            }
        }
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
                    .node_scene_rect(doc, key)
                    .map(|rect| NodeBBox { key, rect })
            })
            .collect();
        self.spatial.rebuild(items);
    }

    /// Runtime/host geometry in the same absolute scene coordinate space as
    /// draw ops and pointer events.
    pub fn node_scene_rect(&self, key: crate::document::NodeKey) -> Option<crate::geometry::Rect> {
        let doc = self.document.as_ref()?;
        self.layout.node_scene_rect(doc, key)
    }

    pub fn focused_node_rect(&self) -> Option<crate::geometry::Rect> {
        self.focus
            .current()
            .and_then(|key| self.node_scene_rect(key))
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
            let Some(rect) = self.layout.node_scene_rect(doc, key) else {
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
        self.note_time(event.t_ms);
        if self.input_frozen() {
            return Vec::new();
        }
        if self.document.is_none() {
            return Vec::new();
        }
        // Slider drag is handled directly off the raw pointer phases
        // (the gesture arena only surfaces Tap on Down+Up): Down over a
        // slider arms a drag, Move scrubs the value, Up disarms it. This
        // runs *before* the arena dispatch so a drag and a tap don't
        // double-set the value — a clean Down+Up still lands as a Tap.
        let (phase, position) = (event.phase, event.position);
        self.handle_slider_drag(phase, position);

        let emitted = {
            let doc = self.document.as_ref().unwrap();
            self.gestures.dispatch(event, doc, &self.spatial)
        };
        // A tap on an interactive widget focuses it and performs its
        // primary action (toggle / slider set-by-x) before the generic
        // onTap action dispatch.
        for ev in &emitted {
            if let SemanticEvent::Tap { node, position } = ev {
                self.activate_widget_on_tap(*node, *position);
            }
        }
        for ev in &emitted {
            self.dispatch_semantic(ev);
        }
        emitted
    }

    /// Pointer-phase driven slider scrubbing. On `Down` over a slider,
    /// focus it and arm the drag (`Slider.dragging = true`). On `Move`
    /// while any slider is armed, set that slider's value from x and
    /// sync its `bind:value`. On `Up`, disarm every slider. No-op when
    /// no slider is under the cursor / armed.
    fn handle_slider_drag(
        &mut self,
        phase: crate::gesture::pointer::PointerPhase,
        position: crate::geometry::Point,
    ) {
        use crate::gesture::pointer::PointerPhase;
        use crate::widget_state::WidgetState;
        use jian_ops_schema::node::PenNode;

        match phase {
            PointerPhase::Down => {
                // Topmost hit node that is a slider arms a drag.
                let Some(doc) = self.document.as_ref() else {
                    return;
                };
                let hit = crate::gesture::hit::hit_test(&self.spatial, doc, position);
                let slider = hit.0.iter().copied().find(|&k| {
                    matches!(
                        doc.tree.nodes.get(k).map(|n| &n.schema),
                        Some(PenNode::Slider(_))
                    )
                });
                if let Some(node) = slider {
                    let id = {
                        let schema = &doc.tree.nodes[node].schema;
                        crate::document::tree::node_schema_id(schema).to_owned()
                    };
                    let _ = self.focus_request(node);
                    self.with_widget_state(node, |st| {
                        if let WidgetState::Slider { dragging, .. } = st {
                            *dragging = true;
                        }
                        false
                    });
                    if self.set_slider_from_x(node, position.x) {
                        self.sync_widget_binding(&id);
                    }
                }
            }
            PointerPhase::Move => {
                // Find the id of whichever slider is currently armed, then
                // resolve its node key. Two steps so the widget-state read
                // and the document read don't overlap-borrow `self`.
                let armed_id = self.widget_states.iter().find_map(|(id, st)| {
                    matches!(st, WidgetState::Slider { dragging: true, .. }).then(|| id.to_owned())
                });
                let Some(id) = armed_id else { return };
                let node = self.document.as_ref().and_then(|doc| doc.tree.get(&id));
                if let Some(node) = node {
                    if self.set_slider_from_x(node, position.x) {
                        self.sync_widget_binding(&id);
                    }
                }
            }
            PointerPhase::Up => {
                // Disarm any armed slider.
                for st in self.widget_states.values_mut() {
                    if let WidgetState::Slider { dragging, .. } = st {
                        *dragging = false;
                    }
                }
            }
            _ => {}
        }
    }

    /// Tap-driven widget activation: focus the tapped widget and, for a
    /// switch/checkbox, flip it; for a slider, set its value from the
    /// tap x within the track. Syncs `bind:value` afterwards. Other
    /// widgets just take focus (text editing / popups come via keys).
    fn activate_widget_on_tap(
        &mut self,
        node: crate::document::NodeKey,
        position: crate::geometry::Point,
    ) {
        use crate::widget_state::WidgetState;
        use jian_ops_schema::node::PenNode;

        enum Act {
            Toggle,
            Slider,
            FocusOnly,
            NotWidget,
        }

        let (id, act) = {
            let Some(doc) = self.document.as_ref() else {
                return;
            };
            let Some(nd) = doc.tree.nodes.get(node) else {
                return;
            };
            let schema = &nd.schema;
            let id = crate::document::tree::node_schema_id(schema).to_owned();
            let act = match schema {
                PenNode::Switch(_) | PenNode::Checkbox(_) => Act::Toggle,
                PenNode::Slider(_) => Act::Slider,
                PenNode::TextInput(_)
                | PenNode::TextArea(_)
                | PenNode::NumberInput(_)
                | PenNode::Select(_)
                | PenNode::RadioGroup(_)
                | PenNode::Tabs(_) => Act::FocusOnly,
                _ => Act::NotWidget,
            };
            (id, act)
        };

        if matches!(act, Act::NotWidget) {
            return;
        }
        let _ = self.focus_request(node);

        let changed = match act {
            Act::Toggle => self.with_widget_state(node, |st| {
                if let WidgetState::Toggle { on } = st {
                    *on = !*on;
                    true
                } else {
                    false
                }
            }),
            Act::Slider => self.set_slider_from_x(node, position.x),
            Act::FocusOnly | Act::NotWidget => false,
        };
        if changed {
            self.sync_widget_binding(&id);
        }
    }

    /// Set a slider's value from a pointer x within its track, using the
    /// node's `min`/`max`/`step` and the same quantization the tap path
    /// uses. Returns `true` when the slider's runtime value changed.
    /// Shared by both the tap path and the drag path. Does NOT sync the
    /// `bind:value` target — the caller does that (so a drag can sync
    /// once per move without re-resolving the id here).
    fn set_slider_from_x(&mut self, node: crate::document::NodeKey, x: f32) -> bool {
        use crate::widget_state::WidgetState;
        use jian_ops_schema::node::PenNode;

        let Some(doc) = self.document.as_ref() else {
            return false;
        };
        let Some(nd) = doc.tree.nodes.get(node) else {
            return false;
        };
        let PenNode::Slider(s) = &nd.schema else {
            return false;
        };
        let (min, max, step) = (
            s.min.unwrap_or(0.0),
            s.max.unwrap_or(100.0),
            s.step.unwrap_or(1.0),
        );
        let Some(r) = self.node_scene_rect(node) else {
            return false;
        };
        let (min_x, width) = (r.min_x(), r.size.width);
        let frac = if width > 0.0 {
            (((x - min_x) / width) as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let raw = min + frac * (max - min);
        let quantized = if step > 0.0 {
            min + ((raw - min) / step).round() * step
        } else {
            raw
        };
        let v = quantized.clamp(min, max);
        self.with_widget_state(node, |st| {
            if let WidgetState::Slider { value, .. } = st {
                if (*value - v).abs() > f64::EPSILON {
                    *value = v;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        })
    }

    /// Run `f` against the lazily-seeded widget state for `node`.
    /// Returns `f`'s result, or `false` when the node has no state.
    fn with_widget_state(
        &mut self,
        node: crate::document::NodeKey,
        f: impl FnOnce(&mut crate::widget_state::WidgetState) -> bool,
    ) -> bool {
        let Some(doc) = self.document.as_ref() else {
            return false;
        };
        let Some(nd) = doc.tree.nodes.get(node) else {
            return false;
        };
        match self.widget_states.get_or_init(&nd.schema, &self.state) {
            Some(st) => f(st),
            None => false,
        }
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
        self.note_time(event.t_ms);
        if self.input_frozen() {
            return Vec::new();
        }
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
        if self.input_frozen() {
            return Vec::new();
        }
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
        if self.input_frozen() {
            return Vec::new();
        }
        if self.document.is_none() {
            return Vec::new();
        }
        let key = key.into();
        if key == "Tab" {
            if modifiers.contains(crate::gesture::pointer::Modifiers::SHIFT) {
                return self.focus_previous().unwrap_or_default();
            }
            return self.focus_next().unwrap_or_default();
        }
        // Route editing keys to the focused editable widget before the
        // generic semantic dispatch. Printable text + paste arrive via
        // `dispatch_text_input`; IME via `dispatch_ime`.
        let now = self.now_ms;
        let focused_id = self.focused_widget_id();
        let mut consumed = false;
        if let Some(st) = self.focused_text_state() {
            use crate::gesture::pointer::Modifiers;
            consumed = match key.as_str() {
                "Backspace" => {
                    st.backspace(now);
                    true
                }
                "Delete" => {
                    st.delete_forward(now);
                    true
                }
                "ArrowLeft" => {
                    st.move_left(modifiers.contains(Modifiers::SHIFT), now);
                    true
                }
                "ArrowRight" => {
                    st.move_right(modifiers.contains(Modifiers::SHIFT), now);
                    true
                }
                "Home" => {
                    st.move_home(modifiers.contains(Modifiers::SHIFT), now);
                    true
                }
                "End" => {
                    st.move_end(modifiers.contains(Modifiers::SHIFT), now);
                    true
                }
                "a" | "A"
                    if modifiers.contains(Modifiers::CMD)
                        || modifiers.contains(Modifiers::CTRL) =>
                {
                    st.select_all();
                    true
                }
                _ => false,
            };
        }
        // Non-text widgets: Space/Enter toggles switch+checkbox; arrows
        // step a slider. (Select/Radio/Tabs keyboard navigation lands
        // with their popup/list behaviour in Phase C.)
        if !consumed {
            if let Some(id) = focused_id.as_deref() {
                consumed = self.route_widget_action_key(id, key.as_str());
            }
        }
        if consumed {
            if let Some(id) = focused_id.as_deref() {
                self.sync_widget_binding(id);
            }
            return Vec::new();
        }
        let Some(target) = self.focus.current() else {
            return Vec::new();
        };
        self.dispatch_key(target, key, modifiers)
    }

    /// Keyboard behaviour for non-text widgets (toggle flip, slider
    /// step). Returns `true` when the key changed widget state. The
    /// caller syncs `bind:value` afterwards.
    fn route_widget_action_key(&mut self, id: &str, key: &str) -> bool {
        use crate::widget_state::WidgetState;
        // Node-derived inputs read before the mutable widget-state borrow.
        let (min, max, step) = self.slider_bounds(id);
        let options = self.widget_option_values(id);
        match self.widget_states.get_mut(id) {
            Some(WidgetState::Toggle { on }) => match key {
                "Enter" | " " | "Space" | "Spacebar" => {
                    *on = !*on;
                    true
                }
                _ => false,
            },
            Some(WidgetState::Slider { value, .. }) => {
                let new = match key {
                    "ArrowRight" | "ArrowUp" => (*value + step).min(max),
                    "ArrowLeft" | "ArrowDown" => (*value - step).max(min),
                    "Home" => min,
                    "End" => max,
                    _ => return false,
                };
                *value = new;
                true
            }
            // Select + radio: arrows cycle the selected option.
            Some(WidgetState::Select { value, .. }) | Some(WidgetState::Radio { value, .. }) => {
                match step_option(&options, value.as_deref(), key) {
                    Some(next) => {
                        *value = Some(next);
                        true
                    }
                    None => false,
                }
            }
            // Tabs: arrows cycle the active tab.
            Some(WidgetState::Tabs { active, .. }) => {
                match step_option(&options, active.as_deref(), key) {
                    Some(next) => {
                        *active = Some(next);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// Ordered `value`s of a node's `options` (select / radio) or `tabs`
    /// (tabs) list — for keyboard cycling. Empty when absent.
    fn widget_option_values(&self, id: &str) -> Vec<String> {
        let Some(node) = self
            .document
            .as_ref()
            .and_then(|d| d.tree.get(id).and_then(|k| d.tree.nodes.get(k)))
        else {
            return Vec::new();
        };
        let Ok(json) = serde_json::to_value(&node.schema) else {
            return Vec::new();
        };
        json.get("options")
            .or_else(|| json.get("tabs"))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|o| o.get("value").and_then(|v| v.as_str()).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `(min, max, step)` for a slider node, defaulting to `0 / 100 / 1`.
    fn slider_bounds(&self, id: &str) -> (f64, f64, f64) {
        use jian_ops_schema::node::PenNode;
        let node = self
            .document
            .as_ref()
            .and_then(|d| d.tree.get(id).map(|k| (d, k)))
            .and_then(|(d, k)| d.tree.nodes.get(k));
        if let Some(PenNode::Slider(s)) = node.map(|n| &n.schema) {
            (
                s.min.unwrap_or(0.0),
                s.max.unwrap_or(100.0),
                s.step.unwrap_or(1.0),
            )
        } else {
            (0.0, 100.0, 1.0)
        }
    }

    /// Update the host clock (ms). Drives widget caret-blink phase; the
    /// gesture pipeline keeps a separate `Instant` via [`Runtime::tick`].
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.note_time(now_ms);
    }

    pub fn note_time(&mut self, now_ms: u64) {
        self.now_ms = self.now_ms.max(now_ms);
        self.task_clock.advance_to(self.now_ms);
        self.state.set_now_ms(self.now_ms);
    }

    pub fn last_now_ms(&self) -> u64 {
        self.now_ms
    }

    /// `&mut TextInputState` for the currently-focused editable widget
    /// (text_input / text_area / number_input), or `None` when nothing
    /// editable is focused.
    fn focused_text_state(&mut self) -> Option<&mut crate::text_input::TextInputState> {
        let target = self.focus.current()?;
        let node = self.document.as_ref()?.tree.nodes.get(target)?;
        match self.widget_states.get_or_init(&node.schema, &self.state)? {
            crate::widget_state::WidgetState::TextInput(st) => Some(st),
            _ => None,
        }
    }

    /// Stable schema id of the currently-focused node, if any.
    fn focused_widget_id(&self) -> Option<String> {
        let key = self.focus.current()?;
        let node = self.document.as_ref()?.tree.nodes.get(key)?;
        Some(crate::document::tree::node_schema_id(&node.schema).to_owned())
    }

    /// Printable text from the host (keypress chars, paste). Routed to
    /// the focused editable widget; returns `true` when consumed.
    pub fn dispatch_text_input(&mut self, text: &str) -> CoreResult<bool> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        if text.is_empty() {
            return Ok(false);
        }
        let now = self.now_ms;
        let id = self.focused_widget_id();
        {
            let Some(st) = self.focused_text_state() else {
                return Ok(false);
            };
            st.insert_str(text, now);
        }
        if let Some(id) = id.as_deref() {
            self.sync_widget_binding(id);
        }
        Ok(true)
    }

    /// IME composition events (`gesture::ime::ImeEvent`). Routed to the
    /// focused editable widget; returns `true` when consumed.
    pub fn dispatch_ime(&mut self, ev: crate::gesture::ime::ImeEvent) -> CoreResult<bool> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        use crate::gesture::ime::ImeKind;
        let now = self.now_ms;
        let id = self.focused_widget_id();
        let committed;
        {
            let Some(st) = self.focused_text_state() else {
                return Ok(false);
            };
            match ev.kind {
                ImeKind::CompositionStart => {
                    st.set_composition(ev.text, 0, now);
                    committed = false;
                }
                ImeKind::CompositionUpdate { selection } => {
                    let cursor = selection.map(|r| r.end).unwrap_or(ev.text.len());
                    st.set_composition(ev.text, cursor, now);
                    committed = false;
                }
                ImeKind::CompositionEnd => {
                    // Replace any in-flight preedit with the final commit
                    // string, then fold it into the text.
                    let len = ev.text.len();
                    st.set_composition(ev.text, len, now);
                    st.commit_composition(now);
                    committed = true;
                }
            }
        }
        // Only a committed composition changes the bound value; an
        // in-flight preedit is overlay-only (paint reads `composition()`).
        if committed {
            if let Some(id) = id.as_deref() {
                self.sync_widget_binding(id);
            }
        }
        Ok(true)
    }

    /// Push the focused/edited widget's current value into the state
    /// graph via its `bindings["bind:value"]` target. No-op when the
    /// node has no `bind:value`, or it targets anything but a writable
    /// single-segment `$state.<key>` (multi-segment writes are not yet
    /// supported by the runtime — see `action::actions::state::write_path`).
    fn sync_widget_binding(&mut self, node_id: &str) {
        use crate::widget_state::WidgetState;
        // `number_input` shares `TextInput` edit state but binds a numeric
        // value — write a JSON number (empty/invalid → null), not a string.
        let numeric = self.widget_is_number_input(node_id);
        let value = match self.widget_states.get_mut(node_id) {
            Some(WidgetState::TextInput(st)) if numeric => st
                .text()
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Some(WidgetState::TextInput(st)) => serde_json::Value::String(st.text().to_owned()),
            Some(WidgetState::Toggle { on }) => serde_json::Value::Bool(*on),
            Some(WidgetState::Slider { value, .. }) => serde_json::json!(*value),
            Some(WidgetState::Select { value, .. }) | Some(WidgetState::Radio { value, .. }) => {
                value
                    .clone()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null)
            }
            Some(WidgetState::Tabs { active, .. }) => active
                .clone()
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
            None => return,
        };
        let Some(key) = self.widget_bind_key(node_id) else {
            return;
        };
        self.state.app_set(&key, value);
        self.scheduler.flush();
    }

    /// Resolve a widget's `bind:value` to a writable single-segment
    /// `$state.<key>` app key. Returns `None` for absent / computed /
    /// non-`$state` / multi-segment targets.
    fn widget_bind_key(&self, node_id: &str) -> Option<String> {
        let key = self.document.as_ref()?.tree.get(node_id)?;
        let node = self.document.as_ref()?.tree.nodes.get(key)?;
        let json = serde_json::to_value(&node.schema).ok()?;
        let raw = json.get("bindings")?.get("bind:value")?.as_str()?;
        let rest = raw.trim().strip_prefix("$state.")?;
        if rest.is_empty() || rest.contains(['.', '[']) || rest.contains(char::is_whitespace) {
            return None;
        }
        Some(rest.to_owned())
    }

    /// True when `node_id` is a `number_input` node (which binds a
    /// numeric value despite sharing `TextInput` edit state).
    fn widget_is_number_input(&self, node_id: &str) -> bool {
        self.document
            .as_ref()
            .and_then(|d| d.tree.get(node_id).map(|k| (d, k)))
            .and_then(|(d, k)| d.tree.nodes.get(k))
            .map(|n| matches!(n.schema, jian_ops_schema::node::PenNode::NumberInput(_)))
            .unwrap_or(false)
    }

    /// Move focus forward one step (`Tab`) and emit the corresponding
    /// `FocusLost` / `FocusGained` events.
    pub fn focus_next(&mut self) -> CoreResult<Vec<SemanticEvent>> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        let change = self.focus.next();
        Ok(self.emit_focus_change(change))
    }

    /// Move focus backward one step (`Shift+Tab`).
    pub fn focus_previous(&mut self) -> CoreResult<Vec<SemanticEvent>> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        let change = self.focus.previous();
        Ok(self.emit_focus_change(change))
    }

    /// Programmatically focus an explicit node. Hosts call this from
    /// click handlers (focus-on-click) or from `jian-action-surface`
    /// when an AI client requests a focus change.
    pub fn focus_request(
        &mut self,
        node: crate::document::NodeKey,
    ) -> CoreResult<Vec<SemanticEvent>> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        let change = self.focus.request(node);
        Ok(self.emit_focus_change(change))
    }

    /// Drop focus entirely — typically wired to clicking outside any
    /// focusable node, or to the window losing OS focus.
    pub fn focus_clear(&mut self) -> CoreResult<Vec<SemanticEvent>> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        let change = self.focus.clear();
        Ok(self.emit_focus_change(change))
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
        if self.input_frozen() {
            return 0;
        }
        let snapshot: Vec<_> = self
            .ws_sessions
            .borrow()
            .iter()
            .map(|(id, handle)| (id.clone(), handle.session.clone(), handle.generation))
            .collect();
        for (id, session, generation) in snapshot {
            if generation != self.document_generation || !self.ws_receive_active.insert(id.clone())
            {
                continue;
            }
            let messages = self.ws_messages.clone();
            let receive_id = id.clone();
            self.task_queue.spawn_future(
                async move {
                    let batch = session.receive().await;
                    messages.borrow_mut().push((receive_id, generation, batch));
                    ExecOutcome {
                        result: Ok(()),
                        warnings: Vec::new(),
                    }
                },
                generation,
                Some(format!("websocket:receive:{id}")),
            );
        }
        self.collect_task_outcomes();
        let received = std::mem::take(&mut *self.ws_messages.borrow_mut());
        let mut fired = 0usize;
        for (id, generation, messages) in received {
            self.ws_receive_active.remove(&id);
            if generation != self.document_generation {
                continue;
            }
            let handler_json = self
                .ws_sessions
                .borrow()
                .get(&id)
                .and_then(|handle| {
                    (handle.generation == generation).then(|| handle.on_message.clone())
                })
                .flatten();
            let Some(handler_json) = handler_json else {
                continue;
            };
            for msg in messages {
                if !self
                    .ws_sessions
                    .borrow()
                    .get(&id)
                    .is_some_and(|handle| handle.generation == generation)
                {
                    break;
                }
                let ctx = self.make_action_ctx_with_event(serde_json::json!({
                    "id": id,
                    "data": msg,
                }));
                if let Err(error) = self.task_queue.spawn(
                    &self.actions,
                    &handler_json,
                    ctx,
                    self.document_generation,
                    Some(format!("websocket:{id}")),
                ) {
                    if self.action_reporting_enabled {
                        self.action_outcomes.push(ReportedActionOutcome {
                            outcome: ExecOutcome {
                                result: Err(error),
                                warnings: Vec::new(),
                            },
                            source: Some(format!("websocket:{id}")),
                        });
                    }
                    continue;
                }
                self.collect_task_outcomes();
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
    pub fn tick(&mut self, now_ms: u64) -> Vec<SemanticEvent> {
        self.note_time(now_ms);
        let emitted = self.gestures.tick(self.now_ms);
        if self.input_frozen() {
            return Vec::new();
        }
        for ev in &emitted {
            self.dispatch_semantic(ev);
        }
        emitted
    }

    fn dispatch_semantic(&mut self, event: &SemanticEvent) {
        let (source_node_id, list) = {
            let doc = self.document.as_ref().expect("no document loaded");
            let source = doc
                .tree
                .nodes
                .get(event.node())
                .map(|node| crate::document::tree::node_schema_id(&node.schema).to_owned());
            (
                source,
                crate::gesture::dispatcher::resolve_handler(doc, event),
            )
        };
        let mut ctx = match event_payload(event) {
            Some(payload) => self.make_action_ctx_with_event(payload),
            None => self.make_action_ctx(),
        };
        ctx.node_id = source_node_id;
        if let Some(list) = list {
            match self.task_queue.spawn(
                &self.actions,
                &list,
                ctx,
                self.document_generation,
                Some(event.handler_key().to_owned()),
            ) {
                Ok(_) => {
                    self.collect_task_outcomes();
                }
                Err(error) if self.action_reporting_enabled => {
                    self.action_outcomes.push(ReportedActionOutcome {
                        outcome: ExecOutcome {
                            result: Err(error),
                            warnings: Vec::new(),
                        },
                        source: Some(event.handler_key().to_owned()),
                    });
                }
                Err(_) => {}
            }
        }
        // Actions mutate state via Signals whose effects are scheduled;
        // flush synchronously so bindings / scene observers see the new
        // values before the host's next frame.
        self.scheduler.flush();
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
            clock: Some(self.task_clock.clone()),
            document_generation: self.document_generation,
            event: None,
            locals: RefCell::new(BTreeMap::new()),
            page_id: Some(self.active_page_key.clone()),
            node_id: None,
            network: self.network.clone(),
            ws_sessions: self.ws_sessions.clone(),
            storage: self.storage.clone(),
            router: self.nav.clone(),
            feedback: self.feedback.clone(),
            async_fb: self.async_feedback.clone(),
            clipboard: self.clipboard.clone(),
            platform: self.platform.clone(),
            capabilities: self.capabilities.clone(),
            logic: self.logic.clone(),
            expr_cache: self.expr_cache.clone(),
            cancel: CancellationToken::new(),
            warnings: RefCell::new(Vec::new()),
        }
    }
}

fn select_viewport_root(
    tree: &crate::document::NodeTree,
    warnings: &mut Vec<String>,
) -> Option<crate::document::NodeKey> {
    let &first = tree.roots.first()?;
    if tree.roots.len() > 1 {
        let warning =
            "responsive document has extra top-level roots; only the first root is viewport-sized"
                .to_owned();
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    }
    if !matches!(
        tree.nodes[first].schema,
        jian_ops_schema::node::PenNode::Frame(_)
    ) {
        let warning =
            "responsive document's first top-level node is not a frame; viewport sizing skipped"
                .to_owned();
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
        return None;
    }
    Some(first)
}

fn root_has_limits(node: &jian_ops_schema::node::PenNode) -> bool {
    let jian_ops_schema::node::PenNode::Frame(frame) = node else {
        return false;
    };
    let limits = frame.container.limits;
    limits.min_width.is_some()
        || limits.max_width.is_some()
        || limits.min_height.is_some()
        || limits.max_height.is_some()
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.task_queue.cancel_all();
        for handle in self.ws_sessions.borrow().values() {
            handle.session.abort();
        }
        self.ws_sessions.borrow_mut().clear();
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

/// Cycle through an ordered option list by one step in the arrow
/// direction, wrapping at the ends. `None` for non-arrow keys or an
/// empty list. Starting from no selection, Down/Right picks the first
/// option and Up/Left the last.
fn step_option(options: &[String], current: Option<&str>, key: &str) -> Option<String> {
    if options.is_empty() {
        return None;
    }
    let delta: i32 = match key {
        "ArrowDown" | "ArrowRight" => 1,
        "ArrowUp" | "ArrowLeft" => -1,
        _ => return None,
    };
    let next = match current.and_then(|c| options.iter().position(|o| o == c)) {
        Some(i) => (i as i32 + delta).rem_euclid(options.len() as i32) as usize,
        None if delta > 0 => 0,
        None => options.len() - 1,
    };
    Some(options[next].clone())
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

    #[test]
    fn pump_retains_every_completed_action_outcome() {
        let mut runtime = Runtime::new();
        runtime.enable_action_reporting();
        for message in ["first", "second"] {
            runtime.task_queue.spawn_future(
                std::future::ready(ExecOutcome {
                    result: Err(crate::action::ActionError::Custom(message.to_owned())),
                    warnings: vec![crate::expression::Diagnostic {
                        kind: crate::expression::DiagKind::RuntimeWarning,
                        message: format!("warning-{message}"),
                        span: crate::expression::Span::zero(),
                    }],
                }),
                runtime.document_generation,
                Some(message.to_owned()),
            );
        }

        runtime.pump(0);
        let outcomes = runtime.take_action_outcomes();
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].outcome.warnings[0].message, "warning-first");
        assert_eq!(outcomes[1].outcome.warnings[0].message, "warning-second");
        assert_eq!(outcomes[0].source.as_deref(), Some("first"));
        assert_eq!(outcomes[1].source.as_deref(), Some("second"));
        assert!(matches!(
            outcomes[0].outcome.result,
            Err(crate::action::ActionError::Custom(ref message)) if message == "first"
        ));
        assert!(runtime.take_action_outcomes().is_empty(), "drain is exact");
    }

    #[test]
    fn synchronous_dispatch_parse_error_is_queued_for_host_reporting() {
        let schema: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","children":[
              {"type":"rectangle","id":"button","width":40,"height":40,
               "events":{"onTap":[{"not_registered":null}]}}
            ]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(schema).unwrap();
        runtime.enable_action_reporting();
        runtime.build_layout((100.0, 100.0)).unwrap();
        runtime.rebuild_spatial();
        for phase in [
            crate::gesture::PointerPhase::Down,
            crate::gesture::PointerPhase::Up,
        ] {
            runtime.dispatch_pointer(PointerEvent::simple(
                0,
                phase,
                crate::geometry::point(10.0, 10.0),
            ));
        }

        let outcomes = runtime.take_action_outcomes();
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(
            outcomes[0].outcome.result,
            Err(crate::action::ActionError::UnknownAction(ref action))
                if action == "not_registered"
        ));
        assert_eq!(outcomes[0].source.as_deref(), Some("onTap"));
    }

    #[test]
    fn top_level_authored_offsets_drive_hit_testing_and_focused_rect() {
        let schema: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","state":{"count":{"type":"int","default":0}},"children":[
              {"type":"rectangle","id":"button","x":80,"y":40,"width":80,"height":80,
               "events":{"onTap":[{"set":{"$app.count":"$app.count + 1"}}]}},
              {"type":"text_input","id":"field","x":20,"y":130,"width":100,"height":30,"value":""}
            ]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(schema).unwrap();
        runtime.build_layout((400.0, 200.0)).unwrap();
        runtime.rebuild_spatial();

        let button = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("button")
            .unwrap();
        assert_eq!(
            runtime.layout.node_rect(button),
            Some(crate::geometry::rect(80.0, 40.0, 80.0, 80.0)),
            "the production layout must retain authored geometry for a document root"
        );

        for phase in [
            crate::gesture::PointerPhase::Down,
            crate::gesture::PointerPhase::Up,
        ] {
            runtime.dispatch_pointer(PointerEvent::simple(
                1,
                phase,
                crate::geometry::point(100.0, 60.0),
            ));
        }
        assert_eq!(runtime.state.app_get("count").unwrap().as_i64(), Some(1));

        let field = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("field")
            .unwrap();
        runtime.focus_request(field).unwrap();
        assert_eq!(
            runtime.focused_node_rect(),
            Some(crate::geometry::rect(20.0, 130.0, 100.0, 30.0))
        );
    }

    #[test]
    fn failed_relayout_keeps_live_layout_spatial_and_dispatch_consistent() {
        let schema: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","responsive":true,
            "state":{"hit":{"type":"int","default":0}},
            "children":[{"type":"frame","id":"root","width":100,"height":100,"children":[
              {"type":"rectangle","id":"button","width":30,"height":30,
               "events":{"onTap":[{"set":{"$app.hit":"1"}}]}}]}]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(schema).unwrap();
        runtime.build_layout((100.0, 100.0)).unwrap();
        let key = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("button")
            .unwrap();
        let old_rect = runtime.layout.node_rect(key).unwrap();

        runtime.set_viewport_size((200.0, 100.0));
        runtime.layout.inject_staged_build_failure();
        assert!(runtime.relayout().is_err());
        assert_eq!(runtime.layout.node_rect(key), Some(old_rect));

        runtime.dispatch_pointer(PointerEvent::simple(
            1,
            crate::gesture::PointerPhase::Down,
            crate::geometry::point(5.0, 5.0),
        ));
        runtime.dispatch_pointer(PointerEvent::simple(
            1,
            crate::gesture::PointerPhase::Up,
            crate::geometry::point(5.0, 5.0),
        ));
        assert_eq!(runtime.state.app_get("hit").unwrap().as_i64(), Some(1));
    }

    #[test]
    fn failed_atomic_reload_keeps_previous_document_state_layout_and_tasks() {
        let mut runtime = Runtime::new();
        runtime
            .load_str(
                r#"{"version":"1.2","state":{"kept":{"type":"int","default":7}},"children":[{"type":"rectangle","id":"old","x":12,"y":8,"width":30,"height":20}]}"#,
            )
            .unwrap();
        runtime.build_layout((100.0, 80.0)).unwrap();
        let old_key = runtime.document.as_ref().unwrap().tree.get("old").unwrap();
        let old_rect = runtime.node_scene_rect(old_key).unwrap();
        runtime.task_queue.spawn_future(
            std::future::pending::<ExecOutcome>(),
            runtime.document_generation,
            Some("kept-task".to_owned()),
        );

        runtime.layout.inject_staged_build_failure();
        let error = runtime
            .load_str_and_relayout(
                r#"{"version":"1.2","state":{"replacement":{"type":"int","default":1}},"children":[{"type":"rectangle","id":"new","width":50,"height":50}]}"#,
            )
            .unwrap_err();

        assert!(matches!(error, CoreError::Layout(_)));
        assert!(runtime.document.as_ref().unwrap().tree.get("new").is_none());
        assert_eq!(runtime.state.app_get("kept").unwrap().as_i64(), Some(7));
        assert!(runtime.state.app_get("replacement").is_none());
        assert_eq!(runtime.node_scene_rect(old_key), Some(old_rect));
        assert!(!runtime.task_queue.is_empty());
    }

    #[test]
    fn atomic_reload_geometry_uses_exact_retained_layout_scopes() {
        let before = r#"{
          "version":"1.2","responsive":true,
          "app":{"name":"t","version":"1","id":"t","capabilities":["storage"]},
          "routes":{"entry":"/detail","routes":{"/detail":{"pageId":"main"}}},
          "state":{"offset":{"type":"int","default":1}},
          "pages":[{"id":"main","name":"Main","state":{"w":{"type":"int","default":10}},
            "children":[{"type":"rectangle","id":"box","width":1,"height":10,
              "state":{"extra":{"type":"int","default":2}}}]}],"children":[]}"#;
        let mut runtime = Runtime::new();
        runtime.load_str(before).unwrap();
        runtime.build_layout((200.0, 100.0)).unwrap();
        runtime.state.app_set("offset", serde_json::json!(5));
        runtime.state.page_set("main", "w", serde_json::json!(40));
        runtime
            .state
            .self_set("main", "box", "extra", serde_json::json!(3));
        runtime.state.storage_set("bump", serde_json::json!(7));
        runtime.nav = Rc::new(crate::screens::ScreenRouter::new(
            "/detail",
            ["/detail".to_owned()],
        ));

        runtime
            .load_str_and_relayout(
                r#"{
                  "version":"1.2","responsive":true,
                  "app":{"name":"t","version":"1","id":"t","capabilities":["storage"]},
                  "routes":{"entry":"/detail","routes":{"/detail":{"pageId":"main"}}},
                  "state":{"offset":{"type":"int","default":2}},
                  "pages":[{"id":"main","name":"Main","state":{"w":{"type":"int","default":12}},
                    "children":[{"type":"rectangle","id":"box","width":1,"height":10,
                      "state":{"extra":{"type":"int","default":4}},
                      "bindings":{"width":"$page.w + $self.extra + $viewport.width / 10 + ($route.path == '/detail' ? 20 : 0) + $storage.bump + $app.offset"}}]}],
                  "children":[]}"#,
            )
            .unwrap();

        let box_key = runtime.document.as_ref().unwrap().tree.get("box").unwrap();
        assert_eq!(runtime.layout.node_rect(box_key).unwrap().size.width, 95.0);
        assert_eq!(
            runtime.state.page_get("main", "w").unwrap().as_i64(),
            Some(40)
        );
        assert_eq!(
            runtime
                .state
                .self_get("main", "box", "extra")
                .unwrap()
                .as_i64(),
            Some(3)
        );
    }

    #[test]
    fn atomic_reload_revokes_storage_before_staging_geometry() {
        let mut runtime = Runtime::new();
        runtime
            .load_str(
                r#"{"version":"1.2","responsive":true,
                "app":{"name":"t","version":"1","id":"t","capabilities":["storage"]},
                "children":[{"type":"rectangle","id":"old","width":10,"height":10}]}"#,
            )
            .unwrap();
        runtime.build_layout((100.0, 100.0)).unwrap();
        runtime.state.storage_set("bump", serde_json::json!(70));

        runtime
            .load_str_and_relayout(
                r#"{"version":"1.2","responsive":true,
                "app":{"name":"t","version":"1","id":"t"},
                "children":[{"type":"rectangle","id":"box","width":1,"height":10,
                "bindings":{"width":"$storage.bump == null ? 11 : $storage.bump"}}]}"#,
            )
            .unwrap();

        let box_key = runtime.document.as_ref().unwrap().tree.get("box").unwrap();
        assert_eq!(runtime.layout.node_rect(box_key).unwrap().size.width, 11.0);
        assert!(runtime.state.storage_snapshot().is_empty());
        assert_eq!(
            runtime.state.storage_cache.snapshot(),
            serde_json::json!({})
        );
    }

    #[test]
    fn atomic_nonresponsive_reload_conforms_top_level_self_state() {
        let mut runtime = Runtime::new();
        runtime
            .load_str(
                r#"{"version":"1.2","children":[{"type":"rectangle","id":"card",
                "width":10,"height":10,"state":{"kept":{"type":"int","default":7}}}]}"#,
            )
            .unwrap();
        runtime.build_layout((100.0, 100.0)).unwrap();
        runtime
            .state
            .self_set("", "card", "kept", serde_json::json!(9));

        runtime
            .load_str_and_relayout(
                r#"{"version":"1.2","children":[{"type":"rectangle","id":"card",
                "width":20,"height":10,"state":{"kept":{"type":"int","default":8},
                "added":{"type":"int","default":4}}}]}"#,
            )
            .unwrap();

        assert_eq!(
            runtime.state.self_get("", "card", "kept").unwrap().as_i64(),
            Some(9)
        );
        assert_eq!(
            runtime
                .state
                .self_get("", "card", "added")
                .unwrap()
                .as_i64(),
            Some(4)
        );
    }

    #[test]
    fn atomic_reload_reseeds_same_discriminant_widget_kind_changes() {
        let mut runtime = Runtime::new();
        runtime
            .load_str(
                r#"{"version":"1.2","children":[
                {"type":"text_input","id":"text","value":"old","width":80,"height":20},
                {"type":"switch","id":"toggle","checked":false,"width":40,"height":20}]}"#,
            )
            .unwrap();
        runtime.build_layout((200.0, 100.0)).unwrap();
        for id in ["text", "toggle"] {
            let key = runtime.document.as_ref().unwrap().tree.get(id).unwrap();
            let schema = runtime.document.as_ref().unwrap().tree.nodes[key]
                .schema
                .clone();
            runtime.widget_states.get_or_init(&schema, &runtime.state);
        }
        if let Some(crate::widget_state::WidgetState::TextInput(text)) =
            runtime.widget_states.get_mut("text")
        {
            text.set_text("durable");
        }
        if let Some(crate::widget_state::WidgetState::Toggle { on }) =
            runtime.widget_states.get_mut("toggle")
        {
            *on = true;
        }

        runtime
            .load_str_and_relayout(
                r#"{"version":"1.2","children":[
                {"type":"text_area","id":"text","value":"fresh-area","width":80,"height":20},
                {"type":"checkbox","id":"toggle","checked":false,"width":40,"height":20}]}"#,
            )
            .unwrap();

        assert!(matches!(
            runtime.widget_states.get("text"),
            Some(crate::widget_state::WidgetState::TextInput(text)) if text.text() == "fresh-area"
        ));
        assert!(matches!(
            runtime.widget_states.get("toggle"),
            Some(crate::widget_state::WidgetState::Toggle { on: false })
        ));
    }

    #[test]
    fn atomic_reload_uses_ordered_numeric_clamp_for_reversed_ranges() {
        let mut runtime = Runtime::new();
        runtime
            .load_str(
                r#"{"version":"1.2","children":[
                {"type":"slider","id":"slider","value":5,"min":0,"max":10,"width":80,"height":20},
                {"type":"number_input","id":"number","value":5,"min":0,"max":10,"width":80,"height":20}]}"#,
            )
            .unwrap();
        runtime.build_layout((200.0, 100.0)).unwrap();
        for id in ["slider", "number"] {
            let key = runtime.document.as_ref().unwrap().tree.get(id).unwrap();
            let schema = runtime.document.as_ref().unwrap().tree.nodes[key]
                .schema
                .clone();
            runtime.widget_states.get_or_init(&schema, &runtime.state);
        }

        runtime
            .load_str_and_relayout(
                r#"{"version":"1.2","children":[
                {"type":"slider","id":"slider","min":100,"max":10,"width":80,"height":20},
                {"type":"number_input","id":"number","value":100,"min":100,"max":10,"width":80,"height":20}]}"#,
            )
            .unwrap();

        assert!(matches!(
            runtime.widget_states.get("slider"),
            Some(crate::widget_state::WidgetState::Slider { value, .. }) if (*value - 100.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            runtime.widget_states.get("number"),
            Some(crate::widget_state::WidgetState::TextInput(text)) if text.text() == "100"
        ));
    }

    #[test]
    fn host_driven_relayout_failure_is_queued_as_layout_error_not_warning() {
        let mut runtime = Runtime::new();
        runtime
            .load_str(
                r#"{"version":"1.2","responsive":true,"children":[{"type":"frame","id":"root","width":"fill_container","height":"fill_container"}]}"#,
            )
            .unwrap();
        runtime.build_layout((100.0, 80.0)).unwrap();
        let warning_count = runtime.load_warnings().len();

        runtime.layout.inject_staged_build_failure();
        runtime.set_viewport_size((120.0, 80.0));

        assert_eq!(runtime.load_warnings().len(), warning_count);
        let errors = runtime.take_layout_errors();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("viewport relayout failed"));
        assert!(runtime.take_layout_errors().is_empty());
    }

    #[test]
    fn pump_interleaves_event_chains_without_reordering_each_chain() {
        let schema: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","state":{"a":{"type":"int","default":0},"b":{"type":"int","default":0}},
            "children":[{"type":"frame","id":"root","layout":"horizontal","width":100,"height":30,"children":[
              {"type":"rectangle","id":"slow","width":40,"height":30,"events":{"onTap":[{"delay":{"ms":100}},{"set":{"$app.a":"1"}}]}},
              {"type":"rectangle","id":"fast","width":40,"height":30,"events":{"onTap":[{"delay":{"ms":50}},{"set":{"$app.b":"1"}}]}}
            ]}]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(schema).unwrap();
        runtime.build_layout((100.0, 30.0)).unwrap();
        for (pointer, x) in [(1, 5.0), (2, 45.0)] {
            runtime.dispatch_pointer(PointerEvent::simple_at(
                pointer,
                crate::gesture::PointerPhase::Down,
                crate::geometry::point(x, 5.0),
                0,
            ));
            runtime.dispatch_pointer(PointerEvent::simple_at(
                pointer,
                crate::gesture::PointerPhase::Up,
                crate::geometry::point(x, 5.0),
                0,
            ));
        }
        assert_eq!(runtime.state.app_get("a").unwrap().as_i64(), Some(0));
        assert_eq!(runtime.state.app_get("b").unwrap().as_i64(), Some(0));

        runtime.pump(50);
        assert_eq!(runtime.state.app_get("a").unwrap().as_i64(), Some(0));
        assert_eq!(runtime.state.app_get("b").unwrap().as_i64(), Some(1));
        runtime.pump(100);
        assert_eq!(runtime.state.app_get("a").unwrap().as_i64(), Some(1));
    }

    #[test]
    fn reload_cancels_pending_fetch_and_compensates_loading_before_merge() {
        struct PendingNetwork;
        #[async_trait::async_trait(?Send)]
        impl crate::action::services::NetworkClient for PendingNetwork {
            async fn request(
                &self,
                _request: crate::action::services::HttpRequest,
            ) -> Result<crate::action::services::HttpResponse, String> {
                std::future::pending().await
            }
        }
        let schema: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","responsive":true,"app":{"name":"t","version":"1","id":"t","capabilities":["network"]},
            "state":{"loading":{"type":"bool","default":false},"failed":{"type":"bool","default":false}},
            "children":[{"type":"rectangle","id":"button","width":30,"height":30,
             "bindings":{"width":"$app.loading ? 90 : 30"},
             "events":{"onTap":[{"fetch":{"url":"'https://example.invalid'","loading":"$app.loading","on_error":[{"set":{"$app.failed":"true"}}]}}]}}]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(schema.clone()).unwrap();
        runtime.network = Rc::new(PendingNetwork);
        runtime.build_layout((100.0, 100.0)).unwrap();
        for phase in [
            crate::gesture::PointerPhase::Down,
            crate::gesture::PointerPhase::Up,
        ] {
            runtime.dispatch_pointer(PointerEvent::simple(
                1,
                phase,
                crate::geometry::point(5.0, 5.0),
            ));
        }
        assert_eq!(
            runtime.state.app_get("loading").unwrap().as_bool(),
            Some(true)
        );
        runtime
            .load_str_and_relayout(&serde_json::to_string(&schema).unwrap())
            .unwrap();
        assert_eq!(
            runtime.state.app_get("loading").unwrap().as_bool(),
            Some(false)
        );
        assert_eq!(
            runtime.state.app_get("failed").unwrap().as_bool(),
            Some(false)
        );
        let button = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("button")
            .unwrap();
        assert_eq!(runtime.layout.node_rect(button).unwrap().size.width, 30.0);
    }

    #[test]
    fn failed_exact_reload_stage_keeps_pending_fetch_and_loading_state() {
        struct PendingNetwork;
        #[async_trait::async_trait(?Send)]
        impl crate::action::services::NetworkClient for PendingNetwork {
            async fn request(
                &self,
                _request: crate::action::services::HttpRequest,
            ) -> Result<crate::action::services::HttpResponse, String> {
                std::future::pending().await
            }
        }
        let source = r#"{"version":"1.2","responsive":true,
          "app":{"name":"t","version":"1","id":"t","capabilities":["network"]},
          "state":{"loading":{"type":"bool","default":false}},
          "children":[{"type":"rectangle","id":"button","width":30,"height":30,
          "events":{"onTap":[{"fetch":{"url":"'https://example.invalid'","loading":"$app.loading"}}]}}]}"#;
        let mut runtime = Runtime::new();
        runtime.load_str(source).unwrap();
        runtime.network = Rc::new(PendingNetwork);
        runtime.build_layout((100.0, 100.0)).unwrap();
        for phase in [
            crate::gesture::PointerPhase::Down,
            crate::gesture::PointerPhase::Up,
        ] {
            runtime.dispatch_pointer(PointerEvent::simple(
                1,
                phase,
                crate::geometry::point(5.0, 5.0),
            ));
        }
        assert_eq!(
            runtime.state.app_get("loading").unwrap().as_bool(),
            Some(true)
        );
        assert!(!runtime.task_queue.is_empty());

        runtime.layout.inject_staged_build_failure();
        assert!(runtime.load_str_and_relayout(source).is_err());

        assert_eq!(
            runtime.state.app_get("loading").unwrap().as_bool(),
            Some(true)
        );
        assert!(!runtime.task_queue.is_empty());
        assert!(runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("button")
            .is_some());
    }

    #[test]
    fn reload_transfers_surviving_image_owner_before_canceling_stale_request() {
        struct CancellationObserver {
            owner: Rc<RefCell<Option<Rc<Cell<u64>>>>>,
            seen_generation: Rc<Cell<Option<u64>>>,
        }

        impl Drop for CancellationObserver {
            fn drop(&mut self) {
                if let Some(owner) = self.owner.borrow().as_ref() {
                    self.seen_generation.set(Some(owner.get()));
                }
            }
        }

        struct PendingImages {
            owner: Rc<RefCell<Option<Rc<Cell<u64>>>>>,
            seen_generation: Rc<Cell<Option<u64>>>,
        }

        #[async_trait::async_trait(?Send)]
        impl crate::render::image_store::ImageResolver for PendingImages {
            async fn resolve(&self, url: &str) -> Result<Vec<u8>, String> {
                let observer = url.ends_with("stale.png").then(|| CancellationObserver {
                    owner: self.owner.clone(),
                    seen_generation: self.seen_generation.clone(),
                });
                std::future::pending::<()>().await;
                drop(observer);
                Ok(Vec::new())
            }
        }

        fn images(sources: &[&str]) -> PenDocument {
            serde_json::from_value(serde_json::json!({
                "version": "1.2",
                "app": {
                    "name": "images",
                    "version": "1",
                    "id": "images",
                    "capabilities": ["network"]
                },
                "children": sources
                    .iter()
                    .enumerate()
                    .map(|(index, source)| serde_json::json!({
                        "type": "image",
                        "id": format!("image-{index}"),
                        "src": source,
                        "width": 10,
                        "height": 10
                    }))
                    .collect::<Vec<_>>()
            }))
            .unwrap()
        }

        let keep = "https://example.invalid/keep.png";
        let stale = "https://example.invalid/stale.png";
        let owner = Rc::new(RefCell::new(None));
        let seen_generation = Rc::new(Cell::new(None));
        let mut runtime = Runtime::new_from_document(images(&[keep, stale])).unwrap();
        runtime.image_resolver = Rc::new(PendingImages {
            owner: owner.clone(),
            seen_generation: seen_generation.clone(),
        });
        runtime.pump(0);
        *owner.borrow_mut() = Some(
            runtime
                .image_requests
                .get(keep)
                .unwrap()
                .owner_generation
                .clone(),
        );

        runtime.replace_document(images(&[keep])).unwrap();

        assert_eq!(seen_generation.get(), Some(runtime.document_generation));
        assert_eq!(
            runtime
                .image_requests
                .get(keep)
                .unwrap()
                .owner_generation
                .get(),
            runtime.document_generation
        );
        assert!(!runtime.image_requests.contains_key(stale));
    }

    #[test]
    fn runtime_drop_aborts_websocket_sessions_synchronously() {
        struct Session(Rc<Cell<bool>>);
        #[async_trait::async_trait(?Send)]
        impl crate::action::services::WebSocketSession for Session {
            fn abort(&self) {
                self.0.set(true);
            }
            async fn send(&self, _: String) -> Result<(), String> {
                Ok(())
            }
            async fn close(&self) -> Result<(), String> {
                Ok(())
            }
        }
        let aborted = Rc::new(Cell::new(false));
        {
            let runtime = Runtime::new();
            runtime.ws_sessions.borrow_mut().insert(
                "socket".into(),
                crate::action::context::WsHandle {
                    session: Rc::new(Session(aborted.clone())),
                    on_message: None,
                    generation: 0,
                },
            );
        }
        assert!(aborted.get());
    }

    #[test]
    fn responsive_storage_read_hydrates_through_expression_and_pump() {
        struct Store;
        #[async_trait::async_trait(?Send)]
        impl StorageBackend for Store {
            async fn get(
                &self,
                key: &str,
            ) -> Result<Option<serde_json::Value>, crate::action::services::ServiceError>
            {
                Ok((key == "theme").then(|| serde_json::json!("dark")))
            }
            async fn set(
                &self,
                _: &str,
                _: serde_json::Value,
            ) -> Result<(), crate::action::services::ServiceError> {
                Ok(())
            }
            async fn delete(&self, _: &str) -> Result<(), crate::action::services::ServiceError> {
                Ok(())
            }
            async fn clear(&self) -> Result<(), crate::action::services::ServiceError> {
                Ok(())
            }
            async fn keys(&self) -> Result<Vec<String>, crate::action::services::ServiceError> {
                Ok(Vec::new())
            }
        }
        let schema: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","responsive":true,"app":{"name":"t","version":"1","id":"t","capabilities":["storage"]},"children":[]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(schema).unwrap();
        runtime.storage = Rc::new(Store);
        let expression = crate::expression::Expression::compile("$storage.theme").unwrap();
        assert!(expression.eval(&runtime.state, None, None).0.is_null());
        runtime.pump(1);
        assert_eq!(
            expression.eval(&runtime.state, None, None).0.as_str(),
            Some("dark")
        );
    }

    #[test]
    fn text_input_keyboard_and_text_routing() {
        use crate::gesture::pointer::Modifiers;
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{"version":"1.1","formatVersion":"1.1","children":[
                  {"type":"frame","id":"root","children":[
                    {"type":"text_input","id":"a"},
                    {"type":"text_input","id":"b"}]}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        // Focus the first input, type, then backspace one char.
        rt.focus_next().unwrap();
        assert!(rt.dispatch_text_input("hi").unwrap());
        rt.dispatch_keyboard("Backspace", Modifiers::empty());
        assert_eq!(widget_text(&mut rt, "a"), "h");
        // Tab to the second input; typing there leaves the first alone.
        rt.focus_next().unwrap();
        assert!(rt.dispatch_text_input("x").unwrap());
        assert_eq!(widget_text(&mut rt, "b"), "x");
        assert_eq!(widget_text(&mut rt, "a"), "h");
    }

    fn widget_text(rt: &mut Runtime, id: &str) -> String {
        match rt.widget_states.get_mut(id) {
            Some(crate::widget_state::WidgetState::TextInput(st)) => st.text().to_owned(),
            _ => panic!("expected text state for {id}"),
        }
    }

    #[test]
    fn bind_value_syncs_text_input_into_state_graph() {
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"email":{"type":"string","default":""}},
                  "children":[
                    {"type":"frame","id":"root","children":[
                      {"type":"text_input","id":"e",
                       "bindings":{"bind:value":"$state.email"}}]}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.focus_next().unwrap();
        assert!(rt.dispatch_text_input("a@b").unwrap());
        let got = rt
            .state
            .app_get("email")
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(got.as_deref(), Some("a@b"));
        // Backspace updates the bound value too.
        rt.dispatch_keyboard("Backspace", crate::gesture::pointer::Modifiers::empty());
        let got = rt
            .state
            .app_get("email")
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(got.as_deref(), Some("a@"));
    }

    #[test]
    fn number_input_bind_value_syncs_as_json_number() {
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"n":{"type":"float","default":0}},
                  "children":[
                    {"type":"frame","id":"root","children":[
                      {"type":"number_input","id":"ni",
                       "bindings":{"bind:value":"$state.n"}}]}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.focus_next().unwrap();
        assert!(rt.dispatch_text_input("42").unwrap());
        // Bound as a number, not the string "42".
        assert_eq!(rt.state.app_get("n").and_then(|v| v.as_f64()), Some(42.0));
    }

    #[test]
    fn switch_and_slider_keyboard_sync_to_state_graph() {
        use crate::gesture::pointer::Modifiers;
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"on":{"type":"bool","default":false},
                           "vol":{"type":"float","default":0}},
                  "children":[
                    {"type":"frame","id":"root","children":[
                      {"type":"switch","id":"sw","bindings":{"bind:value":"$state.on"}},
                      {"type":"slider","id":"sl","min":0,"max":10,"step":2,
                       "bindings":{"bind:value":"$state.vol"}}]}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        // Switch: Space flips it on.
        rt.focus_next().unwrap();
        rt.dispatch_keyboard(" ", Modifiers::empty());
        assert_eq!(rt.state.app_get("on").and_then(|v| v.as_bool()), Some(true));
        // Slider: two ArrowRight steps of 2 → 4.
        rt.focus_next().unwrap();
        rt.dispatch_keyboard("ArrowRight", Modifiers::empty());
        rt.dispatch_keyboard("ArrowRight", Modifiers::empty());
        assert_eq!(rt.state.app_get("vol").and_then(|v| v.as_f64()), Some(4.0));
    }

    #[test]
    fn select_arrow_keys_cycle_options_into_state_graph() {
        use crate::gesture::pointer::Modifiers;
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                // `choice` is deliberately NOT declared in the document
                // state schema: bound keys are created on first write
                // (sync_widget_binding). A declared key would exist at
                // mount and its persisted value would override the
                // authored `value:"a"` seed (bind:value read-back).
                r#"{"version":"1.1","formatVersion":"1.1",
                  "children":[
                    {"type":"frame","id":"root","children":[
                      {"type":"select","id":"se","value":"a",
                       "options":[{"value":"a","label":"A"},{"value":"b","label":"B"},
                                  {"value":"c","label":"C"}],
                       "bindings":{"bind:value":"$state.choice"}}]}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.focus_next().unwrap();
        rt.dispatch_keyboard("ArrowDown", Modifiers::empty()); // a → b
        rt.dispatch_keyboard("ArrowDown", Modifiers::empty()); // b → c
        assert_eq!(
            rt.state
                .app_get("choice")
                .and_then(|v| v.as_str().map(str::to_owned))
                .as_deref(),
            Some("c")
        );
        rt.dispatch_keyboard("ArrowDown", Modifiers::empty()); // c → a (wrap)
        assert_eq!(
            rt.state
                .app_get("choice")
                .and_then(|v| v.as_str().map(str::to_owned))
                .as_deref(),
            Some("a")
        );
    }

    #[test]
    fn tap_toggles_switch_and_syncs_state_graph() {
        use crate::geometry::point;
        use crate::gesture::pointer::{PointerEvent, PointerPhase};
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"on":{"type":"bool","default":false}},
                  "children":[
                    {"type":"frame","id":"root","width":200,"height":80,"children":[
                      {"type":"switch","id":"sw","x":10,"y":10,"width":44,"height":24,
                       "bindings":{"bind:value":"$state.on"}}]}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.build_layout((800.0, 600.0)).unwrap();
        rt.rebuild_spatial();
        let key = rt.document.as_ref().unwrap().tree.get("sw").unwrap();
        let r = rt.layout.node_rect(key).expect("switch laid out");
        let center = point(
            r.min_x() + r.size.width / 2.0,
            r.min_y() + r.size.height / 2.0,
        );
        // A full tap = Down then Up on the switch.
        rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Down, center));
        rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Up, center));
        assert_eq!(rt.state.app_get("on").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn drag_slider_to_track_end_drives_bound_state_toward_max() {
        use crate::geometry::point;
        use crate::gesture::pointer::{PointerEvent, PointerPhase};
        let mut rt = Runtime::new_from_document(
            serde_json::from_str::<PenDocument>(
                r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"vol":{"type":"float","default":0}},
                  "children":[
                    {"type":"frame","id":"root","width":300,"height":80,"children":[
                      {"type":"slider","id":"sl","x":10,"y":30,"width":200,"height":20,
                       "min":0,"max":100,"step":1,
                       "bindings":{"bind:value":"$state.vol"}}]}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        rt.build_layout((800.0, 600.0)).unwrap();
        rt.rebuild_spatial();
        let key = rt.document.as_ref().unwrap().tree.get("sl").unwrap();
        let r = rt.layout.node_rect(key).expect("slider laid out");
        // Down near the left (arms the drag), then Move to past the right
        // edge: the value should clamp to max.
        let left = point(r.min_x() + 2.0, r.min_y() + r.size.height / 2.0);
        let far_right = point(
            r.min_x() + r.size.width + 50.0,
            r.min_y() + r.size.height / 2.0,
        );
        rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Down, left));
        rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Move, far_right));
        rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Up, far_right));
        assert_eq!(
            rt.state.app_get("vol").and_then(|v| v.as_f64()),
            Some(100.0),
            "dragging to the track's right end should drive the value to max"
        );
    }

    #[test]
    fn legacy_role_input_promotes_and_is_editable_via_runtime() {
        use jian_ops_schema::compat::{load_str_with, LoadOptions};
        // End-to-end: an old `frame role="input"` (with a bind:value) is
        // promoted on load, focusable by type, accepts typed text, and
        // syncs into the state graph — exercising Phase A promote +
        // Phase B focus/routing/bind-sync in one path.
        let legacy = r#"{"version":"1.1","formatVersion":"1.1",
          "state":{"q":{"type":"string","default":""}},
          "children":[
            {"type":"frame","id":"root","children":[
              {"type":"frame","id":"f","role":"input",
               "bindings":{"bind:value":"$state.q"}}]}]}"#;
        let loaded = load_str_with(
            legacy,
            LoadOptions {
                promote_legacy_widgets: true,
            },
        )
        .unwrap();
        let mut rt = Runtime::new_from_document(loaded.value).unwrap();
        rt.focus_next().unwrap();
        assert!(rt.dispatch_text_input("hey").unwrap());
        let got = rt
            .state
            .app_get("q")
            .and_then(|v| v.as_str().map(str::to_owned));
        assert_eq!(got.as_deref(), Some("hey"));
    }

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

    #[test]
    fn reload_replaces_nonconforming_live_state_with_staged_default() {
        let old: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","state":{"value":{"type":"string","default":"old"}},"children":[]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(old).unwrap();
        runtime.state.app_set("value", serde_json::json!("live"));
        let new: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","state":{"value":{"type":"int","default":7}},"children":[]}"#,
        )
        .unwrap();
        runtime.replace_document(new).unwrap();
        assert_eq!(runtime.state.app_get("value").unwrap().as_i64(), Some(7));
        assert!(runtime
            .load_warnings()
            .iter()
            .any(|warning| warning.contains("no longer conforms")));
    }

    #[test]
    fn loader_failure_leaves_tasks_sessions_hydration_and_generation_untouched() {
        use crate::action::context::WsHandle;
        use crate::action::services::WebSocketSession;
        use async_trait::async_trait;
        struct Session;
        #[async_trait(?Send)]
        impl WebSocketSession for Session {
            async fn send(&self, _: String) -> Result<(), String> {
                Ok(())
            }
            async fn close(&self) -> Result<(), String> {
                Ok(())
            }
            async fn receive(&self) -> Vec<String> {
                Vec::new()
            }
        }
        let schema: PenDocument =
            serde_json::from_str(r#"{"version":"1.2","children":[]}"#).unwrap();
        let mut runtime = Runtime::new_from_document(schema.clone()).unwrap();
        runtime.ws_sessions.borrow_mut().insert(
            "live".into(),
            WsHandle {
                session: Rc::new(Session),
                on_message: None,
                generation: runtime.document_generation,
            },
        );
        runtime.task_queue.spawn_future(
            std::future::pending::<ExecOutcome>(),
            runtime.document_generation,
            Some("pending".into()),
        );
        let _ = runtime.state.storage_cache.read("theme");
        let generation = runtime.document_generation;
        runtime.fail_next_loader = true;
        assert!(runtime.replace_document(schema).is_err());
        assert_eq!(runtime.document_generation, generation);
        assert!(runtime.ws_sessions.borrow().contains_key("live"));
        assert!(!runtime.task_queue.is_empty());
        assert!(runtime.state.storage_cache.is_hydrating("theme"));
    }

    #[test]
    fn successful_reload_restores_route_snapshot_against_new_valid_paths() {
        use crate::action::services::RouteState;
        struct RecordingRouter {
            restored: RefCell<Option<(RouteState, Vec<String>)>>,
        }
        impl RouterSvc for RecordingRouter {
            fn current(&self) -> RouteState {
                RouteState {
                    path: "/stats".into(),
                    params: [("id".into(), "7".into())].into(),
                    query: [("tab".into(), "all".into())].into(),
                    stack: vec!["/".into(), "/stats".into()],
                }
            }
            fn push(&self, _: &str) {}
            fn replace(&self, _: &str) {}
            fn pop(&self) {}
            fn reset(&self, _: &str) {}
            fn restore(&self, state: RouteState, valid: &[String]) {
                *self.restored.borrow_mut() = Some((state, valid.to_vec()));
            }
        }
        let old: PenDocument = serde_json::from_str(r#"{"version":"1.2","children":[]}"#).unwrap();
        let mut runtime = Runtime::new_from_document(old).unwrap();
        let router = Rc::new(RecordingRouter {
            restored: RefCell::new(None),
        });
        runtime.nav = router.clone();
        let new: PenDocument = serde_json::from_str(r#"{
          "version":"1.2","routes":{"entry":"/","routes":{"/":{"pageId":"home"},"/stats":{"pageId":"stats"}}},
          "pages":[{"id":"home","name":"Home","children":[]},{"id":"stats","name":"Stats","children":[]}],"children":[]}"#).unwrap();
        runtime.replace_document(new).unwrap();
        let restored = router.restored.borrow();
        let (state, valid) = restored.as_ref().expect("restore called");
        assert_eq!(state.path, "/stats");
        assert!(valid.contains(&"/".to_owned()) && valid.contains(&"/stats".to_owned()));
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
        assert!(!rt.capabilities.check(Capability::Network, "fetch", 0));

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
        assert!(rt.capabilities.check(Capability::Network, "fetch", 0));
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
                generation: rt.document_generation,
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
    fn pump_websockets_reports_synchronous_handler_parse_errors() {
        use crate::action::context::WsHandle;
        use crate::action::services::WebSocketSession;
        use async_trait::async_trait;

        struct OneMessage;
        #[async_trait(?Send)]
        impl WebSocketSession for OneMessage {
            async fn send(&self, _: String) -> Result<(), String> {
                Ok(())
            }
            async fn close(&self) -> Result<(), String> {
                Ok(())
            }
            async fn receive(&self) -> Vec<String> {
                vec!["hello".to_owned()]
            }
        }

        let mut runtime = Runtime::new();
        runtime
            .load_str(r#"{"version":"1.2","children":[]}"#)
            .unwrap();
        runtime.enable_action_reporting();
        runtime.ws_sessions.borrow_mut().insert(
            "chat".to_owned(),
            WsHandle {
                session: Rc::new(OneMessage),
                on_message: Some(serde_json::json!([{"not_registered": null}])),
                generation: runtime.document_generation,
            },
        );

        assert_eq!(runtime.pump_websockets(), 0);
        let outcomes = runtime.take_action_outcomes();
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(
            outcomes[0].outcome.result,
            Err(crate::action::ActionError::UnknownAction(ref name))
                if name == "not_registered"
        ));
        assert_eq!(outcomes[0].source.as_deref(), Some("websocket:chat"));
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
        let evs = rt.focus_request(key_a).unwrap();
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

    #[test]
    fn responsive_viewport_root_takes_available_size() {
        let document: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","formatVersion":"1.2","responsive":true,"children":[
                {"type":"frame","id":"root","x":50,"y":50,"width":400,"height":300}]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(document).unwrap();
        runtime.build_layout((800.0, 600.0)).unwrap();
        let key = runtime.document.as_ref().unwrap().tree.get("root").unwrap();
        let rect = runtime.layout.node_rect(key).unwrap();
        assert_eq!(
            (
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height
            ),
            (0.0, 0.0, 800.0, 600.0)
        );
        assert!(runtime.layout.is_origin_normalized(key));
    }

    #[test]
    fn responsive_root_min_max_is_ignored_with_warning() {
        let document: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","formatVersion":"1.2","responsive":true,"children":[
                {"type":"frame","id":"root","width":400,"height":300,"minWidth":900}]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(document).unwrap();
        runtime.build_layout((200.0, 600.0)).unwrap();
        let key = runtime.document.as_ref().unwrap().tree.get("root").unwrap();
        assert_eq!(runtime.layout.node_rect(key).unwrap().size.width, 200.0);
        assert!(runtime
            .load_warnings()
            .iter()
            .any(|warning| warning.contains("min/max")));
    }

    #[test]
    fn non_responsive_root_keeps_authored_size() {
        let document: PenDocument = serde_json::from_str(
            r#"{"version":"1.1","children":[
                {"type":"frame","id":"root","x":50,"y":50,"width":400,"height":300}]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(document).unwrap();
        runtime.build_layout((800.0, 600.0)).unwrap();
        let key = runtime.document.as_ref().unwrap().tree.get("root").unwrap();
        assert_eq!(runtime.layout.node_rect(key).unwrap().size.width, 400.0);
        assert!(!runtime.layout.is_origin_normalized(key));
    }

    #[test]
    fn responsive_constraints_run_when_first_root_is_not_a_frame() {
        let document: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","formatVersion":"1.2","responsive":true,"children":[
                {"type":"text","id":"heading","content":"Heading"},
                {"type":"frame","id":"root","width":100,"height":100,"children":[
                    {"type":"rectangle","id":"c","x":80,"y":0,"width":30,"height":10,
                    "maxWidth":20,"constraints":{"h":"right","v":"top"}}]}]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(document).unwrap();
        runtime.build_layout((800.0, 600.0)).unwrap();
        let key = runtime.document.as_ref().unwrap().tree.get("c").unwrap();
        let rect = runtime.layout.node_rect(key).unwrap();
        assert_eq!((rect.origin.x, rect.size.width), (90.0, 20.0));
    }

    #[test]
    fn non_responsive_build_does_not_mutate_runtime_viewport() {
        let document: PenDocument = serde_json::from_str(
            r#"{"version":"1.1","children":[
                {"type":"frame","id":"root","width":400,"height":300}]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(document).unwrap();
        runtime.build_layout((123.0, 456.0)).unwrap();
        assert_eq!(
            (runtime.viewport.size.width, runtime.viewport.size.height),
            (800.0, 600.0)
        );
    }

    #[test]
    fn responsive_origin_normalization_aligns_scene_and_hit_test() {
        let document: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","formatVersion":"1.2","responsive":true,"children":[
                {"type":"frame","id":"root","x":50,"y":60,"width":100,"height":100,
                "children":[{"type":"rectangle","id":"child","x":10,"y":10,
                "width":20,"height":20}]}]}"#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(document).unwrap();
        runtime.build_layout((100.0, 100.0)).unwrap();
        runtime.rebuild_spatial();
        let child = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("child")
            .unwrap();
        let rect = runtime.layout.node_rect(child).unwrap();
        assert_eq!((rect.origin.x, rect.origin.y), (10.0, 10.0));
        assert!(runtime
            .spatial
            .hit(crate::geometry::point(15.0, 15.0))
            .contains(&child));
        assert!(!runtime
            .spatial
            .hit(crate::geometry::point(65.0, 75.0))
            .contains(&child));
    }

    #[test]
    fn projected_screen_root_is_viewport_sized() {
        let source: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","formatVersion":"1.2","responsive":true,"children":[
                {"type":"frame","id":"screen","screen":"/","x":50,"y":60,
                "width":400,"height":300}]}"#,
        )
        .unwrap();
        let (projected, _) = jian_ops_schema::screen_projection::project_screens(&source);
        let (projected, _) = projected.unwrap();
        let mut runtime = Runtime::new_from_document(projected).unwrap();
        runtime.build_layout((320.0, 480.0)).unwrap();
        let root = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("screen")
            .unwrap();
        let rect = runtime.layout.node_rect(root).unwrap();
        assert_eq!((rect.size.width, rect.size.height), (320.0, 480.0));
        assert!(runtime.layout.is_origin_normalized(root));
    }

    fn variant_runtime() -> Runtime {
        let source: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","responsive":true,"state":{"long":{"type":"int","default":0}},"children":[
              {"type":"frame","id":"desktop","screen":"/","width":300,"height":200,"children":[{"type":"text_input","id":"field","value":"abIMEz","width":100,"height":30,"events":{"onLongPress":[{"set":{"$app.long":"1"}}]}}]},
              {"type":"frame","id":"mobile","screen":"/","breakpoint":{"maxWidth":480},"children":[{"type":"text_input","id":"field","value":"mobile"}]}]}"#,
        ).unwrap();
        let (projected, _) = jian_ops_schema::screen_projection::project_screens(&source);
        let (normalized, variants) = projected.unwrap();
        let desktop = normalized
            .pages
            .as_ref()
            .unwrap()
            .iter()
            .find(|page| page.id == "desktop")
            .unwrap()
            .clone();
        let mut mounted = normalized.clone();
        mounted.pages = Some(vec![desktop]);
        let mut runtime = Runtime::new_from_document(mounted).unwrap();
        runtime.configure_variant_source(normalized, "/", variants);
        runtime
    }

    fn freeze_variant_runtime(runtime: &mut Runtime) {
        let key = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("field")
            .unwrap();
        let node = runtime.document.as_ref().unwrap().tree.nodes[key]
            .schema
            .clone();
        let state = runtime
            .widget_states
            .get_or_init(&node, &runtime.state)
            .unwrap();
        let crate::widget_state::WidgetState::TextInput(state) = state else {
            panic!()
        };
        state.set_composition("pending", 7, 0);
        runtime.switch_variant("mobile@0-480").unwrap();
        assert!(runtime.input_frozen());
    }

    #[test]
    fn focus_entry_points_return_busy_while_variant_input_is_frozen() {
        let mut runtime = variant_runtime();
        let key = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("field")
            .unwrap();
        freeze_variant_runtime(&mut runtime);
        assert!(matches!(runtime.focus_next(), Err(CoreError::Busy)));
        assert!(matches!(runtime.focus_previous(), Err(CoreError::Busy)));
        assert!(matches!(runtime.focus_request(key), Err(CoreError::Busy)));
        assert!(matches!(runtime.focus_clear(), Err(CoreError::Busy)));
    }

    #[test]
    fn websocket_messages_wait_until_variant_freeze_lifts() {
        use crate::action::context::WsHandle;
        use crate::action::services::WebSocketSession;
        use async_trait::async_trait;

        struct Session(Rc<RefCell<Vec<String>>>);
        #[async_trait(?Send)]
        impl WebSocketSession for Session {
            async fn send(&self, _: String) -> Result<(), String> {
                Ok(())
            }
            async fn close(&self) -> Result<(), String> {
                Ok(())
            }
            async fn receive(&self) -> Vec<String> {
                std::mem::take(&mut *self.0.borrow_mut())
            }
        }

        let mut runtime = variant_runtime();
        runtime.state.app_set("last", serde_json::json!(""));
        let inbox = Rc::new(RefCell::new(vec!["later".to_owned()]));
        runtime.ws_sessions.borrow_mut().insert(
            "chat".into(),
            WsHandle {
                session: Rc::new(Session(inbox.clone())),
                on_message: Some(serde_json::json!([{ "set": { "$app.last": "$event.data" } }])),
                generation: runtime.document_generation,
            },
        );
        freeze_variant_runtime(&mut runtime);
        assert_eq!(runtime.pump_websockets(), 0);
        assert_eq!(inbox.borrow().as_slice(), ["later"]);
        let request = match runtime.swap_state {
            SwapState::AwaitingIme { request_id, .. } => request_id,
            _ => unreachable!(),
        };
        runtime.confirm_ime_cancel(request);
        assert_eq!(runtime.pump_websockets(), 1);
        assert_eq!(
            runtime.state.app_get("last").unwrap().as_str(),
            Some("later")
        );
    }

    #[test]
    fn pending_long_press_is_dropped_when_tick_occurs_during_freeze() {
        let mut runtime = variant_runtime();
        runtime.build_layout((300.0, 200.0)).unwrap();
        runtime.rebuild_spatial();
        let key = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("field")
            .unwrap();
        let rect = runtime.layout.node_rect(key).unwrap();
        runtime.dispatch_pointer(PointerEvent::simple(
            1,
            crate::gesture::PointerPhase::Down,
            crate::geometry::point(rect.min_x() + 1.0, rect.min_y() + 1.0),
        ));
        freeze_variant_runtime(&mut runtime);

        let emitted = runtime.tick(800);
        assert!(emitted.is_empty());
        assert_eq!(runtime.state.app_get("long").unwrap().as_i64(), Some(0));
    }

    #[test]
    fn transactional_variant_switch_updates_page_context() {
        let mut runtime = variant_runtime();
        assert!(runtime.switch_variant("mobile@0-480").unwrap());
        assert_eq!(runtime.selected_variant(), Some("mobile@0-480"));
        assert_eq!(runtime.active_page_key(), "mobile@0-480");
        assert!(!runtime.input_frozen());
    }

    #[test]
    fn failed_detached_build_leaves_every_live_variant_context_untouched() {
        let mut runtime = variant_runtime();
        let document_schema = runtime.document.as_ref().unwrap().schema.clone();
        let page_key = runtime.active_page_key().to_owned();
        let selected = runtime.selected_variant().map(str::to_owned);
        let counter = runtime.mutation_counter();
        let capabilities = runtime.capabilities.clone();

        let error = runtime.switch_variant("missing-variant").unwrap_err();
        assert!(matches!(error, CoreError::Layout(_)));
        assert_eq!(runtime.document.as_ref().unwrap().schema, document_schema);
        assert_eq!(runtime.active_page_key(), page_key);
        assert_eq!(runtime.selected_variant(), selected.as_deref());
        assert_eq!(runtime.mutation_counter(), counter);
        assert!(Rc::ptr_eq(&runtime.capabilities, &capabilities));
        assert!(!runtime.input_frozen());
    }

    #[test]
    fn failed_rebuild_while_awaiting_ime_abandons_and_detaches_swap() {
        let mut runtime = variant_runtime();
        freeze_variant_runtime(&mut runtime);
        let request = match runtime.swap_state {
            SwapState::AwaitingIme { request_id, .. } => request_id,
            _ => unreachable!(),
        };

        assert!(runtime.switch_variant("missing-variant").is_err());
        assert!(!runtime.input_frozen());
        assert_eq!(runtime.selected_variant(), Some("desktop"));
        assert!(runtime
            .take_layout_errors()
            .iter()
            .any(|error| error.contains("parked variant rebuild failed")));
        assert_eq!(
            runtime.confirm_ime_cancel(request),
            ImeConfirmOutcome::Applied
        );
        assert_eq!(runtime.selected_variant(), Some("desktop"));
    }

    #[test]
    fn composition_parks_and_confirmation_commits_swap() {
        let mut runtime = variant_runtime();
        let key = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("field")
            .unwrap();
        let node = runtime.document.as_ref().unwrap().tree.nodes[key]
            .schema
            .clone();
        let field = runtime
            .widget_states
            .get_or_init(&node, &runtime.state)
            .unwrap();
        let crate::widget_state::WidgetState::TextInput(field) = field else {
            panic!()
        };
        field.set_caret(2, 0);
        runtime.focus_request(key).unwrap();
        runtime
            .dispatch_ime(crate::gesture::ime::ImeEvent {
                kind: crate::gesture::ime::ImeKind::CompositionStart,
                text: String::new(),
            })
            .unwrap();
        runtime
            .dispatch_ime(crate::gesture::ime::ImeEvent {
                kind: crate::gesture::ime::ImeKind::CompositionUpdate { selection: None },
                text: "IME".into(),
            })
            .unwrap();
        assert!(!runtime.switch_variant("mobile@0-480").unwrap());
        assert!(runtime.input_frozen());
        assert!(matches!(
            runtime.dispatch_text_input("blocked"),
            Err(CoreError::Busy)
        ));
        assert!(matches!(
            runtime.dispatch_ime(crate::gesture::ime::ImeEvent {
                kind: crate::gesture::ime::ImeKind::CompositionEnd,
                text: "blocked".into(),
            }),
            Err(CoreError::Busy)
        ));
        let request_id = match &runtime.swap_state {
            SwapState::AwaitingIme { request_id, .. } => *request_id,
            _ => panic!(),
        };
        assert_eq!(
            runtime.confirm_ime_commit(request_id, "OK"),
            ImeConfirmOutcome::Applied
        );
        assert_eq!(runtime.selected_variant(), Some("mobile@0-480"));
        assert!(!runtime.input_frozen());
        assert_eq!(runtime.last_variant_build_count(), 2);
        match runtime.widget_states.get_for_page("desktop", "field") {
            Some(crate::widget_state::WidgetState::TextInput(field)) => {
                assert_eq!(field.text(), "abOKIMEz");
                assert!(field.composition().is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn pump_reports_swap_deadline_and_times_out_parked_ime_swap() {
        let mut runtime = variant_runtime();
        let key = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("field")
            .unwrap();
        let node = runtime.document.as_ref().unwrap().tree.nodes[key]
            .schema
            .clone();
        let field = runtime
            .widget_states
            .get_or_init(&node, &runtime.state)
            .unwrap();
        let crate::widget_state::WidgetState::TextInput(field) = field else {
            panic!()
        };
        field.set_caret(2, 100);
        runtime.focus_request(key).unwrap();
        runtime
            .dispatch_ime(crate::gesture::ime::ImeEvent {
                kind: crate::gesture::ime::ImeKind::CompositionStart,
                text: String::new(),
            })
            .unwrap();
        runtime
            .dispatch_ime(crate::gesture::ime::ImeEvent {
                kind: crate::gesture::ime::ImeKind::CompositionUpdate { selection: None },
                text: "IME".into(),
            })
            .unwrap();
        assert!(!runtime.switch_variant("mobile@0-480").unwrap());

        let directive = runtime.pump(100);
        assert_eq!(directive.next_wake_ms, Some(500));
        assert!(runtime.input_frozen());

        let directive = runtime.pump(500);
        assert!(directive.needs_paint);
        assert!(!runtime.input_frozen());
        assert_eq!(runtime.selected_variant(), Some("mobile@0-480"));
    }

    #[test]
    fn event_actions_receive_active_page_and_source_node_context() {
        let schema: PenDocument = serde_json::from_str(
            r#"{
          "version":"1.2","responsive":true,
          "pages":[{"id":"responsive-page","name":"P","children":[
            {"type":"frame","id":"button","width":100,"height":50,
             "events":{"onTap":[{"set":{"$page.hit":"1"}},{"set":{"$self.hit":"2"}}]}}
          ]}]}
        "#,
        )
        .unwrap();
        let mut runtime = Runtime::new_from_document(schema).unwrap();
        runtime.build_layout((100.0, 50.0)).unwrap();
        runtime.rebuild_spatial();
        runtime.dispatch_pointer(PointerEvent::simple(
            0,
            crate::gesture::PointerPhase::Down,
            crate::geometry::point(10.0, 10.0),
        ));
        runtime.dispatch_pointer(PointerEvent::simple(
            0,
            crate::gesture::PointerPhase::Up,
            crate::geometry::point(10.0, 10.0),
        ));
        assert_eq!(
            runtime.state.page.borrow()["responsive-page"]["hit"]
                .get()
                .0,
            serde_json::json!(1)
        );
        assert_eq!(
            runtime
                .state
                .self_get("responsive-page", "button", "hit")
                .unwrap()
                .0,
            serde_json::json!(2)
        );
    }

    #[test]
    fn unprojected_responsive_initial_load_normalizes_page_ids() {
        let schema: PenDocument = serde_json::from_str(
            r#"{
          "version":"1.2","responsive":true,
          "pages":[
            {"id":"","name":"A","children":[]},
            {"id":"","name":"B","children":[]},
            {"id":"~root","name":"Reserved","children":[]}
          ]}
        "#,
        )
        .unwrap();
        let runtime = Runtime::new_from_document(schema).unwrap();
        let ids: Vec<&str> = runtime
            .document
            .as_ref()
            .unwrap()
            .schema
            .pages
            .as_ref()
            .unwrap()
            .iter()
            .map(|page| page.id.as_str())
            .collect();
        assert_eq!(ids, ["~root~2", "~root~3", "~root"]);
        assert_eq!(runtime.active_page_key(), "~root~2");
    }
}
