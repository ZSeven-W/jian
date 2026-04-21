//! Navigation actions: push / replace / pop / reset / open_url.

use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::expression::Expression;
use async_trait::async_trait;
use serde_json::Value;

fn expr_from_value(body: &Value) -> Result<Option<Expression>, ActionError> {
    match body {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(Expression::compile(s)?)),
        _ => Err(ActionError::FieldType {
            name: "nav",
            field: "body",
            message: "must be string (expression) or null".into(),
        }),
    }
}

async fn eval_path(
    ctx: &ActionContext,
    path_expr: &Option<Expression>,
) -> Option<String> {
    let expr = path_expr.as_ref()?;
    let locals = ctx.locals_snapshot();
    let (v, ws) = expr.eval_with_locals(
        &ctx.state,
        ctx.page_id.as_deref(),
        ctx.node_id.as_deref(),
        &locals,
    );
    for w in ws {
        ctx.warn(w);
    }
    v.as_str().map(|s| s.to_owned())
}

struct Push {
    path_expr: Option<Expression>,
}
#[async_trait(?Send)]
impl ActionImpl for Push {
    fn name(&self) -> &'static str {
        "push"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let path = eval_path(ctx, &self.path_expr).await.unwrap_or_default();
        ctx.router.push(&path);
        Ok(())
    }
}

struct Replace {
    path_expr: Option<Expression>,
}
#[async_trait(?Send)]
impl ActionImpl for Replace {
    fn name(&self) -> &'static str {
        "replace"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let path = eval_path(ctx, &self.path_expr).await.unwrap_or_default();
        ctx.router.replace(&path);
        Ok(())
    }
}

struct ResetTo {
    path_expr: Option<Expression>,
}
#[async_trait(?Send)]
impl ActionImpl for ResetTo {
    fn name(&self) -> &'static str {
        "reset"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let path = eval_path(ctx, &self.path_expr).await.unwrap_or_default();
        ctx.router.reset(&path);
        Ok(())
    }
}

struct Pop;
#[async_trait(?Send)]
impl ActionImpl for Pop {
    fn name(&self) -> &'static str {
        "pop"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        ctx.router.pop();
        Ok(())
    }
}

struct OpenUrl {
    url_expr: Expression,
}
#[async_trait(?Send)]
impl ActionImpl for OpenUrl {
    fn name(&self) -> &'static str {
        "open_url"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let locals = ctx.locals_snapshot();
        let (v, ws) = self.url_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in ws {
            ctx.warn(w);
        }
        ctx.warn(crate::expression::Diagnostic {
            kind: crate::expression::DiagKind::RuntimeWarning,
            message: format!("open_url: {}", v.as_str().unwrap_or("<non-string>")),
            span: crate::expression::Span::zero(),
        });
        Ok(())
    }
}

pub fn factory_push(body: &Value) -> Result<BoxedAction, ActionError> {
    Ok(Box::new(Push {
        path_expr: expr_from_value(body)?,
    }))
}
pub fn factory_replace(body: &Value) -> Result<BoxedAction, ActionError> {
    Ok(Box::new(Replace {
        path_expr: expr_from_value(body)?,
    }))
}
pub fn factory_reset_nav(body: &Value) -> Result<BoxedAction, ActionError> {
    Ok(Box::new(ResetTo {
        path_expr: expr_from_value(body)?,
    }))
}
pub fn factory_pop(_body: &Value) -> Result<BoxedAction, ActionError> {
    Ok(Box::new(Pop))
}
pub fn factory_open_url(body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "open_url",
        field: "body",
        message: "must be object".into(),
    })?;
    let src = obj
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "open_url",
            field: "url",
        })?;
    Ok(Box::new(OpenUrl {
        url_expr: Expression::compile(src)?,
    }))
}
