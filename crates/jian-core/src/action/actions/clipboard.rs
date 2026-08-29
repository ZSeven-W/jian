//! Clipboard actions: authored `copy` and `paste` service calls.

use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::context::{ActionContext, EffectRequestContext};
use crate::action::error::{ActionError, ActionResult};
use crate::action::services::effect_sink::{EffectOutcome, EffectRequest};
use crate::capability::Capability;
use crate::expression::Expression;
use crate::state::path::StatePath;
use async_trait::async_trait;
use serde_json::Value;

struct CopyText {
    text: Expression,
}

#[async_trait(?Send)]
impl ActionImpl for CopyText {
    fn name(&self) -> &'static str {
        "copy"
    }

    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        require_clipboard(ctx, "copy")?;
        let locals = ctx.locals_snapshot();
        let (value, warnings) = self.text.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for warning in warnings {
            ctx.warn(warning);
        }
        let text = value.as_str().unwrap_or_default();
        // R3: the host sink owns the effect; the legacy clipboard service
        // stays the fallback for runtimes without a sink.
        let ectx = EffectRequestContext {
            handler: None,
            node_id: ctx.node_id.clone(),
            activation: ctx.activation,
        };
        let request = EffectRequest::Copy {
            text: text.to_owned(),
        };
        match ctx.effect_sink.request(&ectx, &request) {
            EffectOutcome::Accepted => return Ok(()),
            EffectOutcome::Rejected(detail) => {
                ctx.warn(crate::expression::Diagnostic {
                    kind: crate::expression::DiagKind::RuntimeWarning,
                    message: format!("copy: {detail}"),
                    span: crate::expression::Span::zero(),
                });
                return Ok(());
            }
            EffectOutcome::Unsupported => {}
        }
        ctx.clipboard
            .write_text(text)
            .await
            .map_err(|error| ActionError::Custom(format!("copy: {error}")))
    }
}

struct PasteText {
    into: StatePath,
}

#[async_trait(?Send)]
impl ActionImpl for PasteText {
    fn name(&self) -> &'static str {
        "paste"
    }

    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        require_clipboard(ctx, "paste")?;
        let text = ctx
            .clipboard
            .read_text()
            .await
            .map_err(|error| ActionError::Custom(format!("paste: {error}")))?;
        super::state::write_path(ctx, &self.into, Value::String(text))
    }
}

fn require_clipboard(ctx: &ActionContext, action: &'static str) -> ActionResult {
    if ctx
        .capabilities
        .check(Capability::Clipboard, action, ctx.now_ms())
    {
        Ok(())
    } else {
        Err(ActionError::CapabilityDenied {
            action,
            needed: Capability::Clipboard,
        })
    }
}

pub fn factory_copy(body: &Value) -> Result<BoxedAction, ActionError> {
    let source = match body {
        Value::String(source) => source.as_str(),
        Value::Object(object) => {
            object
                .get("text")
                .and_then(Value::as_str)
                .ok_or(ActionError::MissingField {
                    name: "copy",
                    field: "text",
                })?
        }
        _ => {
            return Err(ActionError::FieldType {
                name: "copy",
                field: "body",
                message: "must be a text expression or { text: expression }".into(),
            })
        }
    };
    Ok(Box::new(CopyText {
        text: Expression::compile(source)?,
    }))
}

pub fn factory_paste(body: &Value) -> Result<BoxedAction, ActionError> {
    let target = match body {
        Value::String(target) => target.as_str(),
        Value::Object(object) => {
            object
                .get("into")
                .and_then(Value::as_str)
                .ok_or(ActionError::MissingField {
                    name: "paste",
                    field: "into",
                })?
        }
        _ => {
            return Err(ActionError::FieldType {
                name: "paste",
                field: "body",
                message: "must be a state path or { into: state path }".into(),
            })
        }
    };
    let into = StatePath::parse(target)
        .map_err(|error| ActionError::Custom(format!("paste.into: {error}")))?;
    Ok(Box::new(PasteText { into }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::capability::DummyCapabilityGate;
    use crate::action::context::tests::make_ctx;
    use crate::action::services::{ClipboardService, ServiceError};
    use crate::action::{default_registry, ExecOutcome};
    use async_trait::async_trait;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct MockClipboard {
        value: RefCell<String>,
        fail: bool,
    }

    #[async_trait(?Send)]
    impl ClipboardService for MockClipboard {
        async fn read_text(&self) -> Result<String, ServiceError> {
            if self.fail {
                Err(ServiceError("permission denied".into()))
            } else {
                Ok(self.value.borrow().clone())
            }
        }

        async fn write_text(&self, text: &str) -> Result<(), ServiceError> {
            if self.fail {
                Err(ServiceError("permission denied".into()))
            } else {
                *self.value.borrow_mut() = text.to_owned();
                Ok(())
            }
        }
    }

    #[test]
    fn registered_copy_and_paste_reach_service_and_state() {
        futures::executor::block_on(async {
            let clipboard = Rc::new(MockClipboard {
                value: RefCell::new(String::new()),
                fail: false,
            });
            let mut ctx = make_ctx();
            ctx.clipboard = clipboard.clone();
            ctx.capabilities = Rc::new(DummyCapabilityGate);
            let list = serde_json::json!([
                {"copy": {"text": "'hello'"}},
                {"paste": {"into": "$app.pasted"}}
            ]);

            let registry = default_registry();
            let chain = registry.borrow().parse_list(&list).unwrap();
            let outcome = ExecOutcome {
                result: chain.run_serial(&ctx).await,
                warnings: ctx.take_warnings(),
            };
            assert!(outcome.result.is_ok());
            assert_eq!(&*clipboard.value.borrow(), "hello");
            assert_eq!(
                ctx.state.app_get("pasted").unwrap().0,
                serde_json::json!("hello")
            );
        });
    }

    #[test]
    fn clipboard_permission_error_aborts_serial_chain() {
        futures::executor::block_on(async {
            let mut ctx = make_ctx();
            ctx.clipboard = Rc::new(MockClipboard {
                value: RefCell::new(String::new()),
                fail: true,
            });
            ctx.capabilities = Rc::new(DummyCapabilityGate);
            let list = serde_json::json!([
                {"copy": "'secret'"},
                {"set": {"$app.continued": "true"}}
            ]);

            let registry = default_registry();
            let chain = registry.borrow().parse_list(&list).unwrap();
            let outcome = ExecOutcome {
                result: chain.run_serial(&ctx).await,
                warnings: ctx.take_warnings(),
            };
            assert!(matches!(outcome.result, Err(ActionError::Custom(_))));
            assert!(ctx.state.app_get("continued").is_none());
        });
    }
}
