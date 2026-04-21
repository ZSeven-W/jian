//! Aggregated action registration.

use crate::action::action_trait::BoxedAction;
use crate::action::error::ActionError;
use crate::action::registry::ActionRegistry;
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

pub mod control;
pub mod navigation;
pub mod state;

/// Register all MVP actions into a shared registry.
pub fn register_all(reg: &Rc<RefCell<ActionRegistry>>) {
    let weak = Rc::downgrade(reg);
    let mut r = reg.borrow_mut();

    // State
    r.register("set", Box::new(state::factory_set));
    r.register("delete", Box::new(state::factory_delete));

    // `reset` is dual-purpose per spec §3.2:
    //   - string body starting with `$` → state scope reset
    //   - anything else that is an expression string → navigation reset
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
    let w = weak;
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
}
