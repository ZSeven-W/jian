//! ResizeObserver, DPR media-query, and WebGL context lifecycle wiring.

use js_sys::Array;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Event, EventTarget, HtmlCanvasElement, MediaQueryList, ResizeObserver, Window};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewportMetrics {
    pub width: f32,
    pub height: f32,
    pub dpr: f32,
    pub connected: bool,
}

struct MediaHandle {
    query: MediaQueryList,
    callback: Closure<dyn FnMut()>,
}

impl Drop for MediaHandle {
    fn drop(&mut self) {
        let _ = self
            .query
            .remove_listener_with_opt_callback(Some(self.callback.as_ref().unchecked_ref()));
    }
}

struct DomListener {
    target: EventTarget,
    kind: &'static str,
    callback: Closure<dyn FnMut(Event)>,
}

impl Drop for DomListener {
    fn drop(&mut self) {
        let _ = self
            .target
            .remove_event_listener_with_callback(self.kind, self.callback.as_ref().unchecked_ref());
    }
}

struct Inner {
    window: Window,
    canvas: HtmlCanvasElement,
    on_metrics: Rc<dyn Fn(ViewportMetrics)>,
    media: RefCell<Option<MediaHandle>>,
    observer: ResizeObserver,
    observer_callback: Closure<dyn FnMut(Array, ResizeObserver)>,
    context_listeners: Vec<DomListener>,
}

impl Inner {
    fn notify(&self) {
        (self.on_metrics)(metrics(&self.window, &self.canvas));
    }

    fn install_media(self: &Rc<Self>) -> Result<(), JsValue> {
        let previous = self.media.borrow_mut().take();
        drop(previous);
        let dpr = self.window.device_pixel_ratio().max(1.0);
        let query = self
            .window
            .match_media(&format!("(resolution: {dpr}dppx)"))?
            .ok_or_else(|| JsValue::from_str("matchMedia returned null"))?;
        let weak = Rc::downgrade(self);
        let callback = Closure::wrap(Box::new(move || {
            if let Some(inner) = weak.upgrade() {
                let _ = inner.install_media();
                inner.notify();
            }
        }) as Box<dyn FnMut()>);
        query.add_listener_with_opt_callback(Some(callback.as_ref().unchecked_ref()))?;
        *self.media.borrow_mut() = Some(MediaHandle { query, callback });
        Ok(())
    }
}

pub(crate) struct ViewportBridge(Rc<Inner>);

/// Resize the canvas backing store without letting intrinsic canvas dimensions
/// feed back into an otherwise unstyled CSS box. An axis is pinned only when
/// the corresponding attribute write demonstrably changed its rendered size.
pub(crate) fn resize_backing_store_preserving_css_box(
    canvas: &HtmlCanvasElement,
    width: u32,
    height: u32,
) -> Result<(), JsValue> {
    let width = width.max(1);
    let height = height.max(1);
    if canvas.width() == width && canvas.height() == height {
        return Ok(());
    }

    let before = canvas.get_bounding_client_rect();
    if canvas.width() != width {
        canvas.set_width(width);
    }
    if canvas.height() != height {
        canvas.set_height(height);
    }
    let after = canvas.get_bounding_client_rect();
    let style = canvas.style();
    if (after.width() - before.width()).abs() > 0.5 {
        style.set_property("width", &format!("{}px", before.width()))?;
    }
    if (after.height() - before.height()).abs() > 0.5 {
        style.set_property("height", &format!("{}px", before.height()))?;
    }
    Ok(())
}

impl ViewportBridge {
    pub(crate) fn attach(
        canvas: HtmlCanvasElement,
        on_metrics: Rc<dyn Fn(ViewportMetrics)>,
        on_context_lost: Rc<dyn Fn()>,
        on_context_restored: Rc<dyn Fn()>,
    ) -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("window unavailable"))?;
        let inner = Rc::<Inner>::new_cyclic(|weak| {
            let weak_observer = weak.clone();
            let observer_callback =
                Closure::wrap(Box::new(move |_entries: Array, _observer: ResizeObserver| {
                    if let Some(inner) = weak_observer.upgrade() {
                        inner.notify();
                    }
                }) as Box<dyn FnMut(Array, ResizeObserver)>);
            let observer = ResizeObserver::new(observer_callback.as_ref().unchecked_ref())
                .expect("ResizeObserver construction failed");
            observer.observe(&canvas);

            let target: EventTarget = canvas.clone().into();
            let lost = {
                let callback = Closure::wrap(Box::new(move |event: Event| {
                    event.prevent_default();
                    on_context_lost();
                }) as Box<dyn FnMut(Event)>);
                target
                    .add_event_listener_with_callback(
                        "webglcontextlost",
                        callback.as_ref().unchecked_ref(),
                    )
                    .expect("context-lost listener failed");
                DomListener {
                    target: target.clone(),
                    kind: "webglcontextlost",
                    callback,
                }
            };
            let restored = {
                let callback = Closure::wrap(Box::new(move |_event: Event| {
                    on_context_restored();
                }) as Box<dyn FnMut(Event)>);
                target
                    .add_event_listener_with_callback(
                        "webglcontextrestored",
                        callback.as_ref().unchecked_ref(),
                    )
                    .expect("context-restored listener failed");
                DomListener {
                    target,
                    kind: "webglcontextrestored",
                    callback,
                }
            };
            Inner {
                window: window.clone(),
                canvas: canvas.clone(),
                on_metrics,
                media: RefCell::new(None),
                observer,
                observer_callback,
                context_listeners: vec![lost, restored],
            }
        });
        inner.install_media()?;
        inner.notify();
        Ok(Self(inner))
    }

    pub(crate) fn notify_now(&self) {
        self.0.notify();
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.observer.disconnect();
        let media = self.media.borrow_mut().take();
        drop(media);
        self.context_listeners.clear();
        let _ = &self.observer_callback;
    }
}

fn metrics(window: &Window, canvas: &HtmlCanvasElement) -> ViewportMetrics {
    let bounds = canvas.get_bounding_client_rect();
    ViewportMetrics {
        width: bounds.width().max(0.0) as f32,
        height: bounds.height().max(0.0) as f32,
        dpr: window.device_pixel_ratio().max(1.0) as f32,
        connected: canvas.is_connected(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn physical_size_rounding_contract() {
        assert_eq!((100.4_f32 * 2.0).round() as u32, 201);
    }
}
