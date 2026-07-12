//! Compatibility execute facade for callers outside `Runtime`.

use super::context::ActionContext;
use super::error::ActionResult;
use super::registry::ActionRegistry;
use crate::expression::Diagnostic;
use serde_json::Value;

pub struct ExecOutcome {
    pub result: ActionResult,
    pub warnings: Vec<Diagnostic>,
}

pub async fn execute_list_async(
    registry: &ActionRegistry,
    list: &Value,
    ctx: &ActionContext,
) -> ExecOutcome {
    let chain = match registry.parse_list(list) {
        Ok(c) => c,
        Err(e) => {
            return ExecOutcome {
                result: Err(e),
                warnings: ctx.take_warnings(),
            }
        }
    };

    let result = chain.run_serial(ctx).await;

    ExecOutcome {
        result,
        warnings: ctx.take_warnings(),
    }
}
