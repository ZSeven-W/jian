//! Compatibility execute facade for callers outside `Runtime`.

use super::context::ActionContext;
use super::error::{ActionError, ActionResult};
use super::registry::ActionRegistry;
use crate::expression::Diagnostic;
use serde_json::Value;

pub struct ExecOutcome {
    pub result: ActionResult,
    pub warnings: Vec<Diagnostic>,
}

/// Parse + execute a JSON ActionList blob in the given ActionContext.
/// Runtime-owned event chains use `TaskQueue` and are resumed by
/// `Runtime::pump`; this compatibility facade accepts only immediately-ready
/// chains and reports (rather than silently dropping) a parked future.
pub fn execute_list(registry: &ActionRegistry, list: &Value, ctx: &ActionContext) -> ExecOutcome {
    let chain = match registry.parse_list(list) {
        Ok(c) => c,
        Err(e) => {
            return ExecOutcome {
                result: Err(e),
                warnings: ctx.take_warnings(),
            }
        }
    };

    use std::task::{Context, Poll};
    let mut future = chain.run_serial(ctx);
    let waker = futures::task::noop_waker();
    let mut task_context = Context::from_waker(&waker);
    let result: ActionResult = match future.as_mut().poll(&mut task_context) {
        Poll::Ready(result) => result,
        Poll::Pending => Err(ActionError::Custom(
            "asynchronous action list must be dispatched through Runtime's TaskQueue".into(),
        )),
    };

    ExecOutcome {
        result,
        warnings: ctx.take_warnings(),
    }
}

/// Shared-registry convenience wrapper.
pub fn execute_list_shared(
    reg: &std::rc::Rc<std::cell::RefCell<ActionRegistry>>,
    list: &Value,
    ctx: &ActionContext,
) -> ExecOutcome {
    execute_list(&reg.borrow(), list, ctx)
}

#[allow(dead_code)]
fn _type_check(_: ActionError) {}
