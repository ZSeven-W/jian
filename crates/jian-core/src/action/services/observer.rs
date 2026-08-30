//! Runtime observer invoked around action execution, including nested lists.

use crate::action::context::ActionContext;
use crate::action::error::ActionResult;

pub trait ActionObserver {
    fn action_started(&self, action: &'static str, context: &ActionContext) -> u64;
    fn action_finished(
        &self,
        token: u64,
        action: &'static str,
        context: &ActionContext,
        result: &ActionResult,
    );
}

pub struct NullActionObserver;

impl ActionObserver for NullActionObserver {
    fn action_started(&self, _action: &'static str, _context: &ActionContext) -> u64 {
        0
    }

    fn action_finished(
        &self,
        _token: u64,
        _action: &'static str,
        _context: &ActionContext,
        _result: &ActionResult,
    ) {
    }
}
