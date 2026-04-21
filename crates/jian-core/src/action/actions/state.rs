//! State-mutation actions: `set`, `reset`, `delete`.

use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::expression::Expression;
use crate::state::{path::StatePath, Scope};
use async_trait::async_trait;
use serde_json::Value;

struct Set {
    pairs: Vec<(StatePath, Expression)>,
}

#[async_trait(?Send)]
impl ActionImpl for Set {
    fn name(&self) -> &'static str {
        "set"
    }

    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let locals = ctx.locals_snapshot();
        for (path, expr) in &self.pairs {
            let (v, warnings) = expr.eval_with_locals(
                &ctx.state,
                ctx.page_id.as_deref(),
                ctx.node_id.as_deref(),
                &locals,
            );
            for w in warnings {
                ctx.warn(w);
            }
            write_path(ctx, path, v.0)?;
        }
        Ok(())
    }
}

pub(crate) fn write_path(ctx: &ActionContext, path: &StatePath, value: Value) -> ActionResult {
    if path.segments.len() != 1 {
        return Err(ActionError::Custom(format!(
            "set: multi-segment paths not yet supported: `{:?}`",
            path
        )));
    }
    let key = match &path.segments[0] {
        crate::state::Segment::Key(k) => k.clone(),
        crate::state::Segment::Index(_) => {
            return Err(ActionError::Custom(
                "set: array-index write not supported".into(),
            ));
        }
    };
    match path.scope {
        Scope::App => ctx.state.app_set(&key, value),
        Scope::Vars => ctx.state.vars_set(&key, value),
        Scope::Page => {
            if let Some(pid) = &ctx.page_id {
                ctx.state.page_set(pid, &key, value);
            } else {
                return Err(ActionError::Custom(
                    "set: $page write without active page".into(),
                ));
            }
        }
        Scope::SelfNode => {
            if let Some(nid) = &ctx.node_id {
                ctx.state.self_set(nid, &key, value);
            } else {
                return Err(ActionError::Custom(
                    "set: $self write outside node context".into(),
                ));
            }
        }
        Scope::Route | Scope::Storage => {
            return Err(ActionError::Custom(format!(
                "set: {} is not directly writable; use router/storage actions",
                path.scope.as_prefix()
            )));
        }
    };
    Ok(())
}

pub fn factory_set(body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "set",
        field: "body",
        message: "must be object".into(),
    })?;

    let pairs = if let (Some(target), Some(value)) = (obj.get("target"), obj.get("value")) {
        let tgt = target.as_str().ok_or(ActionError::FieldType {
            name: "set",
            field: "target",
            message: "must be string".into(),
        })?;
        let val = value.as_str().ok_or(ActionError::FieldType {
            name: "set",
            field: "value",
            message: "must be string (expression)".into(),
        })?;
        vec![(
            StatePath::parse(tgt)
                .map_err(|e| ActionError::Custom(format!("set.target: {}", e)))?,
            Expression::compile(val)?,
        )]
    } else {
        let mut pairs = Vec::with_capacity(obj.len());
        for (k, v) in obj {
            let val_src = v.as_str().ok_or(ActionError::FieldType {
                name: "set",
                field: "<value>",
                message: "must be string (expression)".into(),
            })?;
            pairs.push((
                StatePath::parse(k)
                    .map_err(|e| ActionError::Custom(format!("set.{}: {}", k, e)))?,
                Expression::compile(val_src)?,
            ));
        }
        pairs
    };

    Ok(Box::new(Set { pairs }))
}

// ---- reset + delete ----

struct Reset {
    scope: Scope,
}

#[async_trait(?Send)]
impl ActionImpl for Reset {
    fn name(&self) -> &'static str {
        "reset"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        match self.scope {
            Scope::App => ctx.state.app.borrow_mut().clear(),
            Scope::Vars => ctx.state.vars.borrow_mut().clear(),
            Scope::Page => {
                if let Some(pid) = &ctx.page_id {
                    ctx.state.page.borrow_mut().remove(pid);
                }
            }
            Scope::SelfNode => {
                if let Some(nid) = &ctx.node_id {
                    ctx.state.self_.borrow_mut().remove(nid);
                }
            }
            _ => {
                return Err(ActionError::Custom(format!(
                    "reset: {} is not resettable",
                    self.scope.as_prefix()
                )))
            }
        }
        Ok(())
    }
}

pub fn factory_reset(body: &Value) -> Result<BoxedAction, ActionError> {
    let scope_s = body.as_str().ok_or(ActionError::FieldType {
        name: "reset",
        field: "body",
        message: "must be scope string (e.g. \"$app\")".into(),
    })?;
    let scope = Scope::parse_prefix(scope_s).ok_or(ActionError::Custom(format!(
        "reset: unknown scope `{}`",
        scope_s
    )))?;
    Ok(Box::new(Reset { scope }))
}

struct Delete {
    path: StatePath,
}

#[async_trait(?Send)]
impl ActionImpl for Delete {
    fn name(&self) -> &'static str {
        "delete"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        write_path(ctx, &self.path, serde_json::Value::Null)
    }
}

pub fn factory_delete(body: &Value) -> Result<BoxedAction, ActionError> {
    let p = body.as_str().ok_or(ActionError::FieldType {
        name: "delete",
        field: "body",
        message: "must be state path string".into(),
    })?;
    let path =
        StatePath::parse(p).map_err(|e| ActionError::Custom(format!("delete: {}", e)))?;
    Ok(Box::new(Delete { path }))
}
