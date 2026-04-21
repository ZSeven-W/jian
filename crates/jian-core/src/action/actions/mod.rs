//! Aggregated action registration.

use crate::action::registry::ActionRegistry;
use std::cell::RefCell;
use std::rc::Rc;

pub mod control;
pub mod state;

/// Register all MVP actions into a shared registry.
///
/// Recursive actions (if, for_each, parallel, race) need to re-parse nested
/// ActionLists at parse time. They capture a `Weak<RefCell<ActionRegistry>>`
/// in the factory closure so they can look up inner actions without forming
/// an Rc cycle.
pub fn register_all(reg: &Rc<RefCell<ActionRegistry>>) {
    let weak = Rc::downgrade(reg);
    let mut r = reg.borrow_mut();

    // State
    r.register("set", Box::new(state::factory_set));
    r.register("reset", Box::new(state::factory_reset));
    r.register("delete", Box::new(state::factory_delete));

    // Control (non-nested)
    r.register("abort", Box::new(control::factory_abort));
    r.register("delay", Box::new(control::factory_delay));

    // Control (nested — via weak registry upgrade)
    let w = weak.clone();
    r.register(
        "if",
        Box::new(move |body| {
            let s = w
                .upgrade()
                .ok_or(crate::action::error::ActionError::Custom(
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
            let s = w
                .upgrade()
                .ok_or(crate::action::error::ActionError::Custom(
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
            let s = w
                .upgrade()
                .ok_or(crate::action::error::ActionError::Custom(
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
            let s = w
                .upgrade()
                .ok_or(crate::action::error::ActionError::Custom(
                    "registry dropped while parsing `race`".into(),
                ))?;
            let r = s.borrow();
            control::make_race_body(&r, body)
        }),
    );
}
