//! Runtime-node visibility actions.

use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::action::services::{UiMutationOutcome, UiMutationRequest};
use async_trait::async_trait;
use serde_json::Value;

#[derive(Clone, Copy)]
enum VisibilityMode {
    Show,
    Hide,
    Toggle,
}

struct Visibility {
    name: &'static str,
    target_id: String,
    mode: VisibilityMode,
}

#[async_trait(?Send)]
impl ActionImpl for Visibility {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let request = match self.mode {
            VisibilityMode::Show => UiMutationRequest::SetVisibility {
                node_id: self.target_id.clone(),
                visible: true,
            },
            VisibilityMode::Hide => UiMutationRequest::SetVisibility {
                node_id: self.target_id.clone(),
                visible: false,
            },
            VisibilityMode::Toggle => UiMutationRequest::ToggleVisibility {
                node_id: self.target_id.clone(),
            },
        };
        emit_ui_mutation(ctx, self.name, request);
        Ok(())
    }
}

pub(crate) fn emit_ui_mutation(
    ctx: &ActionContext,
    action: &'static str,
    request: UiMutationRequest,
) {
    match ctx.ui_mutation_sink.apply(&request) {
        UiMutationOutcome::Applied(_) => {}
        UiMutationOutcome::Unsupported => warn(ctx, format!("{action}: UI mutations unavailable")),
        UiMutationOutcome::Rejected(detail) => warn(ctx, format!("{action}: {detail}")),
    }
}

fn warn(ctx: &ActionContext, message: String) {
    ctx.warn(crate::expression::Diagnostic {
        kind: crate::expression::DiagKind::RuntimeWarning,
        message,
        span: crate::expression::Span::zero(),
    });
}

fn target_id(body: &Value, action: &'static str) -> Result<String, ActionError> {
    let authored = body
        .as_str()
        .or_else(|| body.get("target").and_then(Value::as_str))
        .ok_or(ActionError::FieldType {
            name: action,
            field: "body",
            message: "must be a node id string or an object with target".into(),
        })?;
    let target = authored.trim();
    if target.is_empty() {
        return Err(ActionError::FieldType {
            name: action,
            field: "target",
            message: "must be a non-empty node id".into(),
        });
    }
    Ok(target.to_owned())
}

fn factory(
    body: &Value,
    name: &'static str,
    mode: VisibilityMode,
) -> Result<BoxedAction, ActionError> {
    Ok(Box::new(Visibility {
        name,
        target_id: target_id(body, name)?,
        mode,
    }))
}

pub fn factory_show(body: &Value) -> Result<BoxedAction, ActionError> {
    factory(body, "show", VisibilityMode::Show)
}

pub fn factory_hide(body: &Value) -> Result<BoxedAction, ActionError> {
    factory(body, "hide", VisibilityMode::Hide)
}

pub fn factory_toggle_visibility(body: &Value) -> Result<BoxedAction, ActionError> {
    factory(body, "toggle_visibility", VisibilityMode::Toggle)
}
