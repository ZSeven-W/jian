//! wasm-bindgen mount handle and FIFO hot-reload queue.

use crate::clock::HostClock;
use crate::event::EventBridge;
use crate::ime_input::ImeInput;
use crate::raf_pump::RafPump;
use crate::runtime_slot::RuntimeSlot;
use crate::services::{AbortRegistry, AssetPolicy, WebImageResolver, WebServices};
use crate::CanvasKitBackend;
use js_sys::{Array, ArrayBuffer, Function, Object, Promise, Reflect, Uint8Array, JSON};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::HtmlCanvasElement;

#[derive(Clone, Default)]
pub(crate) struct Callbacks {
    warning: Option<Function>,
    error: Option<Function>,
}

impl Callbacks {
    pub(crate) fn warning(&self, message: impl AsRef<str>) {
        let message = message.as_ref();
        let source = message
            .strip_prefix("image `")
            .and_then(|rest| rest.split('`').next())
            .filter(|source| !source.is_empty())
            .or_else(|| message.contains("route").then_some("route"))
            .unwrap_or("runtime");
        call_callback(&self.warning, "warning", message, Some(source));
    }

    pub(crate) fn action_warning(&self, message: impl AsRef<str>, source: Option<&str>) {
        call_callback(&self.warning, "warning", message.as_ref(), source);
    }

    pub(crate) fn error(&self, message: impl AsRef<str>) {
        call_callback(
            &self.error,
            "internal",
            message.as_ref(),
            Some("setDocument"),
        );
    }

    pub(crate) fn action_error(&self, message: impl AsRef<str>, source: Option<&str>) {
        call_callback(&self.error, "action", message.as_ref(), source);
    }

    pub(crate) fn surface_error(&self, message: impl AsRef<str>) {
        call_callback(&self.error, "internal", message.as_ref(), Some("surface"));
    }

    pub(crate) fn layout_error(&self, message: impl AsRef<str>) {
        let message = message.as_ref();
        let source = if message.contains("viewport") {
            "viewport"
        } else if message.contains("route") {
            "route"
        } else if message.contains("binding") {
            "binding"
        } else {
            "layout"
        };
        call_callback(&self.error, "layout", message, Some(source));
    }
}

struct FontOption {
    family: String,
    bytes: Vec<u8>,
}

struct MountOptions {
    canvas_kit_base: String,
    asset_base: Option<String>,
    fonts: Vec<FontOption>,
    callbacks: Callbacks,
}

struct MountedHost {
    runtime: Rc<RuntimeSlot>,
    pump: RafPump,
    events: Option<EventBridge>,
    services: Option<WebServices>,
    callbacks: Callbacks,
}

impl MountedHost {
    fn dispose(&mut self) {
        let services = self.services.take();
        let cleanup = services.as_ref().map(|services| services.aborts.cleanup());
        if let Some(cleanup) = &cleanup {
            cleanup.begin();
        }
        self.events.take();
        // Dropping TaskQueue futures first exercises their AbortLease /
        // TimeoutLease compensation. WebServices::drop is only a final sweep
        // for resources not owned by an action future.
        self.pump.dispose();
        if let Some(services) = services {
            services.aborts.abort_all();
            drop(services);
        }
        if let Some(cleanup) = &cleanup {
            cleanup.finish();
        }
    }
}

impl Drop for MountedHost {
    fn drop(&mut self) {
        self.dispose();
    }
}

struct QueuedDocument {
    value: JsValue,
    resolve: Function,
    reject: Function,
}

struct HandleState {
    host: Option<MountedHost>,
    queue: VecDeque<QueuedDocument>,
    processing: bool,
    disposed: bool,
    in_flight_reject: Option<Function>,
}

#[wasm_bindgen]
pub struct JianHandle {
    inner: Rc<RefCell<HandleState>>,
}

#[wasm_bindgen]
impl JianHandle {
    #[wasm_bindgen(js_name = setDocument)]
    pub fn set_document(&self, document: JsValue) -> Promise {
        let inner = self.inner.clone();
        Promise::new(&mut move |resolve, reject| {
            if inner.borrow().disposed {
                reject_error(&reject, "Jian handle is disposed");
                return;
            }
            let start = {
                let mut state = inner.borrow_mut();
                state.queue.push_back(QueuedDocument {
                    value: document.clone(),
                    resolve,
                    reject,
                });
                if state.processing {
                    false
                } else {
                    state.processing = true;
                    true
                }
            };
            if start {
                process_queue(inner.clone());
            }
        })
    }

    pub fn dispose(&self) {
        dispose_handle(&self.inner);
    }
}

impl Drop for JianHandle {
    fn drop(&mut self) {
        dispose_handle(&self.inner);
    }
}

fn dispose_handle(inner: &Rc<RefCell<HandleState>>) {
    let error = js_sys::Error::new("Jian handle was disposed");
    let (in_flight, queued, mut host) = {
        let mut state = inner.borrow_mut();
        if state.disposed {
            return;
        }
        state.disposed = true;
        state.processing = false;
        (
            state.in_flight_reject.take(),
            state.queue.drain(..).collect::<Vec<_>>(),
            state.host.take(),
        )
    };
    if let Some(reject) = in_flight {
        let _ = reject.call1(&JsValue::UNDEFINED, error.as_ref());
    }
    for queued in queued {
        let _ = queued.reject.call1(&JsValue::UNDEFINED, error.as_ref());
    }
    if let Some(host) = host.as_mut() {
        host.dispose();
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
impl JianHandle {
    pub(crate) fn test_runtime(&self) -> Option<Rc<RuntimeSlot>> {
        self.inner
            .borrow()
            .host
            .as_ref()
            .map(|host| host.runtime.clone())
    }

    pub(crate) fn test_presented_frames(&self) -> u64 {
        self.inner
            .borrow()
            .host
            .as_ref()
            .map_or(0, |host| host.pump.presented_frames())
    }

    pub(crate) fn test_backend_has_image(&self, key: &str) -> bool {
        self.inner
            .borrow()
            .host
            .as_ref()
            .is_some_and(|host| host.pump.backend_has_image(key))
    }

    pub(crate) fn test_last_frame_trace(&self) -> String {
        self.inner.borrow().host.as_ref().map_or_else(
            || "host unavailable".to_owned(),
            |host| host.pump.last_frame_trace(),
        )
    }

    pub(crate) fn test_last_frame_layer_trace(&self) -> String {
        self.inner.borrow().host.as_ref().map_or_else(
            || "host unavailable".to_owned(),
            |host| host.pump.last_frame_layer_trace(),
        )
    }

    pub(crate) fn test_read_pixel(&self, x: f32, y: f32) -> Option<[u8; 4]> {
        self.inner
            .borrow()
            .host
            .as_ref()
            .and_then(|host| host.pump.read_logical_pixel(x, y))
    }

    pub(crate) fn test_region_has_ink(&self, x: f32, y: f32, width: f32, height: f32) -> bool {
        self.inner
            .borrow()
            .host
            .as_ref()
            .is_some_and(|host| host.pump.logical_region_has_ink(x, y, width, height))
    }

    pub(crate) fn test_fail_next_surface(&self) {
        if let Some(host) = self.inner.borrow().host.as_ref() {
            host.pump.fail_next_surface_for_test();
        }
    }

    pub(crate) fn test_needs_paint(&self) -> bool {
        self.inner
            .borrow()
            .host
            .as_ref()
            .is_some_and(|host| host.pump.needs_paint_for_test())
    }

    pub(crate) fn test_has_pending_frame(&self) -> bool {
        self.inner
            .borrow()
            .host
            .as_ref()
            .is_some_and(|host| host.pump.has_pending_frame_for_test())
    }

    pub(crate) fn test_disposed(&self) -> bool {
        self.inner.borrow().disposed
    }
}

#[wasm_bindgen]
pub async fn mount_jian(
    canvas: HtmlCanvasElement,
    document: JsValue,
    options: JsValue,
) -> Result<JianHandle, JsValue> {
    let options = parse_options(options)?;
    let raw = document_json(&document)?;
    validate_document(&raw)?;
    let aborts = AbortRegistry::default();
    let clock = HostClock::new()?;
    let asset_policy = options
        .asset_base
        .as_deref()
        .map(AssetPolicy::parse)
        .transpose()
        .map_err(|error| JsValue::from_str(&error))?;

    let backend = CanvasKitBackend::load(canvas.clone(), &options.canvas_kit_base).await?;
    #[cfg(all(test, target_arch = "wasm32"))]
    let backend = {
        let mut backend = backend;
        backend.preserve_drawing_buffer_for_test();
        backend
    };
    let fonts = backend.font_registry();
    for font in &options.fonts {
        fonts
            .register(&font.family, &font.bytes)
            .map_err(|error| JsValue::from_str(&error))?;
    }
    let measure = backend.measure_backend();
    let mut runtime = jian_core::Runtime::new();
    runtime.enable_action_reporting();
    runtime.set_now_ms(clock.now_ms());
    runtime.image_resolver = Rc::new(WebImageResolver::new(
        asset_policy,
        aborts.clone(),
        Rc::new(|| {}),
    ));
    runtime
        .load_str(&raw)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let bounds = canvas.get_bounding_client_rect();
    let width = (bounds.width() as f32).max(1.0);
    let height = (bounds.height() as f32).max(1.0);
    runtime
        .build_layout_with(Rc::new(measure), (width, height))
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    runtime.set_viewport_size((width, height));
    let runtime = Rc::new(RuntimeSlot::new(runtime));
    let pump = RafPump::start_with_clock(
        canvas.clone(),
        runtime.clone(),
        backend,
        None,
        clock.clone(),
        options.callbacks.clone(),
    )?;
    pump.refresh_navigation();
    let wake = pump.waker();
    let ime = ImeInput::attach_with_clock(&canvas, runtime.clone(), clock.clone(), wake.clone())?;
    let ime_keyboard_target = ime.keyboard_target();
    pump.set_ime(ime);
    let mut live = runtime.take();
    let services = WebServices::install(
        &mut live,
        options.asset_base.as_deref(),
        aborts,
        wake.clone(),
    );
    runtime.put(live);
    let services = services.map_err(|error| JsValue::from_str(&error))?;
    let events = EventBridge::attach_with_clock_and_keyboard_target(
        canvas,
        runtime.clone(),
        clock,
        wake,
        Some(ime_keyboard_target),
    )?;
    pump.sync_viewport_now();
    let host = MountedHost {
        runtime,
        pump,
        events: Some(events),
        services: Some(services),
        callbacks: options.callbacks,
    };
    Ok(JianHandle {
        inner: Rc::new(RefCell::new(HandleState {
            host: Some(host),
            queue: VecDeque::new(),
            processing: false,
            disposed: false,
            in_flight_reject: None,
        })),
    })
}

fn process_queue(inner: Rc<RefCell<HandleState>>) {
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            let queued = {
                let mut state = inner.borrow_mut();
                if state.disposed {
                    state.processing = false;
                    return;
                }
                let Some(queued) = state.queue.pop_front() else {
                    state.processing = false;
                    return;
                };
                state.in_flight_reject = Some(queued.reject.clone());
                queued
            };
            // Keep the public operation observably cancellable between FIFO
            // dequeue and its synchronous atomic commit. This is the sole
            // in-flight window now that image settlement is pump-owned.
            let _ = wasm_bindgen_futures::JsFuture::from(Promise::resolve(&JsValue::NULL)).await;
            if inner.borrow().disposed {
                return;
            }
            let result = apply_document(inner.clone(), queued.value);
            let disposed = inner.borrow().disposed;
            inner.borrow_mut().in_flight_reject = None;
            if disposed {
                continue;
            }
            match result {
                Ok(()) => {
                    let _ = queued
                        .resolve
                        .call1(&JsValue::UNDEFINED, &JsValue::UNDEFINED);
                }
                Err(error) => {
                    let callbacks = inner
                        .borrow()
                        .host
                        .as_ref()
                        .map(|host| host.callbacks.clone());
                    if let Some(callbacks) = callbacks {
                        let message = error.as_string().unwrap_or_else(|| format!("{error:?}"));
                        if message.starts_with("layout:") {
                            callbacks.layout_error(message);
                        } else {
                            callbacks.error(message);
                        }
                    }
                    let _ = queued.reject.call1(&JsValue::UNDEFINED, &error);
                }
            }
        }
    });
}

fn apply_document(inner: Rc<RefCell<HandleState>>, document: JsValue) -> Result<(), JsValue> {
    let raw = document_json(&document)?;
    validate_document(&raw)?;
    if inner.borrow().disposed {
        return Err(JsValue::from_str("Jian handle is disposed"));
    }
    let mut host = inner
        .borrow_mut()
        .host
        .take()
        .ok_or_else(|| JsValue::from_str("Jian handle is disposed"))?;
    let cleanup = host
        .services
        .as_ref()
        .map(|services| services.aborts.cleanup());
    if let Some(cleanup) = &cleanup {
        cleanup.begin();
    }
    let mut runtime = host.runtime.take();
    let result = runtime
        .load_str_and_relayout(&raw)
        .map_err(|error| JsValue::from_str(&error.to_string()));
    host.runtime.put(runtime);
    if let Some(cleanup) = &cleanup {
        // AbortController.abort() and clearTimeout() may synchronously enter
        // authored JS. Flush only after the Runtime RefMut is gone and while
        // the host is detached from HandleState, so dispose/setDocument can
        // safely re-enter.
        cleanup.finish();
    }
    if inner.borrow().disposed {
        host.dispose();
        return Err(JsValue::from_str("Jian handle is disposed"));
    }
    if let Err(error) = result {
        inner.borrow_mut().host = Some(host);
        return Err(error);
    }
    host.pump.reset_diagnostics();
    host.pump.refresh_navigation();
    host.pump.sync_viewport_now();
    host.pump.wake_soon();
    if inner.borrow().disposed {
        host.dispose();
        return Err(JsValue::from_str("Jian handle is disposed"));
    }
    inner.borrow_mut().host = Some(host);
    Ok(())
}

fn document_json(document: &JsValue) -> Result<String, JsValue> {
    if let Some(text) = document.as_string() {
        return Ok(text);
    }
    JSON::stringify(document)?
        .as_string()
        .ok_or_else(|| JsValue::from_str("document is not JSON-serializable"))
}

fn validate_document(document: &str) -> Result<(), JsValue> {
    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(document)
        .map_err(|error| JsValue::from_str(&format!("invalid document: {error}")))
}

fn parse_options(value: JsValue) -> Result<MountOptions, JsValue> {
    let value = if value.is_null() || value.is_undefined() {
        Object::new().into()
    } else {
        value
    };
    let canvas_kit =
        string_property(&value, "canvasKitUrl").unwrap_or_else(|| "/assets/canvaskit/".to_owned());
    let canvas_kit_base = if canvas_kit.ends_with(".js") {
        canvas_kit
            .rsplit_once('/')
            .map_or_else(|| "./".to_owned(), |(base, _)| format!("{base}/"))
    } else if canvas_kit.ends_with('/') {
        canvas_kit
    } else {
        format!("{canvas_kit}/")
    };
    let callbacks = Callbacks {
        warning: function_property(&value, "onWarning"),
        error: function_property(&value, "onError"),
    };
    let mut fonts = Vec::new();
    let font_value = Reflect::get(&value, &JsValue::from_str("fonts"))?;
    if Array::is_array(&font_value) {
        for entry in Array::from(&font_value).iter() {
            let family = string_property(&entry, "family")
                .ok_or_else(|| JsValue::from_str("font family is required"))?;
            let data = Reflect::get(&entry, &JsValue::from_str("data"))?;
            let buffer: ArrayBuffer = data
                .dyn_into()
                .map_err(|_| JsValue::from_str("font data must be an ArrayBuffer"))?;
            fonts.push(FontOption {
                family,
                bytes: Uint8Array::new(&buffer).to_vec(),
            });
        }
    }
    Ok(MountOptions {
        canvas_kit_base,
        asset_base: string_property(&value, "assetBase"),
        fonts,
        callbacks,
    })
}

fn string_property(value: &JsValue, name: &str) -> Option<String> {
    Reflect::get(value, &JsValue::from_str(name))
        .ok()
        .and_then(|value| value.as_string())
}

fn function_property(value: &JsValue, name: &str) -> Option<Function> {
    Reflect::get(value, &JsValue::from_str(name))
        .ok()
        .and_then(|value| value.dyn_into().ok())
}

fn call_callback(callback: &Option<Function>, kind: &str, message: &str, source: Option<&str>) {
    let Some(callback) = callback else { return };
    let payload = Object::new();
    let _ = Reflect::set(
        &payload,
        &JsValue::from_str("kind"),
        &JsValue::from_str(kind),
    );
    if let Some(source) = source {
        let _ = Reflect::set(
            &payload,
            &JsValue::from_str("source"),
            &JsValue::from_str(source),
        );
    }
    let _ = Reflect::set(
        &payload,
        &JsValue::from_str("message"),
        &JsValue::from_str(message),
    );
    let _ = callback.call1(&JsValue::UNDEFINED, &payload);
}

fn reject_error(reject: &Function, message: &str) {
    let error = js_sys::Error::new(message);
    let _ = reject.call1(&JsValue::UNDEFINED, error.as_ref());
}
