//! Network action: `fetch` with loading/into/on_error/capability gate.

use crate::action::action_trait::{ActionChain, ActionImpl, BoxedAction};
use crate::action::capability::Capability;
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::action::registry::ActionRegistry;
use crate::action::services::{HttpRequest, HttpResponse};
use crate::expression::Expression;
use crate::state::path::StatePath;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;

struct LoadingGuard {
    state: std::rc::Rc<crate::state::StateGraph>,
    path: StatePath,
    page_id: Option<String>,
    node_id: Option<String>,
    armed: bool,
    cancel: crate::action::CancellationToken,
    compensation_id: u64,
}

impl Drop for LoadingGuard {
    fn drop(&mut self) {
        self.cancel.unregister_compensation(self.compensation_id);
        if !self.armed || self.path.segments.len() != 1 {
            return;
        }
        let crate::state::Segment::Key(key) = &self.path.segments[0] else {
            return;
        };
        use crate::state::Scope;
        match self.path.scope {
            Scope::App => self.state.app_set(key, Value::Bool(false)),
            Scope::Vars => self.state.vars_set(key, Value::Bool(false)),
            Scope::Page => {
                if let Some(page) = &self.page_id {
                    self.state.page_set(page, key, Value::Bool(false));
                }
            }
            Scope::SelfNode => {
                if let Some(node) = &self.node_id {
                    self.state.self_set(
                        self.page_id.as_deref().unwrap_or(""),
                        node,
                        key,
                        Value::Bool(false),
                    );
                }
            }
            _ => {}
        }
    }
}

pub struct Fetch {
    url_expr: Expression,
    method: String,
    headers_expr: BTreeMap<String, Expression>,
    body_expr: Option<Expression>,
    into: Option<StatePath>,
    loading: Option<StatePath>,
    on_error: Option<ActionChain>,
    timeout_ms: Option<u64>,
}

#[async_trait(?Send)]
impl ActionImpl for Fetch {
    fn name(&self) -> &'static str {
        "fetch"
    }

    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        if !ctx
            .capabilities
            .check(Capability::Network, "fetch", ctx.now_ms())
        {
            return Err(ActionError::CapabilityDenied {
                action: "fetch",
                needed: Capability::Network,
            });
        }

        if let Some(ref path) = self.loading {
            crate::action::actions::state::write_path(ctx, path, Value::Bool(true))?;
        }
        let mut loading_guard = self.loading.clone().map(|path| {
            let compensation_id = ctx.cancel.register_false_write(
                path.clone(),
                ctx.page_id.clone(),
                ctx.node_id.clone(),
            );
            LoadingGuard {
                state: ctx.state.clone(),
                path,
                page_id: ctx.page_id.clone(),
                node_id: ctx.node_id.clone(),
                armed: true,
                cancel: ctx.cancel.clone(),
                compensation_id,
            }
        });

        let locals = ctx.locals_snapshot();
        let (url_v, ws) = self.url_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in ws {
            ctx.warn(w);
        }
        let url = url_v.as_str().unwrap_or("").to_owned();

        let mut headers = BTreeMap::new();
        for (k, ex) in &self.headers_expr {
            let (v, ws) = ex.eval_with_locals(
                &ctx.state,
                ctx.page_id.as_deref(),
                ctx.node_id.as_deref(),
                &locals,
            );
            for w in ws {
                ctx.warn(w);
            }
            if let Some(s) = v.as_str() {
                headers.insert(k.clone(), s.to_owned());
            }
        }

        let body = if let Some(ref e) = self.body_expr {
            let (v, ws) = e.eval_with_locals(
                &ctx.state,
                ctx.page_id.as_deref(),
                ctx.node_id.as_deref(),
                &locals,
            );
            for w in ws {
                ctx.warn(w);
            }
            Some(v.0)
        } else {
            None
        };

        let req = HttpRequest {
            url,
            method: self.method.clone(),
            headers,
            body,
            timeout_ms: self.timeout_ms,
        };

        let outcome = ctx.network.request(req).await;

        if let Some(ref path) = self.loading {
            crate::action::actions::state::write_path(ctx, path, Value::Bool(false))?;
        }
        if let Some(guard) = &mut loading_guard {
            guard.armed = false;
        }

        match outcome {
            Ok(HttpResponse { body, .. }) => {
                if let Some(ref path) = self.into {
                    crate::action::actions::state::write_path(ctx, path, body)?;
                }
                Ok(())
            }
            Err(msg) => {
                if let Some(ref chain) = self.on_error {
                    chain.run_serial(ctx).await?;
                    Ok(())
                } else {
                    Err(ActionError::Network(msg))
                }
            }
        }
    }
}

pub fn make_fetch_body(reg: &ActionRegistry, body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "fetch",
        field: "body",
        message: "must be object".into(),
    })?;
    let url_src = obj
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "fetch",
            field: "url",
        })?;
    let method = obj
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_owned();
    let mut headers_expr = BTreeMap::new();
    if let Some(h) = obj.get("headers").and_then(|v| v.as_object()) {
        for (k, vv) in h {
            if let Some(s) = vv.as_str() {
                headers_expr.insert(k.clone(), Expression::compile(s)?);
            }
        }
    }
    let body_expr = if let Some(b) = obj.get("body") {
        if let Some(s) = b.as_str() {
            Some(Expression::compile(s)?)
        } else if b.is_null() {
            None
        } else {
            let s = serde_json::to_string(b).unwrap();
            Some(Expression::compile(&s)?)
        }
    } else {
        None
    };
    let into = obj
        .get("into")
        .and_then(|v| v.as_str())
        .map(StatePath::parse)
        .transpose()
        .map_err(|e| ActionError::Custom(format!("fetch.into: {}", e)))?;
    let loading = obj
        .get("loading")
        .and_then(|v| v.as_str())
        .map(StatePath::parse)
        .transpose()
        .map_err(|e| ActionError::Custom(format!("fetch.loading: {}", e)))?;
    let on_error = obj.get("on_error").map(|v| reg.parse_list(v)).transpose()?;
    let timeout_ms = obj.get("timeout_ms").and_then(|v| v.as_u64());

    Ok(Box::new(Fetch {
        url_expr: Expression::compile(url_src)?,
        method,
        headers_expr,
        body_expr,
        into,
        loading,
        on_error,
        timeout_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::capability::DummyCapabilityGate;
    use crate::action::context::tests::make_ctx;
    use crate::action::services::{HttpRequest, NetworkClient};
    use crate::action::{default_registry, ExecOutcome};
    use async_trait::async_trait;
    use std::rc::Rc;

    struct FailingNetwork;

    #[async_trait(?Send)]
    impl NetworkClient for FailingNetwork {
        async fn request(&self, _request: HttpRequest) -> Result<HttpResponse, String> {
            Err("offline".to_owned())
        }
    }

    #[test]
    fn handlerless_failure_aborts_serial_chain() {
        futures::executor::block_on(async {
            let mut ctx = make_ctx();
            ctx.network = Rc::new(FailingNetwork);
            ctx.capabilities = Rc::new(DummyCapabilityGate);
            let list = serde_json::json!([
                {"fetch": {"url": "'https://example.invalid'"}},
                {"set": {"$app.continued": "true"}}
            ]);

            let registry = default_registry();
            let chain = registry.borrow().parse_list(&list).unwrap();
            let outcome = ExecOutcome {
                result: chain.run_serial(&ctx).await,
                warnings: ctx.take_warnings(),
            };
            assert!(matches!(outcome.result, Err(ActionError::Network(_))));
            assert!(ctx.state.app_get("continued").is_none());
        });
    }

    #[test]
    fn authored_error_handler_recovers_and_chain_continues() {
        futures::executor::block_on(async {
            let mut ctx = make_ctx();
            ctx.network = Rc::new(FailingNetwork);
            ctx.capabilities = Rc::new(DummyCapabilityGate);
            let list = serde_json::json!([
                {"fetch": {
                    "url": "'https://example.invalid'",
                    "on_error": [{"set": {"$app.caught": "true"}}]
                }},
                {"set": {"$app.continued": "true"}}
            ]);

            let registry = default_registry();
            let chain = registry.borrow().parse_list(&list).unwrap();
            let outcome = ExecOutcome {
                result: chain.run_serial(&ctx).await,
                warnings: ctx.take_warnings(),
            };
            assert!(outcome.result.is_ok());
            assert_eq!(
                ctx.state.app_get("caught").unwrap().0,
                serde_json::json!(true)
            );
            assert_eq!(
                ctx.state.app_get("continued").unwrap().0,
                serde_json::json!(true)
            );
        });
    }
}
