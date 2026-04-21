//! BindingEffect — attaches a compiled Expression to a target mutation
//! callback and registers it with the Effect registry.
//!
//! Semantics: every time any Signal read during the Expression's evaluation
//! changes, the effect re-runs, recomputes the value, and calls the target
//! callback with the new value. Integrators (scene-property wiring) provide
//! the callback.

use crate::effect::{EffectHandle, EffectRegistry};
use crate::expression::{Diagnostic, Expression};
use crate::state::StateGraph;
use crate::value::RuntimeValue;
use std::cell::RefCell;
use std::rc::Rc;

pub struct BindingEffect {
    _handle: EffectHandle,
}

impl BindingEffect {
    pub fn new(
        reg: &Rc<EffectRegistry>,
        expr: Expression,
        state: Rc<StateGraph>,
        page_id: Option<String>,
        node_id: Option<String>,
        apply: impl FnMut(RuntimeValue, Vec<Diagnostic>) + 'static,
    ) -> Self {
        let apply = RefCell::new(apply);
        let handle = reg.register(move || {
            let (v, warnings) = expr.eval(&state, page_id.as_deref(), node_id.as_deref());
            (apply.borrow_mut())(v, warnings);
        });
        Self { _handle: handle }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::EffectRegistry;
    use crate::signal::scheduler::Scheduler;
    use serde_json::json;
    use std::cell::RefCell;

    #[test]
    fn binding_updates_target_on_signal_change() {
        let sched = Rc::new(Scheduler::new());
        let reg = EffectRegistry::new();
        reg.install_on(&sched);
        let state = Rc::new(StateGraph::new(sched.clone()));
        state.app_set("count", json!(1));

        let last = Rc::new(RefCell::new(RuntimeValue::null()));
        let last2 = last.clone();
        let expr = Expression::compile("$app.count * 2").unwrap();
        let _b = BindingEffect::new(&reg, expr, state.clone(), None, None, move |v, _| {
            *last2.borrow_mut() = v;
        });

        assert_eq!(last.borrow().as_i64(), Some(2));

        state.app_set("count", json!(5));
        sched.flush();
        assert_eq!(last.borrow().as_i64(), Some(10));
    }

    #[test]
    fn binding_warnings_flow_through() {
        let sched = Rc::new(Scheduler::new());
        let reg = EffectRegistry::new();
        reg.install_on(&sched);
        let state = Rc::new(StateGraph::new(sched.clone()));

        let warns = Rc::new(RefCell::new(Vec::new()));
        let warns2 = warns.clone();
        let expr = Expression::compile("unknownFn(42)").unwrap();
        let _b = BindingEffect::new(&reg, expr, state.clone(), None, None, move |_, ws| {
            warns2.borrow_mut().extend(ws);
        });
        assert!(!warns.borrow().is_empty());
    }
}
