//! Top-level execute facade. Parses a JSON ActionList and executes it via
//! `futures::executor::block_on`, returning the final result + any warnings.

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
/// Blocks until all actions resolve (including fetches & delays).
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

    let result: ActionResult = futures::executor::block_on(async { chain.run_serial(ctx).await });

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
