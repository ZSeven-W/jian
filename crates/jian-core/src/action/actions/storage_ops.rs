//! Storage actions: `storage_set`, `storage_clear`, `storage_wipe`.

use crate::action::action_trait::{ActionChain, ActionImpl, BoxedAction};
use crate::action::capability::Capability;
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::expression::Expression;
use async_trait::async_trait;
use serde_json::Value;

pub struct StorageSet {
    pairs: Vec<(String, Expression)>,
}

#[async_trait(?Send)]
impl ActionImpl for StorageSet {
    fn name(&self) -> &'static str {
        "storage_set"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        if !ctx
            .capabilities
            .check(Capability::Storage, "storage_set", ctx.now_ms())
        {
            return Err(ActionError::CapabilityDenied {
                action: "storage_set",
                needed: Capability::Storage,
            });
        }
        let locals = ctx.locals_snapshot();
        for (key, expr) in &self.pairs {
            let (v, ws) = expr.eval_with_locals(
                &ctx.state,
                ctx.page_id.as_deref(),
                ctx.node_id.as_deref(),
                &locals,
            );
            for w in ws {
                ctx.warn(w);
            }
            let value = v.0;
            ctx.storage
                .set(key, value.clone())
                .await
                .map_err(|error| ActionError::Custom(format!("storage_set: {error}")))?;
            if ctx.state.is_responsive() {
                ctx.state.storage_set(key, value);
            }
        }
        Ok(())
    }
}

pub fn factory_storage_set(body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "storage_set",
        field: "body",
        message: "must be object of key → expression".into(),
    })?;
    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj {
        let src = v.as_str().ok_or(ActionError::FieldType {
            name: "storage_set",
            field: "<value>",
            message: "must be string (expression)".into(),
        })?;
        pairs.push((k.clone(), Expression::compile(src)?));
    }
    Ok(Box::new(StorageSet { pairs }))
}

struct StorageClear {
    key: String,
}

#[async_trait(?Send)]
impl ActionImpl for StorageClear {
    fn name(&self) -> &'static str {
        "storage_clear"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        if !ctx
            .capabilities
            .check(Capability::Storage, "storage_clear", ctx.now_ms())
        {
            return Err(ActionError::CapabilityDenied {
                action: "storage_clear",
                needed: Capability::Storage,
            });
        }
        ctx.storage
            .delete(&self.key)
            .await
            .map_err(|error| ActionError::Custom(format!("storage_clear: {error}")))?;
        if ctx.state.is_responsive() {
            ctx.state.storage_cache.remove(&self.key);
        }
        Ok(())
    }
}

pub fn factory_storage_clear(body: &Value) -> Result<BoxedAction, ActionError> {
    let key = body
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: "storage_clear",
            field: "key",
        })?
        .to_owned();
    Ok(Box::new(StorageClear { key }))
}

struct StorageWipe {
    on_error: Option<ActionChain>,
}

#[async_trait(?Send)]
impl ActionImpl for StorageWipe {
    fn name(&self) -> &'static str {
        "storage_wipe"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        if !ctx
            .capabilities
            .check(Capability::Storage, "storage_wipe", ctx.now_ms())
        {
            return Err(ActionError::CapabilityDenied {
                action: "storage_wipe",
                needed: Capability::Storage,
            });
        }
        match ctx.storage.clear().await {
            Ok(()) => {
                if ctx.state.is_responsive() {
                    ctx.state.storage_cache.clear_present();
                }
                Ok(())
            }
            Err(error) => {
                if let Some(handler) = &self.on_error {
                    handler.run_serial(ctx).await
                } else {
                    Err(ActionError::Custom(format!("storage_wipe: {error}")))
                }
            }
        }
    }
}

pub fn make_storage_wipe_body(
    registry: &crate::action::ActionRegistry,
    body: &Value,
) -> Result<BoxedAction, ActionError> {
    let on_error = body
        .as_object()
        .and_then(|object| object.get("on_error"))
        .map(|handler| registry.parse_list(handler))
        .transpose()?;
    Ok(Box::new(StorageWipe { on_error }))
}
