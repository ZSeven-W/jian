//! Trigger a named manual animation on a node or its descendants.

use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::action::services::AnimationOutcome;
use crate::expression::{DiagKind, Diagnostic, Span};
use crate::motion::MotionControl;
use async_trait::async_trait;
use serde_json::Value;

struct PlayMotion(MotionControl);

#[async_trait(?Send)]
impl ActionImpl for PlayMotion {
    fn name(&self) -> &'static str {
        "play_motion"
    }

    async fn execute(&self, context: &ActionContext) -> ActionResult {
        let message = match context.animation_sink.control(&self.0, context.now_ms()) {
            AnimationOutcome::Accepted => return Ok(()),
            AnimationOutcome::Unsupported => "animation sink unavailable".into(),
            AnimationOutcome::Rejected(detail) => detail,
        };
        context.warn(Diagnostic {
            kind: DiagKind::RuntimeWarning,
            message: format!("play_motion: {message}"),
            span: Span::zero(),
        });
        Ok(())
    }
}

pub fn factory_play_motion(body: &Value) -> Result<BoxedAction, ActionError> {
    let control: MotionControl =
        serde_json::from_value(body.clone()).map_err(|error| ActionError::FieldType {
            name: "play_motion",
            field: "body",
            message: error.to_string(),
        })?;
    for (field, value) in [
        ("target", &control.target),
        ("animation", &control.animation),
    ] {
        if value.trim().is_empty() {
            return Err(ActionError::FieldType {
                name: "play_motion",
                field,
                message: "must be a non-empty id".into(),
            });
        }
    }
    Ok(Box::new(PlayMotion(control)))
}
