//! UI feedback actions: `toast`, `alert`, `confirm`.

use crate::action::action_trait::{ActionChain, ActionImpl, BoxedAction};
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::action::registry::ActionRegistry;
use crate::action::services::FeedbackLevel;
use crate::expression::Expression;
use async_trait::async_trait;
use serde_json::Value;

fn parse_level(s: &str) -> FeedbackLevel {
    match s {
        "success" => FeedbackLevel::Success,
        "warning" => FeedbackLevel::Warning,
        "error" => FeedbackLevel::Error,
        _ => FeedbackLevel::Info,
    }
}

struct Toast {
    message_expr: Expression,
    duration_ms: u32,
    level: FeedbackLevel,
}

#[async_trait(?Send)]
impl ActionImpl for Toast {
    fn name(&self) -> &'static str {
        "toast"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let locals = ctx.locals_snapshot();
        let (v, ws) = self.message_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in ws {
            ctx.warn(w);
        }
        ctx.feedback
            .toast(v.as_str().unwrap_or(""), self.level, self.duration_ms);
        Ok(())
    }
}

pub fn factory_toast(body: &Value) -> Result<BoxedAction, ActionError> {
    // Accept either a plain expression string (message only) or an object.
    let (msg_src, duration, level) = if let Some(s) = body.as_str() {
        (s.to_owned(), 2000, FeedbackLevel::Info)
    } else {
        let obj = body.as_object().ok_or(ActionError::FieldType {
            name: "toast",
            field: "body",
            message: "must be string or object".into(),
        })?;
        let msg = obj
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or(ActionError::MissingField {
                name: "toast",
                field: "message",
            })?
            .to_owned();
        let dur = obj
            .get("duration_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(2000) as u32;
        let lvl = obj
            .get("level")
            .and_then(|v| v.as_str())
            .map(parse_level)
            .unwrap_or(FeedbackLevel::Info);
        (msg, dur, lvl)
    };
    Ok(Box::new(Toast {
        message_expr: Expression::compile(&msg_src)?,
        duration_ms: duration,
        level,
    }))
}

struct Alert {
    title_expr: Expression,
    message_expr: Expression,
}

#[async_trait(?Send)]
impl ActionImpl for Alert {
    fn name(&self) -> &'static str {
        "alert"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let locals = ctx.locals_snapshot();
        let (t, ws) = self.title_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in ws {
            ctx.warn(w);
        }
        let (m, ws) = self.message_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in ws {
            ctx.warn(w);
        }
        ctx.feedback
            .alert(t.as_str().unwrap_or(""), m.as_str().unwrap_or(""));
        Ok(())
    }
}

pub fn factory_alert(body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "alert",
        field: "body",
        message: "must be object".into(),
    })?;
    let t = obj
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "alert",
            field: "title",
        })?;
    let m = obj
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "alert",
            field: "message",
        })?;
    Ok(Box::new(Alert {
        title_expr: Expression::compile(t)?,
        message_expr: Expression::compile(m)?,
    }))
}

pub struct Confirm {
    title_expr: Expression,
    message_expr: Expression,
    on_confirm: Option<ActionChain>,
    on_cancel: Option<ActionChain>,
}

#[async_trait(?Send)]
impl ActionImpl for Confirm {
    fn name(&self) -> &'static str {
        "confirm"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let locals = ctx.locals_snapshot();
        let (t, _) = self.title_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        let (m, _) = self.message_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        let ok = ctx
            .async_fb
            .confirm(t.as_str().unwrap_or(""), m.as_str().unwrap_or(""))
            .await;
        if ok {
            if let Some(ref c) = self.on_confirm {
                c.run_serial(ctx).await?;
            }
        } else if let Some(ref c) = self.on_cancel {
            c.run_serial(ctx).await?;
        }
        Ok(())
    }
}

pub fn make_confirm_body(reg: &ActionRegistry, body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "confirm",
        field: "body",
        message: "must be object".into(),
    })?;
    let t = obj
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "confirm",
            field: "title",
        })?;
    let m = obj
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "confirm",
            field: "message",
        })?;
    let on_confirm = obj
        .get("on_confirm")
        .map(|v| reg.parse_list(v))
        .transpose()?;
    let on_cancel = obj
        .get("on_cancel")
        .map(|v| reg.parse_list(v))
        .transpose()?;
    Ok(Box::new(Confirm {
        title_expr: Expression::compile(t)?,
        message_expr: Expression::compile(m)?,
        on_confirm,
        on_cancel,
    }))
}
