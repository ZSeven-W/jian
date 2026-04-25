//! Aggregated action registration.

use crate::action::action_trait::BoxedAction;
use crate::action::error::ActionError;
use crate::action::registry::ActionRegistry;
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

pub mod control;
pub mod feedback;
pub mod logic;
pub mod navigation;
pub mod network;
pub mod platform;
pub mod state;
pub mod storage_ops;
pub mod websocket;

/// Register all MVP actions into a shared registry.
pub fn register_all(reg: &Rc<RefCell<ActionRegistry>>) {
    let weak = Rc::downgrade(reg);
    let mut r = reg.borrow_mut();

    // State
    r.register("set", Box::new(state::factory_set));
    r.register("delete", Box::new(state::factory_delete));

    // `reset` is dual-purpose (spec §3.2).
    r.register(
        "reset",
        Box::new(|body: &Value| -> Result<BoxedAction, ActionError> {
            if let Some(s) = body.as_str() {
                if s.starts_with('$') {
                    return state::factory_reset(body);
                }
            }
            navigation::factory_reset_nav(body)
        }),
    );

    // Control (non-nested)
    r.register("abort", Box::new(control::factory_abort));
    r.register("delay", Box::new(control::factory_delay));

    // Navigation
    r.register("push", Box::new(navigation::factory_push));
    r.register("replace", Box::new(navigation::factory_replace));
    r.register("pop", Box::new(navigation::factory_pop));
    r.register("open_url", Box::new(navigation::factory_open_url));

    // Storage
    r.register("storage_set", Box::new(storage_ops::factory_storage_set));
    r.register(
        "storage_clear",
        Box::new(storage_ops::factory_storage_clear),
    );
    r.register("storage_wipe", Box::new(storage_ops::factory_storage_wipe));

    // UI feedback (non-nested)
    r.register("toast", Box::new(feedback::factory_toast));
    r.register("alert", Box::new(feedback::factory_alert));

    // WebSocket
    r.register("ws_connect", Box::new(websocket::factory_ws_connect));
    r.register("ws_send", Box::new(websocket::factory_ws_send));
    r.register("ws_close", Box::new(websocket::factory_ws_close));

    // L4 platform stubs
    r.register("vibrate", Box::new(platform::factory_vibrate));
    r.register("haptic", Box::new(platform::factory_haptic));
    r.register("share", Box::new(platform::factory_share));
    r.register("notify", Box::new(platform::factory_notify));
    r.register("focus", Box::new(platform::factory_focus));
    r.register("blur", Box::new(platform::factory_blur));

    // Control (nested — via weak registry upgrade)
    let w = weak.clone();
    r.register(
        "if",
        Box::new(move |body| {
            let s = w.upgrade().ok_or(ActionError::Custom(
                "registry dropped while parsing `if`".into(),
            ))?;
            let r = s.borrow();
            control::make_if_body(&r, body)
        }),
    );
    let w = weak.clone();
    r.register(
        "for_each",
        Box::new(move |body| {
            let s = w.upgrade().ok_or(ActionError::Custom(
                "registry dropped while parsing `for_each`".into(),
            ))?;
            let r = s.borrow();
            control::make_for_each_body(&r, body)
        }),
    );
    let w = weak.clone();
    r.register(
        "parallel",
        Box::new(move |body| {
            let s = w.upgrade().ok_or(ActionError::Custom(
                "registry dropped while parsing `parallel`".into(),
            ))?;
            let r = s.borrow();
            control::make_parallel_body(&r, body)
        }),
    );
    let w = weak.clone();
    r.register(
        "race",
        Box::new(move |body| {
            let s = w.upgrade().ok_or(ActionError::Custom(
                "registry dropped while parsing `race`".into(),
            ))?;
            let r = s.borrow();
            control::make_race_body(&r, body)
        }),
    );

    // Network (nested — fetch.on_error)
    let w = weak.clone();
    r.register(
        "fetch",
        Box::new(move |body| {
            let s = w.upgrade().ok_or(ActionError::Custom(
                "registry dropped while parsing `fetch`".into(),
            ))?;
            let r = s.borrow();
            network::make_fetch_body(&r, body)
        }),
    );

    // UI feedback (nested — confirm.on_confirm/on_cancel)
    let w = weak.clone();
    r.register(
        "confirm",
        Box::new(move |body| {
            let s = w.upgrade().ok_or(ActionError::Custom(
                "registry dropped while parsing `confirm`".into(),
            ))?;
            let r = s.borrow();
            feedback::make_confirm_body(&r, body)
        }),
    );

    // Tier 3 `call` (nested — on_error)
    let w = weak;
    r.register(
        "call",
        Box::new(move |body| {
            let s = w.upgrade().ok_or(ActionError::Custom(
                "registry dropped while parsing `call`".into(),
            ))?;
            let r = s.borrow();
            logic::make_call_body(&r, body)
        }),
    );
}
