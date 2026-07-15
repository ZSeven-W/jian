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
    AsyncFeedback, ClipboardService, FeedbackSink, NetworkClient, PlatformService,
    Router as RouterSvc, StorageBackend,
};
use crate::action::{ExecOutcome, SharedRegistry, TaskClock, TaskQueue};
use crate::binding::DeferredBindingQueue;
use crate::capability::{AuditLog, CapabilityGate, PermissionBroker};
use crate::document::RuntimeDocument;
use crate::effect::EffectRegistry;
use crate::expression::ExpressionCache;
use crate::gesture::{FocusManager, PointerRouter};
use crate::layout::LayoutEngine;
use crate::signal::scheduler::Scheduler;
use crate::spatial::SpatialIndex;
use crate::state::StateGraph;
use crate::viewport::Viewport;
use jian_ops_schema::document::PenDocument;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
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

mod async_runtime;
mod construction;
mod diagnostics;
mod document;
mod document_geometry;
mod document_prepare;
mod focus_input;
mod image_runtime;
mod ime_handshake;
mod keyboard_input;
mod layout_runtime;
mod lifecycle;
mod pointer_input;
mod pump;
mod reload_resources;
mod text_geometry;
mod text_input;
mod variant_swap;
mod websocket_runtime;

pub use ime_handshake::{ImeConfirmOutcome, ImeControlOp, ImeHost, ImeSnapshot};
pub use pump::FrameDirective;
pub use text_input::{EditableInputKind, EditableTextSnapshot};
pub use variant_swap::{ParkedBuild, SwapState};

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
    pub image_store: crate::render::image_store::ImageStore,
    pub image_resolver: Rc<dyn crate::render::image_store::ImageResolver>,
    text_geometry: Option<Rc<dyn crate::render::TextGeometry>>,
    text_geometry_ready: bool,
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
    /// Install a Tier-3 `LogicProvider`. Replaces the default
    /// `NullLogicProvider` and takes effect for every subsequent
    /// `call` action dispatch (the cached `ActionContext` is rebuilt
    /// per action chain, so no cache invalidation is needed).
    pub fn set_logic_provider(&mut self, provider: Rc<dyn crate::logic::LogicProvider>) {
        self.logic = provider;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreError;
    use crate::geometry::size;
    use crate::gesture::{PointerEvent, SemanticEvent};

    include!("runtime/tests_reload.rs");
    include!("runtime/tests_async_resources.rs");
    include!("runtime/tests_widgets.rs");
    include!("runtime/tests_input_layout.rs");
    include!("runtime/tests_variants.rs");
}
