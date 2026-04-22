//! L4 platform stubs — emit warnings only until real adapters land.
//!
//! Actions: `vibrate`, `share`, `haptic`, `notify`. All succeed and push a
//! RuntimeWarning describing the parameters; real dispatch arrives with the
//! host-adapter plans.

use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::capability::Capability;
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
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
            if !ctx.capabilities.check(cap, self.name_) {
                return Err(ActionError::CapabilityDenied {
                    action: self.name_,
                    needed: cap,
                });
            }
        }
        warn_stub(ctx, self.name_, &self.body);
        Ok(())
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
