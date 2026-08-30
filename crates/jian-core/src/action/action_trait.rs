//! The Action trait + type-erased boxing for the registry.

use super::context::ActionContext;
use super::error::ActionResult;
use async_trait::async_trait;
use futures::future::LocalBoxFuture;
use serde_json::Value;

/// Parsed action — holds whatever pre-processed state an action needs
/// (typically a compiled Expression pool; see actions/state.rs).
#[async_trait(?Send)]
pub trait ActionImpl: 'static {
    fn name(&self) -> &'static str;
    async fn execute(&self, ctx: &ActionContext) -> ActionResult;

    /// Optional rejection branch (R3 policy): runs when the context's
    /// policy rejects this action's NAME, before its capability gate or
    /// side effect ever run. Default no-op; actions that parse an
    /// authored error continuation override this to run that
    /// already-parsed branch.
    async fn on_policy_rejected(&self, _ctx: &ActionContext) {}
}

pub type BoxedAction = Box<dyn ActionImpl>;

/// Factory signature: given a JSON body, construct the BoxedAction.
pub type ActionFactory = Box<dyn Fn(&Value) -> Result<BoxedAction, super::error::ActionError>>;

/// Executable list of actions, produced by `parse_list`.
pub struct ActionChain(pub Vec<BoxedAction>);

impl ActionChain {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn run_serial<'a>(&'a self, ctx: &'a ActionContext) -> LocalBoxFuture<'a, ActionResult> {
        use futures::future::FutureExt;
        async move {
            for act in &self.0 {
                if ctx.cancel.is_cancelled() {
                    return Err(super::error::ActionError::Aborted);
                }
                // R3 policy guard: runs BEFORE the action's capability
                // gate or side effect. A rejection is a structured,
                // non-fatal diagnostic — the optional rejection branch
                // runs and later SAFE SIBLINGS still execute, so one
                // forbidden action can never swallow the rest of the
                // list.
                if let Some(policy) = &ctx.policy {
                    if let Err(error) = policy.check(act.name()) {
                        ctx.warn(crate::expression::Diagnostic {
                            kind: crate::expression::DiagKind::RuntimeWarning,
                            message: error.to_string(),
                            span: crate::expression::Span::zero(),
                        });
                        act.on_policy_rejected(ctx).await;
                        continue;
                    }
                }
                let observation = ctx.observer.action_started(act.name(), ctx);
                let result = act.execute(ctx).await;
                ctx.observer
                    .action_finished(observation, act.name(), ctx, &result);
                result?;
            }
            Ok(())
        }
        .boxed_local()
    }
}

impl Default for ActionChain {
    fn default() -> Self {
        Self::new()
    }
}
