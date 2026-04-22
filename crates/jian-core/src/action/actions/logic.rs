//! Tier 3 dispatch: `call` invokes a `LogicProvider::call`.
//!
//! Currently uses the `NullLogicProvider` when no provider is installed —
//! every call errs via on_error chain (or soft warn + null return).

use crate::action::action_trait::{ActionChain, ActionImpl, BoxedAction};
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::action::registry::ActionRegistry;
use crate::expression::Expression;
use crate::state::path::StatePath;
use async_trait::async_trait;
use serde_json::Value;

pub struct Call {
    module_id: String,
    function: String,
    args: Vec<Expression>,
    into: Option<StatePath>,
    on_error: Option<ActionChain>,
}

#[async_trait(?Send)]
impl ActionImpl for Call {
    fn name(&self) -> &'static str {
        "call"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let locals = ctx.locals_snapshot();
        let mut arg_values = Vec::with_capacity(self.args.len());
        for e in &self.args {
            let (v, ws) = e.eval_with_locals(
                &ctx.state,
                ctx.page_id.as_deref(),
                ctx.node_id.as_deref(),
                &locals,
            );
            for w in ws {
                ctx.warn(w);
            }
            arg_values.push(v);
        }

        // Dispatch through the host-injected LogicProvider. When the
        // host hasn't installed one, `NullLogicProvider` returns an Err
        // from `call`, which is routed through `on_error` below.
        let _ = &self.module_id;
        let outcome = ctx.logic.call(&self.function, &arg_values);

        match outcome {
            Ok(v) => {
                if let Some(ref path) = self.into {
                    crate::action::actions::state::write_path(ctx, path, v.0)?;
                }
                Ok(())
            }
            Err(msg) => {
                ctx.warn(crate::expression::Diagnostic {
                    kind: crate::expression::DiagKind::RuntimeWarning,
                    message: format!("call: {}", msg),
                    span: crate::expression::Span::zero(),
                });
                if let Some(ref chain) = self.on_error {
                    chain.run_serial(ctx).await?;
                }
                Ok(())
            }
        }
    }
}

pub fn make_call_body(reg: &ActionRegistry, body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "call",
        field: "body",
        message: "must be object".into(),
    })?;
    let module_id = obj
        .get("module")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "call",
            field: "module",
        })?
        .to_owned();
    let function = obj
        .get("function")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "call",
            field: "function",
        })?
        .to_owned();
    let args = if let Some(arr) = obj.get("args").and_then(|v| v.as_array()) {
        arr.iter()
            .map(|v| {
                v.as_str()
                    .ok_or(ActionError::FieldType {
                        name: "call",
                        field: "<arg>",
                        message: "must be string (expression)".into(),
                    })
                    .and_then(|s| Expression::compile(s).map_err(ActionError::from))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let into = obj
        .get("into")
        .and_then(|v| v.as_str())
        .map(StatePath::parse)
        .transpose()
        .map_err(|e| ActionError::Custom(format!("call.into: {}", e)))?;
    let on_error = obj.get("on_error").map(|v| reg.parse_list(v)).transpose()?;
    Ok(Box::new(Call {
        module_id,
        function,
        args,
        into,
        on_error,
    }))
}
