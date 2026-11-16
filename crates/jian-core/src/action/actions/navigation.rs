//! Navigation actions: push / replace / pop / reset / open_url.

use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::context::{ActionContext, EffectRequestContext};
use crate::action::error::{ActionError, ActionResult};
use crate::action::services::effect_sink::{EffectOutcome, EffectRequest};
use crate::capability::Capability;
use crate::expression::Expression;
use crate::motion::screen_transition::ScreenTransitionSpec;
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

async fn eval_path(ctx: &ActionContext, path_expr: &Option<Expression>) -> Option<String> {
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
    transition: Option<ScreenTransitionSpec>,
}
#[async_trait(?Send)]
impl ActionImpl for Push {
    fn name(&self) -> &'static str {
        "push"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let path = eval_path(ctx, &self.path_expr).await.unwrap_or_default();
        ctx.router.push_with(&path, self.transition.clone());
        Ok(())
    }
}

struct Replace {
    path_expr: Option<Expression>,
    transition: Option<ScreenTransitionSpec>,
}
#[async_trait(?Send)]
impl ActionImpl for Replace {
    fn name(&self) -> &'static str {
        "replace"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let path = eval_path(ctx, &self.path_expr).await.unwrap_or_default();
        ctx.router.replace_with(&path, self.transition.clone());
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

struct Pop(Option<ScreenTransitionSpec>);
#[async_trait(?Send)]
impl ActionImpl for Pop {
    fn name(&self) -> &'static str {
        "pop"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        ctx.router.pop_with(self.0.clone());
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
        if !ctx
            .capabilities
            .check(Capability::Network, "open_url", ctx.now_ms())
        {
            return Err(ActionError::CapabilityDenied {
                action: "open_url",
                needed: Capability::Network,
            });
        }
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
        let url = v.as_str().unwrap_or("");
        // R3: the host sink owns the effect; the legacy platform service
        // stays the fallback for runtimes without a sink.
        let ectx = EffectRequestContext {
            handler: ctx.handler.clone(),
            node_id: ctx.node_id.clone(),
            activation: ctx.activation,
            at_ms: ctx.now_ms(),
        };
        let request = EffectRequest::OpenUrl {
            url: url.to_owned(),
        };
        match ctx.effect_sink.request(&ectx, &request) {
            EffectOutcome::Accepted | EffectOutcome::AcceptedWithCompletion(_) => return Ok(()),
            EffectOutcome::Rejected(detail) => {
                ctx.warn(crate::expression::Diagnostic {
                    kind: crate::expression::DiagKind::RuntimeWarning,
                    message: format!("open_url: {detail}"),
                    span: crate::expression::Span::zero(),
                });
                return Ok(());
            }
            EffectOutcome::Unsupported => {}
        }
        if let Err(error) = ctx.platform.open_url(url) {
            ctx.warn(crate::expression::Diagnostic {
                kind: crate::expression::DiagKind::RuntimeWarning,
                message: format!("open_url: {error}"),
                span: crate::expression::Span::zero(),
            });
        }
        Ok(())
    }
}

fn nav_body(
    body: &Value,
) -> Result<(Option<Expression>, Option<ScreenTransitionSpec>), ActionError> {
    if let Some(object) = body.as_object() {
        let path = object.get("path").ok_or(ActionError::MissingField {
            name: "nav",
            field: "path",
        })?;
        return Ok((expr_from_value(path)?, transition_from_body(body)?));
    }
    Ok((expr_from_value(body)?, None))
}

fn transition_from_body(body: &Value) -> Result<Option<ScreenTransitionSpec>, ActionError> {
    let Some(value) = body.get("transition").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| ActionError::FieldType {
            name: "nav",
            field: "transition",
            message: error.to_string(),
        })
}

pub fn factory_push(body: &Value) -> Result<BoxedAction, ActionError> {
    let (path_expr, transition) = nav_body(body)?;
    Ok(Box::new(Push {
        path_expr,
        transition,
    }))
}
pub fn factory_replace(body: &Value) -> Result<BoxedAction, ActionError> {
    let (path_expr, transition) = nav_body(body)?;
    Ok(Box::new(Replace {
        path_expr,
        transition,
    }))
}
pub fn factory_reset_nav(body: &Value) -> Result<BoxedAction, ActionError> {
    Ok(Box::new(ResetTo {
        path_expr: expr_from_value(body)?,
    }))
}
pub fn factory_pop(body: &Value) -> Result<BoxedAction, ActionError> {
    Ok(Box::new(Pop(transition_from_body(body)?)))
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
