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
                act.execute(ctx).await?;
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
