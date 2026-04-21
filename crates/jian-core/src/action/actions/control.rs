//! Control-flow actions: `if`, `delay`, `abort`, `for_each`, `parallel`, `race`.

use crate::action::action_trait::{ActionChain, ActionImpl, BoxedAction};
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::action::registry::ActionRegistry;
use crate::expression::Expression;
use async_trait::async_trait;
use serde_json::Value;

// ---- abort ----

struct Abort;

#[async_trait(?Send)]
impl ActionImpl for Abort {
    fn name(&self) -> &'static str {
        "abort"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        ctx.cancel.cancel();
        Err(ActionError::Aborted)
    }
}

pub fn factory_abort(_body: &Value) -> Result<BoxedAction, ActionError> {
    Ok(Box::new(Abort))
}

// ---- delay ----

struct Delay {
    _ms: u64,
}

#[async_trait(?Send)]
impl ActionImpl for Delay {
    fn name(&self) -> &'static str {
        "delay"
    }
    async fn execute(&self, _ctx: &ActionContext) -> ActionResult {
        // MVP: instantaneous. Real timer service arrives with host adapter.
        Ok(())
    }
}

pub fn factory_delay(body: &Value) -> Result<BoxedAction, ActionError> {
    let ms_val = body.get("ms").ok_or(ActionError::MissingField {
        name: "delay",
        field: "ms",
    })?;
    let ms = ms_val.as_u64().ok_or(ActionError::FieldType {
        name: "delay",
        field: "ms",
        message: "must be positive integer".into(),
    })?;
    Ok(Box::new(Delay { _ms: ms }))
}

// ---- if ----

pub(crate) struct If {
    condition: Expression,
    then_chain: ActionChain,
    else_chain: Option<ActionChain>,
}

#[async_trait(?Send)]
impl ActionImpl for If {
    fn name(&self) -> &'static str {
        "if"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let locals = ctx.locals_snapshot();
        let (v, ws) = self.condition.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in ws {
            ctx.warn(w);
        }
        if is_truthy(&v.0) {
            self.then_chain.run_serial(ctx).await
        } else if let Some(ref else_chain) = self.else_chain {
            else_chain.run_serial(ctx).await
        } else {
            Ok(())
        }
    }
}

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub fn make_if_body(reg: &ActionRegistry, body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "if",
        field: "body",
        message: "must be object".into(),
    })?;
    let expr_src = obj
        .get("expr")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "if",
            field: "expr",
        })?;
    let then_list = obj.get("then").ok_or(ActionError::MissingField {
        name: "if",
        field: "then",
    })?;
    let then_chain = reg.parse_list(then_list)?;
    let else_chain = body.get("else").map(|el| reg.parse_list(el)).transpose()?;
    Ok(Box::new(If {
        condition: Expression::compile(expr_src)?,
        then_chain,
        else_chain,
    }))
}

// ---- for_each ----

struct ForEach {
    in_expr: Expression,
    as_name: String,
    body: ActionChain,
    max_iter: usize,
}

#[async_trait(?Send)]
impl ActionImpl for ForEach {
    fn name(&self) -> &'static str {
        "for_each"
    }

    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let locals = ctx.locals_snapshot();
        let (v, ws) = self.in_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in ws {
            ctx.warn(w);
        }
        let arr = match &v.0 {
            Value::Array(a) => a.clone(),
            _ => {
                return Err(ActionError::FieldType {
                    name: "for_each",
                    field: "in",
                    message: "must evaluate to array".into(),
                })
            }
        };
        if arr.len() > self.max_iter {
            return Err(ActionError::Custom(format!(
                "for_each: {} iterations exceeds limit {}",
                arr.len(),
                self.max_iter
            )));
        }
        for (i, item) in arr.into_iter().enumerate() {
            if ctx.cancel.is_cancelled() {
                return Err(ActionError::Aborted);
            }
            ctx.push_local(self.as_name.clone(), crate::value::RuntimeValue(item));
            ctx.push_local("index", crate::value::RuntimeValue::from_i64(i as i64));
            let r = self.body.run_serial(ctx).await;
            ctx.pop_local(&self.as_name);
            ctx.pop_local("index");
            r?;
        }
        Ok(())
    }
}

pub fn make_for_each_body(reg: &ActionRegistry, body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "for_each",
        field: "body",
        message: "must be object".into(),
    })?;
    let in_src = obj
        .get("in")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "for_each",
            field: "in",
        })?;
    let as_name = obj
        .get("as")
        .and_then(|v| v.as_str())
        .unwrap_or("item")
        .to_owned();
    let body_list = obj.get("do").ok_or(ActionError::MissingField {
        name: "for_each",
        field: "do",
    })?;

    Ok(Box::new(ForEach {
        in_expr: Expression::compile(in_src)?,
        as_name,
        body: reg.parse_list(body_list)?,
        max_iter: 10_000,
    }))
}

// ---- parallel ----

struct Parallel {
    chains: Vec<ActionChain>,
}

#[async_trait(?Send)]
impl ActionImpl for Parallel {
    fn name(&self) -> &'static str {
        "parallel"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        let futs = self.chains.iter().map(|c| c.run_serial(ctx));
        let results = futures::future::join_all(futs).await;
        for r in results {
            r?;
        }
        Ok(())
    }
}

pub fn make_parallel_body(reg: &ActionRegistry, body: &Value) -> Result<BoxedAction, ActionError> {
    let arr = body.as_array().ok_or(ActionError::FieldType {
        name: "parallel",
        field: "body",
        message: "must be array of ActionLists".into(),
    })?;
    let mut chains = Vec::with_capacity(arr.len());
    for item in arr {
        let list = if item.is_array() {
            item.clone()
        } else {
            serde_json::json!([item])
        };
        chains.push(reg.parse_list(&list)?);
    }
    Ok(Box::new(Parallel { chains }))
}

// ---- race ----

struct Race {
    chains: Vec<ActionChain>,
}

#[async_trait(?Send)]
impl ActionImpl for Race {
    fn name(&self) -> &'static str {
        "race"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        use futures::future::{select_all, FutureExt};
        let futs: Vec<_> = self
            .chains
            .iter()
            .map(|c| c.run_serial(ctx).boxed_local())
            .collect();
        if futs.is_empty() {
            return Ok(());
        }
        let (first, _, _) = select_all(futs).await;
        first
    }
}

pub fn make_race_body(reg: &ActionRegistry, body: &Value) -> Result<BoxedAction, ActionError> {
    let arr = body.as_array().ok_or(ActionError::FieldType {
        name: "race",
        field: "body",
        message: "must be array".into(),
    })?;
    let mut chains = Vec::with_capacity(arr.len());
    for item in arr {
        let list = if item.is_array() {
            item.clone()
        } else {
            serde_json::json!([item])
        };
        chains.push(reg.parse_list(&list)?);
    }
    Ok(Box::new(Race { chains }))
}
