use super::{queue_pump_microtask, State};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Event, MediaQueryList, Window};

pub(super) struct MotionPreferenceListener {
    query: MediaQueryList,
    on_change: Closure<dyn FnMut(Event)>,
}

impl Drop for MotionPreferenceListener {
    fn drop(&mut self) {
        let _ = self
            .query
            .remove_event_listener_with_callback("change", self.on_change.as_ref().unchecked_ref());
    }
}

impl MotionPreferenceListener {
    pub(super) fn install(
        state: &Rc<RefCell<State>>,
        window: &Window,
    ) -> Result<Option<Self>, JsValue> {
        if let Some(query) = window.match_media("(prefers-reduced-motion: reduce)")? {
            update_motion_preference(state, query.matches());
            let weak = Rc::downgrade(state);
            let media = query.clone();
            let on_change = Closure::wrap(Box::new(move |_: Event| {
                if let Some(state) = weak.upgrade() {
                    update_motion_preference(&state, media.matches());
                }
            }) as Box<dyn FnMut(Event)>);
            query.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())?;
            Ok(Some(Self { query, on_change }))
        } else {
            Ok(None)
        }
    }
}

fn update_motion_preference(state: &Rc<RefCell<State>>, reduced: bool) {
    let runtime = state.borrow().runtime.clone();
    let mut live = runtime.take();
    live.set_standalone_reduce_motion(reduced);
    runtime.put(live);
    queue_pump_microtask(state);
}
