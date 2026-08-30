//! L4 platform stubs — emit warnings only until real adapters land.
//!
//! Actions: vibrate, share, haptic, notify, focus, blur, dismiss_keyboard.
//! RuntimeWarning describing the parameters; real dispatch arrives with the
//! host-adapter plans.

use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::capability::Capability;
use crate::action::context::{ActionContext, EffectRequestContext};
use crate::action::error::{ActionError, ActionResult};
use crate::action::services::effect_sink::{EffectOutcome, EffectRequest};
use async_trait::async_trait;
use serde_json::Value;

fn warn_stub(ctx: &ActionContext, name: &str, body: &Value) {
    ctx.warn(crate::expression::Diagnostic {
        kind: crate::expression::DiagKind::RuntimeWarning,
        message: format!(
            "{}: no adapter installed; stub invoked with body={}",
            name, body
        ),
        span: crate::expression::Span::zero(),
    });
}

struct Stub {
    name_: &'static str,
    capability: Option<Capability>,
    body: Value,
}

#[async_trait(?Send)]
impl ActionImpl for Stub {
    fn name(&self) -> &'static str {
        self.name_
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        if let Some(cap) = self.capability {
            if !ctx.capabilities.check(cap, self.name_, ctx.now_ms()) {
                return Err(ActionError::CapabilityDenied {
                    action: self.name_,
                    needed: cap,
                });
            }
        }
        // R3: these are HOST effects. Route through the sink first; a
        // runtime without a sink falls back to the legacy warn-stub.
        if let Some(request) = self.effect_request() {
            let ectx = EffectRequestContext {
                handler: ctx.handler.clone(),
                node_id: ctx.node_id.clone(),
                activation: ctx.activation,
                at_ms: ctx.now_ms(),
            };
            match ctx.effect_sink.request(&ectx, &request) {
                EffectOutcome::Accepted | EffectOutcome::AcceptedWithCompletion(_) => return Ok(()),
                EffectOutcome::Rejected(detail) => {
                    ctx.warn(crate::expression::Diagnostic {
                        kind: crate::expression::DiagKind::RuntimeWarning,
                        message: format!("{}: {detail}", self.name_),
                        span: crate::expression::Span::zero(),
                    });
                    return Ok(());
                }
                EffectOutcome::Unsupported => {}
            }
        }
        warn_stub(ctx, self.name_, &self.body);
        Ok(())
    }
}

impl Stub {
    /// The host-effect request this stub maps to, or `None` for actions
    /// that are not host effects (`vibrate`/`notify` keep warning until
    /// their adapters land).
    fn effect_request(&self) -> Option<EffectRequest> {
        match self.name_ {
            "haptic" => Some(EffectRequest::Haptic {
                style: self
                    .body
                    .get("style")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("medium")
                    .to_owned(),
            }),
            "share" => Some(EffectRequest::Share {
                payload: self.body.clone(),
            }),
            "focus" => Some(EffectRequest::FocusNode {
                node_id: self
                    .body
                    .get("nodeId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }),
            "blur" => Some(EffectRequest::BlurFocus),
            "dismiss_keyboard" => Some(EffectRequest::DismissKeyboard),
            _ => None,
        }
    }
}

macro_rules! stub_factory {
    ($fn_name:ident, $action:literal, $cap:expr) => {
        pub fn $fn_name(body: &Value) -> Result<BoxedAction, ActionError> {
            Ok(Box::new(Stub {
                name_: $action,
                capability: $cap,
                body: body.clone(),
            }))
        }
    };
}

stub_factory!(factory_vibrate, "vibrate", Some(Capability::Haptic));
stub_factory!(factory_haptic, "haptic", Some(Capability::Haptic));
stub_factory!(factory_share, "share", Some(Capability::Network));
stub_factory!(factory_notify, "notify", Some(Capability::Notifications));
// `focus` / `blur` programmatically move keyboard focus. Pure-runtime
// (no capability) — real FocusManager wiring lands with Plan 9
// host-desktop. The stubs warn + return Ok so the map's declaration of
// these as registered actions is honoured.
stub_factory!(factory_focus, "focus", None);
stub_factory!(factory_blur, "blur", None);
stub_factory!(factory_dismiss_keyboard, "dismiss_keyboard", None);
