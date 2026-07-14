//! DOM listeners feeding the real Jian runtime gesture entry points.

pub mod keyboard;
pub mod pointer;
pub mod wheel;

use crate::clock::HostClock;
use crate::runtime_slot::RuntimeSlot;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{AddEventListenerOptions, Event, EventTarget, HtmlCanvasElement};

pub(crate) type RuntimeHandle = Rc<RuntimeSlot>;

struct Listener {
    target: EventTarget,
    kind: &'static str,
    callback: Closure<dyn FnMut(Event)>,
}

impl Listener {
    fn attach(
        target: EventTarget,
        kind: &'static str,
        passive: Option<bool>,
        callback: impl FnMut(Event) + 'static,
    ) -> Result<Self, JsValue> {
        let callback = Closure::wrap(Box::new(callback) as Box<dyn FnMut(Event)>);
        if let Some(passive) = passive {
            let options = AddEventListenerOptions::new();
            options.set_passive(passive);
            target.add_event_listener_with_callback_and_add_event_listener_options(
                kind,
                callback.as_ref().unchecked_ref(),
                &options,
            )?;
        } else {
            target.add_event_listener_with_callback(kind, callback.as_ref().unchecked_ref())?;
        }
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

/// Owns every listener installed for one mount. Dropping it detaches all DOM
/// callbacks and restores the canvas's prior touch-action and tabindex.
pub(crate) struct EventBridge {
    canvas: HtmlCanvasElement,
    previous_touch_action: String,
    previous_tab_index: Option<String>,
    listeners: Vec<Listener>,
}

impl EventBridge {
    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn attach(
        canvas: HtmlCanvasElement,
        runtime: RuntimeHandle,
    ) -> Result<Self, JsValue> {
        Self::attach_with_clock_and_keyboard_target(
            canvas,
            runtime,
            HostClock::new()?,
            Rc::new(|| {}),
            None,
        )
    }

    pub(crate) fn attach_with_clock_and_keyboard_target(
        canvas: HtmlCanvasElement,
        runtime: RuntimeHandle,
        clock: HostClock,
        wake: Rc<dyn Fn()>,
        owned_keyboard_target: Option<EventTarget>,
    ) -> Result<Self, JsValue> {
        let previous_touch_action = canvas.style().get_property_value("touch-action")?;
        let previous_tab_index = canvas.get_attribute("tabindex");
        canvas.style().set_property("touch-action", "none")?;
        canvas.set_tab_index(0);
        let canvas_target: EventTarget = canvas.clone().into();
        let mut listeners = Vec::new();

        for (kind, phase) in [
            ("pointerdown", jian_core::gesture::PointerPhase::Down),
            ("pointermove", jian_core::gesture::PointerPhase::Move),
            ("pointerup", jian_core::gesture::PointerPhase::Up),
            ("pointercancel", jian_core::gesture::PointerPhase::Cancel),
        ] {
            let canvas_for_event = canvas.clone();
            let runtime = runtime.clone();
            let clock = clock.clone();
            let wake = wake.clone();
            listeners.push(Listener::attach(
                canvas_target.clone(),
                kind,
                None,
                move |event| {
                    let Ok(event) = event.dyn_into::<web_sys::PointerEvent>() else {
                        return;
                    };
                    let logical = runtime.viewport_size();
                    let mapped = pointer::map_pointer(
                        &event,
                        &canvas_for_event,
                        logical.0,
                        logical.1,
                        phase,
                        clock.now_ms(),
                    );
                    // Focus may scroll the document. The event's client
                    // coordinates belong to the pre-focus viewport, so map
                    // them before any focus/capture side effects can move the
                    // canvas's bounding box.
                    if phase == jian_core::gesture::PointerPhase::Down {
                        let _ = canvas_for_event.set_pointer_capture(event.pointer_id());
                        let options = web_sys::FocusOptions::new();
                        options.set_prevent_scroll(true);
                        let _ = canvas_for_event.focus_with_options(&options);
                    }
                    runtime.dispatch_pointer(mapped);
                    wake();
                },
            )?);
        }

        {
            let canvas = canvas.clone();
            let runtime = runtime.clone();
            let clock = clock.clone();
            let wake = wake.clone();
            listeners.push(Listener::attach(
                canvas_target,
                "wheel",
                Some(false),
                move |event| {
                    let Ok(event) = event.dyn_into::<web_sys::WheelEvent>() else {
                        return;
                    };
                    event.prevent_default();
                    let logical = runtime.viewport_size();
                    let mapped =
                        wheel::map_wheel(&event, &canvas, logical.0, logical.1, clock.now_ms());
                    runtime.dispatch_wheel(mapped);
                    wake();
                },
            )?);
        }

        let mut keyboard_targets: Vec<EventTarget> = vec![canvas.clone().into()];
        if let Some(target) = owned_keyboard_target {
            keyboard_targets.push(target);
        }
        for target in keyboard_targets {
            for (kind, pressed) in [("keydown", true), ("keyup", false)] {
                let runtime = runtime.clone();
                let clock = clock.clone();
                let wake = wake.clone();
                listeners.push(Listener::attach(
                    target.clone(),
                    kind,
                    None,
                    move |event| {
                        let Ok(event) = event.dyn_into::<web_sys::KeyboardEvent>() else {
                            return;
                        };
                        if event.is_composing() {
                            wake();
                            return;
                        }
                        let now = clock.now_ms();
                        if pressed
                            && runtime
                                .dispatch_keyboard(
                                    keyboard::key(&event),
                                    keyboard::modifiers(&event),
                                    now,
                                )
                                .unwrap_or(true)
                        {
                            event.prevent_default();
                        }
                        wake();
                    },
                )?);
            }
        }

        Ok(Self {
            canvas,
            previous_touch_action,
            previous_tab_index,
            listeners,
        })
    }
}

impl Drop for EventBridge {
    fn drop(&mut self) {
        self.listeners.clear();
        if self.previous_touch_action.is_empty() {
            let _ = self.canvas.style().remove_property("touch-action");
        } else {
            let _ = self
                .canvas
                .style()
                .set_property("touch-action", &self.previous_touch_action);
        }
        if let Some(previous) = &self.previous_tab_index {
            let _ = self.canvas.set_attribute("tabindex", previous);
        } else {
            let _ = self.canvas.remove_attribute("tabindex");
        }
    }
}
