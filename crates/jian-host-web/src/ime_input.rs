//! Hidden DOM input used as the browser IME composition target.

use crate::clock::HostClock;
use crate::event::RuntimeHandle;
use jian_core::gesture::{ImeEvent, ImeKind};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Event, EventTarget, HtmlCanvasElement, HtmlInputElement};

struct Listener {
    target: EventTarget,
    kind: &'static str,
    callback: Closure<dyn FnMut(Event)>,
}

impl Listener {
    fn new(
        target: EventTarget,
        kind: &'static str,
        callback: impl FnMut(Event) + 'static,
    ) -> Result<Self, JsValue> {
        let callback = Closure::wrap(Box::new(callback) as Box<dyn FnMut(Event)>);
        target.add_event_listener_with_callback(kind, callback.as_ref().unchecked_ref())?;
        Ok(Self {
            target,
            kind,
            callback,
        })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self
            .target
            .remove_event_listener_with_callback(self.kind, self.callback.as_ref().unchecked_ref());
    }
}

pub(crate) struct ImeInput {
    listeners: Vec<Listener>,
    input: HtmlInputElement,
    canvas: HtmlCanvasElement,
    runtime: RuntimeHandle,
    focused: bool,
}

impl ImeInput {
    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn attach(
        canvas: &HtmlCanvasElement,
        runtime: RuntimeHandle,
    ) -> Result<Self, JsValue> {
        Self::attach_with_clock(canvas, runtime, HostClock::new()?, Rc::new(|| {}))
    }

    pub(crate) fn attach_with_clock(
        canvas: &HtmlCanvasElement,
        runtime: RuntimeHandle,
        clock: HostClock,
        wake: Rc<dyn Fn()>,
    ) -> Result<Self, JsValue> {
        let document = canvas
            .owner_document()
            .ok_or_else(|| JsValue::from_str("canvas has no owner document"))?;
        let input: HtmlInputElement = document.create_element("input")?.dyn_into()?;
        input.set_type("text");
        input.set_attribute("aria-hidden", "true")?;
        input.set_attribute("tabindex", "-1")?;
        input.set_attribute("autocomplete", "off")?;
        input.set_attribute("autocorrect", "off")?;
        input.set_attribute("autocapitalize", "off")?;
        input.set_attribute("spellcheck", "false")?;
        input.set_attribute(
            "style",
            "position:fixed;opacity:0;border:0;padding:0;margin:0;pointer-events:none;z-index:-1;",
        )?;
        if let Some(parent) = canvas.parent_node() {
            parent.append_child(&input)?;
        } else {
            document
                .body()
                .ok_or_else(|| JsValue::from_str("document body unavailable"))?
                .append_child(&input)?;
        }

        let target: EventTarget = input.clone().into();
        let composing = Rc::new(Cell::new(false));
        let ignore_next_input = Rc::new(Cell::new(false));
        let mut listeners = Vec::new();

        {
            let runtime = runtime.clone();
            let clock = clock.clone();
            let composing = composing.clone();
            let input = input.clone();
            let wake = wake.clone();
            listeners.push(Listener::new(
                target.clone(),
                "compositionstart",
                move |_event| {
                    composing.set(true);
                    input.set_value("");
                    dispatch_ime(
                        &runtime,
                        clock.now_ms(),
                        ImeKind::CompositionStart,
                        String::new(),
                    );
                    wake();
                },
            )?);
        }
        {
            let runtime = runtime.clone();
            let clock = clock.clone();
            let wake = wake.clone();
            listeners.push(Listener::new(
                target.clone(),
                "compositionupdate",
                move |event| {
                    let Ok(event) = event.dyn_into::<web_sys::CompositionEvent>() else {
                        return;
                    };
                    dispatch_ime(
                        &runtime,
                        clock.now_ms(),
                        ImeKind::CompositionUpdate { selection: None },
                        event.data().unwrap_or_default(),
                    );
                    wake();
                },
            )?);
        }
        {
            let runtime = runtime.clone();
            let clock = clock.clone();
            let composing = composing.clone();
            let ignore_next_input = ignore_next_input.clone();
            let input = input.clone();
            let wake = wake.clone();
            listeners.push(Listener::new(
                target.clone(),
                "compositionend",
                move |event| {
                    let Ok(event) = event.dyn_into::<web_sys::CompositionEvent>() else {
                        return;
                    };
                    composing.set(false);
                    ignore_next_input.set(true);
                    dispatch_ime(
                        &runtime,
                        clock.now_ms(),
                        ImeKind::CompositionEnd,
                        event.data().unwrap_or_default(),
                    );
                    input.set_value("");
                    wake();
                },
            )?);
        }
        {
            let runtime = runtime.clone();
            let clock = clock.clone();
            let composing = composing.clone();
            let ignore_next_input = ignore_next_input.clone();
            let input = input.clone();
            let wake = wake.clone();
            listeners.push(Listener::new(target, "input", move |_event| {
                if composing.get() {
                    wake();
                    return;
                }
                if ignore_next_input.replace(false) {
                    input.set_value("");
                    wake();
                    return;
                }
                let text = input.value();
                input.set_value("");
                if text.is_empty() {
                    wake();
                    return;
                }
                runtime.dispatch_text(text, clock.now_ms());
                wake();
            })?);
        }

        Ok(Self {
            listeners,
            input,
            canvas: canvas.clone(),
            runtime,
            focused: false,
        })
    }

    pub(crate) fn keyboard_target(&self) -> EventTarget {
        self.input.clone().into()
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn input(&self) -> &HtmlInputElement {
        &self.input
    }

    /// Synchronize focus and position after layout/paint. Position is fixed in
    /// CSS pixels so the browser anchors its candidate window over the field.
    pub(crate) fn sync_from_runtime(&mut self) {
        let runtime = self.runtime.borrow();
        let Some(rect) = runtime.focused_node_rect() else {
            drop(runtime);
            self.set_focused(false);
            return;
        };
        let viewport = runtime.viewport.size;
        drop(runtime);
        // Focusing a DOM input is allowed to scroll the document. Focus first,
        // then sample the canvas box so the fixed-position candidate anchor is
        // expressed in the post-focus viewport coordinate space.
        self.set_focused(true);
        let bounds = self.canvas.get_bounding_client_rect();
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            self.set_focused(false);
            return;
        }
        let sx = bounds.width() / f64::from(viewport.width.max(1.0));
        let sy = bounds.height() / f64::from(viewport.height.max(1.0));
        let style = self.input.style();
        let _ = style.set_property("left", &px(bounds.left() + f64::from(rect.origin.x) * sx));
        let _ = style.set_property("top", &px(bounds.top() + f64::from(rect.origin.y) * sy));
        let _ = style.set_property("width", &px(f64::from(rect.size.width) * sx));
        let _ = style.set_property("height", &px(f64::from(rect.size.height) * sy));
    }

    fn set_focused(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if focused {
            let options = web_sys::FocusOptions::new();
            options.set_prevent_scroll(true);
            let _ = self.input.focus_with_options(&options);
        } else {
            let _ = self.input.blur();
        }
    }
}

impl Drop for ImeInput {
    fn drop(&mut self) {
        self.listeners.clear();
        self.input.remove();
    }
}

fn dispatch_ime(runtime: &RuntimeHandle, now: u64, kind: ImeKind, text: String) {
    runtime.dispatch_ime(ImeEvent { kind, text }, now);
}

fn px(value: f64) -> String {
    if (value - value.round()).abs() < 0.001 {
        format!("{}px", value.round() as i64)
    } else {
        format!("{value:.3}px")
    }
}

#[cfg(test)]
mod tests {
    use super::px;

    #[test]
    fn css_pixel_format_is_stable() {
        assert_eq!(px(20.0), "20px");
        assert_eq!(px(1.23456), "1.235px");
    }
}
