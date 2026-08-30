//! Typed scroll request action.

use super::visibility::emit_ui_mutation;
use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::action::services::{ScrollAlignment, UiMutationRequest};
use async_trait::async_trait;
use serde_json::Value;

struct ScrollTo {
    target_id: String,
    alignment: ScrollAlignment,
}

#[async_trait(?Send)]
impl ActionImpl for ScrollTo {
    fn name(&self) -> &'static str {
        "scroll_to"
    }

    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        emit_ui_mutation(
            ctx,
            "scroll_to",
            UiMutationRequest::ScrollTo {
                target_id: self.target_id.clone(),
                alignment: self.alignment,
            },
        );
        Ok(())
    }
}

pub fn factory_scroll_to(body: &Value) -> Result<BoxedAction, ActionError> {
    let object = body.as_object().ok_or(ActionError::FieldType {
        name: "scroll_to",
        field: "body",
        message: "must be an object".into(),
    })?;
    let target = object
        .get("target")
        .and_then(Value::as_str)
        .ok_or(ActionError::MissingField {
            name: "scroll_to",
            field: "target",
        })?
        .trim();
    if target.is_empty() {
        return Err(ActionError::FieldType {
            name: "scroll_to",
            field: "target",
            message: "must be a non-empty node id".into(),
        });
    }
    let authored_alignment = object
        .get("alignment")
        .or_else(|| object.get("align"))
        .and_then(Value::as_str)
        .unwrap_or("nearest");
    let alignment = ScrollAlignment::parse(authored_alignment).ok_or(ActionError::FieldType {
        name: "scroll_to",
        field: "alignment",
        message: "must be start, center, end, or nearest".into(),
    })?;
    Ok(Box::new(ScrollTo {
        target_id: target.to_owned(),
        alignment,
    }))
}
