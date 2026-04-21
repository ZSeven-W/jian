//! Storage actions: `storage_set`, `storage_clear`, `storage_wipe`.

use crate::action::action_trait::{ActionImpl, BoxedAction};
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
        if !ctx.capabilities.check(Capability::Storage) {
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
            ctx.storage.set(key, v.0).await;
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
        if !ctx.capabilities.check(Capability::Storage) {
            return Err(ActionError::CapabilityDenied {
                action: "storage_clear",
                needed: Capability::Storage,
            });
        }
        ctx.storage.delete(&self.key).await;
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

struct StorageWipe;

#[async_trait(?Send)]
impl ActionImpl for StorageWipe {
    fn name(&self) -> &'static str {
        "storage_wipe"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        if !ctx.capabilities.check(Capability::Storage) {
            return Err(ActionError::CapabilityDenied {
                action: "storage_wipe",
                needed: Capability::Storage,
            });
        }
        ctx.storage.clear().await;
        Ok(())
    }
}

pub fn factory_storage_wipe(_body: &Value) -> Result<BoxedAction, ActionError> {
    Ok(Box::new(StorageWipe))
}
