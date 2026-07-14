//! Pump-driven rAF/timeout scheduler with paint-idle semantics.
mod surface_lifecycle;

use self::surface_lifecycle::{apply_metrics, context_lost, context_restored, recreate_surface};
use crate::backend::{CanvasKitBackend, CanvasKitSurface};
use crate::clock::HostClock;
use crate::ime_input::ImeInput;
use crate::mount::Callbacks;
use crate::runtime_slot::RuntimeSlot;
use crate::viewport::ViewportBridge;
use jian_core::action::services::{NullRouter, RouteState, Router};
use jian_core::render::{
    collect_rich_draws_with_state, collect_scene_paint_commands_with_state, DrawOp, RenderBackend,
    RichDrawList, ScenePaintCommand,
};
use jian_core::screens::{reconcile_screens_with_layout, ScreenRouter, ScreenTable};
use js_sys::Function;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{HtmlCanvasElement, Window};
pub(crate) type RuntimeHandle = Rc<RuntimeSlot>;
struct NavigationState {
    router: Rc<ScreenRouter>,
    table: ScreenTable,
    current_path: String,
}

enum FramePaintCommands {
    /// The pre-responsive production path. Keeping this replay separate is
    /// required by the §1.1 no-output-change contract for legacy documents.
    Legacy(RichDrawList),
    Responsive(Vec<ScenePaintCommand>),
}
struct State {
    window: Window,
    clock: HostClock,
    runtime: RuntimeHandle,
    backend: Option<CanvasKitBackend>,
    surface: Option<CanvasKitSurface>,
    ime: Option<ImeInput>,
    logical: (f32, f32),
    physical: (u32, u32),
    dpr: f32,
    connected: bool,
    context_lost: bool,
    backend_generation: u64,
    disposed: bool,
    raf_id: Option<i32>,
    raf_epoch: u64,
    raf_pending_epoch: Option<u64>,
    timeout_id: Option<i32>,
    timeout_epoch: u64,
    timeout_pending_epoch: Option<u64>,
    pump_microtask_pending: bool,
    raf_callback: Option<Closure<dyn FnMut(f64)>>,
    timeout_callback: Option<Closure<dyn FnMut()>>,
    presented_frames: u64,
    callbacks: Callbacks,
    warning_count: usize,
    navigation: Option<NavigationState>,
}
impl State {
    fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }
    fn paintable(&self) -> bool {
        self.connected
            && self.logical.0 > 0.0
            && self.logical.1 > 0.0
            && !self.context_lost
            && self.backend.is_some()
            && self.surface.is_some()
    }
}
pub(crate) struct RafPump {
    state: Rc<RefCell<State>>,
    viewport: Option<ViewportBridge>,
}
impl RafPump {
    pub(crate) fn start_with_clock(
        canvas: HtmlCanvasElement,
        runtime: RuntimeHandle,
        backend: CanvasKitBackend,
        ime: Option<ImeInput>,
        clock: HostClock,
        callbacks: Callbacks,
    ) -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("window unavailable"))?;
        let state = Rc::new(RefCell::new(State {
            window: window.clone(),
            clock,
            runtime,
            backend: Some(backend),
            surface: None,
            ime,
            logical: (0.0, 0.0),
            physical: (0, 0),
            dpr: 1.0,
            connected: false,
            context_lost: false,
            backend_generation: 0,
            disposed: false,
            raf_id: None,
            raf_epoch: 0,
            raf_pending_epoch: None,
            timeout_id: None,
            timeout_epoch: 0,
            timeout_pending_epoch: None,
            pump_microtask_pending: false,
            raf_callback: None,
            timeout_callback: None,
            presented_frames: 0,
            callbacks,
            warning_count: 0,
            navigation: None,
        }));
        install_callbacks(&state);
        let weak = Rc::downgrade(&state);
        let on_metrics = Rc::new(move |metrics| {
            if let Some(state) = weak.upgrade() {
                apply_metrics(&state, metrics);
            }
        });
        let weak = Rc::downgrade(&state);
        let on_lost = Rc::new(move || {
            if let Some(state) = weak.upgrade() {
                context_lost(&state);
            }
        });
        let weak = Rc::downgrade(&state);
        let on_restored = Rc::new(move || {
            if let Some(state) = weak.upgrade() {
                context_restored(&state);
            }
        });
        let viewport = ViewportBridge::attach(canvas, on_metrics, on_lost, on_restored)?;
        request_frame(&state);
        Ok(Self {
            state,
            viewport: Some(viewport),
        })
    }

    pub(crate) fn set_ime(&self, ime: ImeInput) {
        let previous = self.state.borrow_mut().ime.replace(ime);
        drop(previous);
    }
    pub(crate) fn wake_soon(&self) {
        queue_pump_microtask(&self.state);
    }
    pub(crate) fn waker(&self) -> Rc<dyn Fn()> {
        let weak = Rc::downgrade(&self.state);
        Rc::new(move || {
            if let Some(state) = weak.upgrade() {
                queue_pump_microtask(&state);
            }
        })
    }
    pub(crate) fn sync_viewport_now(&self) {
        if let Some(viewport) = &self.viewport {
            viewport.notify_now();
        }
    }
    pub(crate) fn reset_diagnostics(&self) {
        self.state.borrow_mut().warning_count = 0;
    }
    pub(crate) fn refresh_navigation(&self) {
        refresh_navigation(&self.state);
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn presented_frames(&self) -> u64 {
        self.state.borrow().presented_frames
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn backend_has_image(&self, key: &str) -> bool {
        let host = self.state.borrow();
        host.backend.as_ref().is_some_and(|b| b.has_image(key))
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn last_frame_trace(&self) -> String {
        self.state.borrow().backend.as_ref().map_or_else(
            || "backend unavailable".to_owned(),
            CanvasKitBackend::last_frame_trace,
        )
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn last_frame_layer_trace(&self) -> String {
        self.state.borrow().backend.as_ref().map_or_else(
            || "backend unavailable".to_owned(),
            CanvasKitBackend::last_frame_layer_trace,
        )
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn read_logical_pixel(&self, x: f32, y: f32) -> Option<[u8; 4]> {
        let host = self.state.borrow();
        let surface = host.surface.as_ref()?;
        Some(surface.read_pixel(
            (x * host.dpr).round().max(0.0) as u32,
            (y * host.dpr).round().max(0.0) as u32,
        ))
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn logical_region_has_ink(&self, x: f32, y: f32, width: f32, height: f32) -> bool {
        let host = self.state.borrow();
        host.surface.as_ref().is_some_and(|surface| {
            surface.region_has_ink(
                (x * host.dpr).round().max(0.0) as u32,
                (y * host.dpr).round().max(0.0) as u32,
                (width * host.dpr).round().max(0.0) as u32,
                (height * host.dpr).round().max(0.0) as u32,
            )
        })
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn fail_next_surface_for_test(&self) {
        if let Some(backend) = self.state.borrow().backend.as_ref() {
            backend.fail_next_surface_for_test();
        }
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn needs_paint_for_test(&self) -> bool {
        let (runtime, now) = {
            let host = self.state.borrow();
            (host.runtime.clone(), host.now_ms())
        };
        let mut live = runtime.take();
        let needs_paint = live.pump(now).needs_paint;
        runtime.put(live);
        needs_paint
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn has_pending_frame_for_test(&self) -> bool {
        let host = self.state.borrow();
        host.raf_id.is_some() || host.raf_pending_epoch.is_some()
    }

    pub(crate) fn dispose(&mut self) {
        self.viewport.take();
        let resources = {
            let mut state = self.state.borrow_mut();
            if state.disposed {
                return;
            }
            state.disposed = true;
            state.raf_pending_epoch = None;
            state.timeout_pending_epoch = None;
            state.pump_microtask_pending = false;
            (
                state.window.clone(),
                state.runtime.clone(),
                state.raf_id.take(),
                state.timeout_id.take(),
                state.surface.take(),
                state.backend.take(),
                state.ime.take(),
                state.raf_callback.take(),
                state.timeout_callback.take(),
            )
        };
        let (window, runtime, raf_id, timeout_id, surface, backend, ime, raf, timeout) = resources;
        if let Some(id) = raf_id {
            let _ = window.cancel_animation_frame(id);
        }
        if let Some(id) = timeout_id {
            window.clear_timeout_with_handle(id);
        }
        // Hidden IME listeners and the DOM input must be gone before task
        // cancellation aborts browser requests. Abort handlers are arbitrary
        // JavaScript and may synchronously probe or dispatch at the old input.
        drop(ime);
        runtime.cancel_all_tasks();
        drop((surface, backend, raf, timeout));
    }
}

impl Drop for RafPump {
    fn drop(&mut self) {
        self.dispose();
    }
}

fn install_callbacks(state: &Rc<RefCell<State>>) {
    let weak = Rc::downgrade(state);
    let raf = Closure::wrap(Box::new(move |_timestamp: f64| {
        if let Some(state) = weak.upgrade() {
            let run = {
                let mut host = state.borrow_mut();
                host.raf_id = None;
                host.raf_pending_epoch = None;
                !host.disposed
            };
            if run {
                tick(&state);
            }
        }
    }) as Box<dyn FnMut(f64)>);
    let weak = Rc::downgrade(state);
    let timeout = Closure::wrap(Box::new(move || {
        if let Some(state) = weak.upgrade() {
            let run = {
                let mut host = state.borrow_mut();
                host.timeout_id = None;
                host.timeout_pending_epoch = None;
                !host.disposed
            };
            if run {
                request_frame(&state);
            }
        }
    }) as Box<dyn FnMut()>);
    let mut state = state.borrow_mut();
    state.raf_callback = Some(raf);
    state.timeout_callback = Some(timeout);
}

fn request_frame(state: &Rc<RefCell<State>>) {
    let (window, callback, epoch) = {
        let mut host = state.borrow_mut();
        if host.disposed || host.raf_id.is_some() || host.raf_pending_epoch.is_some() {
            return;
        }
        host.raf_epoch = host.raf_epoch.wrapping_add(1).max(1);
        let epoch = host.raf_epoch;
        host.raf_pending_epoch = Some(epoch);
        let callback = host
            .raf_callback
            .as_ref()
            .expect("rAF callback installed")
            .as_ref()
            .unchecked_ref::<Function>()
            .clone();
        (host.window.clone(), callback, epoch)
    };
    let result = window.request_animation_frame(&callback);
    let cancel = {
        let mut host = state.borrow_mut();
        if host.raf_pending_epoch != Some(epoch) {
            result.ok()
        } else {
            host.raf_pending_epoch = None;
            match result {
                Ok(id) if !host.disposed => {
                    host.raf_id = Some(id);
                    None
                }
                Ok(id) => Some(id),
                Err(_) => None,
            }
        }
    };
    if let Some(id) = cancel {
        let _ = window.cancel_animation_frame(id);
    }
}

/// Poll runtime work as soon as a browser Promise or production input wakes
/// it, while leaving actual painting on rAF. Streaming image responses can
/// settle several Promise layers (fetch, then one or more reader reads)
/// between frames; deferring every poll to rAF leaves a newly admitted image
/// Pending after the host has otherwise gone idle.
fn queue_pump_microtask(state: &Rc<RefCell<State>>) {
    {
        let mut host = state.borrow_mut();
        if host.disposed || host.pump_microtask_pending {
            return;
        }
        host.pump_microtask_pending = true;
    }
    let weak = Rc::downgrade(state);
    wasm_bindgen_futures::spawn_local(async move {
        let _ =
            wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL)).await;
        let Some(state) = weak.upgrade() else {
            return;
        };
        let run = {
            let mut host = state.borrow_mut();
            host.pump_microtask_pending = false;
            !host.disposed
        };
        if run {
            pump_without_paint(&state);
        }
    });
}

fn pump_without_paint(state: &Rc<RefCell<State>>) {
    let (runtime, now) = {
        let host = state.borrow();
        if host.disposed {
            return;
        }
        (host.runtime.clone(), host.now_ms())
    };
    // Authored route actions may complete synchronously during DOM dispatch,
    // before this queued wake polls any task. Apply the router projection on
    // every production wake just as the rAF path does; otherwise a completed
    // `push` with no remaining dirty/deadline signal can go permanently idle.
    reconcile_navigation(state, &runtime);
    let mut live = runtime.take();
    let directive = live.pump(now);
    runtime.put(live);
    report_runtime(state, &runtime);
    if state.borrow().disposed {
        return;
    }
    schedule(state, directive.needs_paint, directive.next_wake_ms, now);
}

fn tick(state: &Rc<RefCell<State>>) {
    let (runtime, now) = {
        let host = state.borrow();
        if host.disposed {
            return;
        }
        (host.runtime.clone(), host.now_ms())
    };
    reconcile_navigation(state, &runtime);
    let mut live = runtime.take();
    let first = live.pump(now);
    runtime.put(live);
    report_runtime(state, &runtime);
    if state.borrow().disposed {
        return;
    }
    if first.needs_paint {
        render_if_possible(state, &runtime);
    }
    sync_ime(state);
    let mut live = runtime.take();
    let directive = live.pump(now);
    runtime.put(live);
    report_runtime(state, &runtime);
    schedule(state, directive.needs_paint, directive.next_wake_ms, now);
}

fn refresh_navigation(state: &Rc<RefCell<State>>) {
    let (runtime, previous) = {
        let mut host = state.borrow_mut();
        if host.disposed {
            return;
        }
        (host.runtime.clone(), host.navigation.take())
    };
    let mut live = runtime.take();
    let Some(table) = live.screen_table() else {
        live.nav = Rc::new(NullRouter);
        runtime.put(live);
        return;
    };
    let paths = table.paths();
    let active_path = live
        .active_screen_path()
        .unwrap_or(table.entry_path())
        .to_owned();
    let router = if let Some(previous) = previous {
        let saved = previous.router.current();
        previous.router.restore(saved, &paths);
        previous.router
    } else {
        let router = Rc::new(ScreenRouter::new(table.entry_path(), paths.clone()));
        if active_path != table.entry_path() {
            router.restore(
                RouteState {
                    path: active_path.clone(),
                    params: Default::default(),
                    query: Default::default(),
                    stack: vec![active_path.clone()],
                },
                &paths,
            );
        }
        router
    };
    live.nav = router.clone();
    let mut navigation = NavigationState {
        router,
        table,
        current_path: active_path,
    };
    match reconcile_screens_with_layout(
        &mut live,
        &navigation.router,
        &navigation.table,
        &mut navigation.current_path,
    ) {
        Ok(outcome) => {
            for rejected in outcome.rejections {
                live.push_load_warning(format!(
                    "unknown route `{}` ({}) ignored",
                    rejected.path, rejected.verb
                ));
            }
            if outcome.switched.is_some() {
                live.mark_dirty();
            }
        }
        Err(error) => live.push_layout_error(format!("route refresh failed: {error}")),
    }
    runtime.put(live);
    let mut host = state.borrow_mut();
    if !host.disposed {
        host.navigation = Some(navigation);
    }
}

fn reconcile_navigation(state: &Rc<RefCell<State>>, runtime: &RuntimeHandle) {
    let Some(mut navigation) = ({
        let mut host = state.borrow_mut();
        (!host.disposed).then(|| host.navigation.take()).flatten()
    }) else {
        return;
    };
    let mut switched = false;
    let mut live = runtime.take();
    match reconcile_screens_with_layout(
        &mut live,
        &navigation.router,
        &navigation.table,
        &mut navigation.current_path,
    ) {
        Ok(outcome) => {
            for rejected in outcome.rejections {
                live.push_load_warning(format!(
                    "unknown route `{}` ({}) ignored",
                    rejected.path, rejected.verb
                ));
            }
            if outcome.switched.is_some() {
                switched = true;
                live.mark_dirty();
            }
        }
        Err(error) => live.push_layout_error(format!("route reconcile failed: {error}")),
    }
    runtime.put(live);
    let mut host = state.borrow_mut();
    if !host.disposed {
        if switched {
            // Route projection replaces the document warning vector, so its
            // host cursor belongs to the previous document generation.
            host.warning_count = 0;
        }
        host.navigation = Some(navigation);
    }
}

fn report_runtime(state: &Rc<RefCell<State>>, runtime: &RuntimeHandle) {
    let warning_count = state.borrow().warning_count;
    let (warnings, layout_errors, outcomes, total_warnings) = {
        let mut runtime = runtime.borrow_mut();
        let total = runtime.load_warnings().len();
        let start = warning_count.min(total);
        let warnings = runtime.load_warnings()[start..].to_vec();
        let layout_errors = runtime.take_layout_errors();
        let outcomes = runtime.take_action_outcomes();
        (warnings, layout_errors, outcomes, total)
    };
    let callbacks = {
        let mut host = state.borrow_mut();
        host.warning_count = total_warnings;
        host.callbacks.clone()
    };
    for warning in warnings {
        if state.borrow().disposed {
            return;
        }
        callbacks.warning(warning);
    }
    for error in layout_errors {
        if state.borrow().disposed {
            return;
        }
        callbacks.layout_error(error);
    }
    for reported in outcomes {
        let source = reported.source.as_deref();
        for warning in reported.outcome.warnings {
            if state.borrow().disposed {
                return;
            }
            callbacks.action_warning(warning.message, source);
        }
        if let Err(error) = reported.outcome.result {
            if state.borrow().disposed {
                return;
            }
            callbacks.action_error(error.to_string(), source);
        }
    }
}

fn render_if_possible(state: &Rc<RefCell<State>>, runtime: &RuntimeHandle) {
    let commands = {
        let runtime = runtime.borrow();
        runtime.document.as_ref().map(|document| {
            if document.schema.is_responsive() {
                FramePaintCommands::Responsive(collect_scene_paint_commands_with_state(
                    document,
                    &runtime.layout,
                    &runtime.state,
                ))
            } else {
                FramePaintCommands::Legacy(collect_rich_draws_with_state(
                    document,
                    &runtime.layout,
                    &runtime.state,
                ))
            }
        })
    };
    let Some((mut backend, mut surface, generation, physical, dpr)) = ({
        let mut host = state.borrow_mut();
        if !host.paintable() {
            None
        } else {
            Some((
                host.backend.take().expect("paintable backend"),
                host.surface.take().expect("paintable surface"),
                host.backend_generation,
                host.physical,
                host.dpr,
            ))
        }
    }) else {
        return;
    };

    let mut live = runtime.take();
    live.prepare_frame(&mut backend, generation);
    runtime.put(live);
    backend.begin_frame(&mut surface, 0xffffffff);
    if let Some(commands) = commands {
        match commands {
            FramePaintCommands::Legacy(draws) => {
                let mut text_runs = draws.text_runs.into_iter().peekable();
                for (index, op) in draws.ops.into_iter().enumerate() {
                    if text_runs
                        .peek()
                        .is_some_and(|(op_index, _)| *op_index == index)
                    {
                        let (_, spans) = text_runs.next().expect("peeked rich text run");
                        if let DrawOp::Text(run) = op {
                            backend.draw_text_runs(&run, &spans);
                        } else {
                            debug_assert!(false, "rich text metadata indexed a non-text op");
                            backend.draw(&op);
                        }
                    } else {
                        backend.draw(&op);
                    }
                }
            }
            FramePaintCommands::Responsive(commands) => {
                for command in commands {
                    match command {
                        ScenePaintCommand::PushClip(rect) => backend.push_clip(rect),
                        ScenePaintCommand::PushTransform(transform) => {
                            backend.push_transform(&transform);
                        }
                        ScenePaintCommand::Pop => backend.pop(),
                        ScenePaintCommand::ApplyBlur(sigma) => backend.apply_blur(sigma),
                        ScenePaintCommand::ApplyShadow(shadow) => backend.apply_shadow(&shadow),
                        ScenePaintCommand::PushLayer(bounds) => backend.push_layer(bounds),
                        ScenePaintCommand::PopLayer => backend.pop_layer(),
                        ScenePaintCommand::Draw(op) => backend.draw(&op),
                        ScenePaintCommand::RichText { run, plan } => {
                            backend.draw_text_plan(&run, &plan);
                        }
                    }
                }
            }
        }
    }
    backend.end_frame(&mut surface);

    let generation_changed = state.borrow().backend_generation != generation;
    if generation_changed {
        backend.invalidate_images();
    }

    let (presented, recreate) = {
        let mut host = state.borrow_mut();
        if host.disposed {
            (false, false)
        } else {
            host.backend = Some(backend);
            let current = host.backend_generation == generation
                && host.physical == physical
                && (host.dpr - dpr).abs() <= 0.001
                && host.connected
                && host.logical.0 > 0.0
                && host.logical.1 > 0.0
                && !host.context_lost;
            if current {
                host.surface = Some(surface);
                (true, false)
            } else {
                (false, !host.context_lost && host.connected)
            }
        }
    };
    if presented {
        runtime.borrow_mut().frame_presented();
        let mut host = state.borrow_mut();
        if !host.disposed {
            host.presented_frames = host.presented_frames.saturating_add(1);
        }
    } else if recreate {
        let _ = recreate_surface(state);
    }
}

fn sync_ime(state: &Rc<RefCell<State>>) {
    let Some(mut ime) = ({
        let mut host = state.borrow_mut();
        (!host.disposed).then(|| host.ime.take()).flatten()
    }) else {
        return;
    };
    ime.sync_from_runtime();
    let mut host = state.borrow_mut();
    if !host.disposed {
        host.ime = Some(ime);
    }
}

fn schedule(state: &Rc<RefCell<State>>, needs_paint: bool, next_wake_ms: Option<u64>, now: u64) {
    let can_paint = state.borrow().paintable();
    if needs_paint && can_paint {
        request_frame(state);
        return;
    }
    let Some(deadline) = next_wake_ms else {
        return;
    };
    if deadline <= now {
        request_frame(state);
        return;
    }
    let delay = deadline.saturating_sub(now).min(i32::MAX as u64) as i32;
    let (window, callback, old_id, epoch) = {
        let mut host = state.borrow_mut();
        if host.disposed {
            return;
        }
        host.timeout_epoch = host.timeout_epoch.wrapping_add(1).max(1);
        let epoch = host.timeout_epoch;
        host.timeout_pending_epoch = Some(epoch);
        let callback = host
            .timeout_callback
            .as_ref()
            .expect("timeout callback installed")
            .as_ref()
            .unchecked_ref::<Function>()
            .clone();
        (host.window.clone(), callback, host.timeout_id.take(), epoch)
    };
    if let Some(id) = old_id {
        window.clear_timeout_with_handle(id);
    }
    let result = window.set_timeout_with_callback_and_timeout_and_arguments_0(&callback, delay);
    let cancel = {
        let mut host = state.borrow_mut();
        if host.timeout_pending_epoch != Some(epoch) {
            result.ok()
        } else {
            host.timeout_pending_epoch = None;
            match result {
                Ok(id) if !host.disposed => {
                    host.timeout_id = Some(id);
                    None
                }
                Ok(id) => Some(id),
                Err(_) => None,
            }
        }
    };
    if let Some(id) = cancel {
        window.clear_timeout_with_handle(id);
    }
}
