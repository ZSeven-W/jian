//! Aggregated action registration.

use crate::action::registry::ActionRegistry;
use std::cell::RefCell;
use std::rc::Rc;

pub mod state;

/// Register all MVP actions into a shared registry.
///
/// Recursive actions (if, for_each, parallel, race) need to re-parse nested
/// ActionLists at parse time, which requires access to the registry. We
/// achieve that by capturing a `Weak<RefCell<ActionRegistry>>` in the
/// factory closure.
pub fn register_all(reg: &Rc<RefCell<ActionRegistry>>) {
    let _weak = Rc::downgrade(reg);
    let mut r = reg.borrow_mut();

    // State
    r.register("set", Box::new(state::factory_set));
    r.register("reset", Box::new(state::factory_reset));
    r.register("delete", Box::new(state::factory_delete));
}
