//! ActionContext — the bundle of references an Action needs during execution.

use super::capability::CapabilityGate;
use super::cancel::CancellationToken;
use super::services::{
    AsyncFeedback, ClipboardService, FeedbackSink, NetworkClient, Router, StorageBackend,
};
use crate::expression::{Diagnostic, ExpressionCache};
use crate::signal::scheduler::Scheduler;
use crate::state::StateGraph;
use crate::value::RuntimeValue;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub struct ActionContext {
    pub state: Rc<StateGraph>,
    pub scheduler: Rc<Scheduler>,

    /// $event value for event-triggered action chains. None for lifecycle.
    pub event: Option<RuntimeValue>,

    /// Local vars injected by enclosing control constructs (for_each -> $item/$index).
    pub locals: RefCell<BTreeMap<String, RuntimeValue>>,

    pub page_id: Option<String>,
    pub node_id: Option<String>,

    pub network: Rc<dyn NetworkClient>,
    pub storage: Rc<dyn StorageBackend>,
    pub router: Rc<dyn Router>,
    pub feedback: Rc<dyn FeedbackSink>,
    pub async_fb: Rc<dyn AsyncFeedback>,
    pub clipboard: Rc<dyn ClipboardService>,

    pub capabilities: Rc<dyn CapabilityGate>,
    pub expr_cache: Rc<ExpressionCache>,

    pub cancel: CancellationToken,
    pub warnings: RefCell<Vec<Diagnostic>>,
}

impl ActionContext {
    /// Push a local override (e.g. `$item`) for the duration of a scope.
    /// Returns the previous value if one existed.
    pub fn push_local(
        &self,
        name: impl Into<String>,
        value: RuntimeValue,
    ) -> Option<RuntimeValue> {
        self.locals.borrow_mut().insert(name.into(), value)
    }

    pub fn pop_local(&self, name: &str) -> Option<RuntimeValue> {
        self.locals.borrow_mut().remove(name)
    }

    pub fn warn(&self, d: Diagnostic) {
        self.warnings.borrow_mut().push(d);
    }

    pub fn take_warnings(&self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.warnings.borrow_mut())
    }

    pub fn locals_snapshot(&self) -> BTreeMap<String, RuntimeValue> {
        self.locals.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::capability::DummyCapabilityGate;
    use crate::action::services::{
        NullClipboard, NullFeedback, NullNetworkClient, NullRouter, NullStorageBackend,
    };

    pub(crate) fn make_ctx() -> ActionContext {
        let sched = Rc::new(Scheduler::new());
        ActionContext {
            state: Rc::new(StateGraph::new(sched.clone())),
            scheduler: sched,
            event: None,
            locals: RefCell::new(BTreeMap::new()),
            page_id: None,
            node_id: None,
            network: Rc::new(NullNetworkClient),
            storage: Rc::new(NullStorageBackend),
            router: Rc::new(NullRouter),
            feedback: Rc::new(NullFeedback),
            async_fb: Rc::new(NullFeedback),
            clipboard: Rc::new(NullClipboard),
            capabilities: Rc::new(DummyCapabilityGate),
            expr_cache: Rc::new(ExpressionCache::new()),
            cancel: CancellationToken::new(),
            warnings: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn push_pop_locals() {
        let ctx = make_ctx();
        assert!(ctx.locals.borrow().is_empty());
        ctx.push_local("item", RuntimeValue::from_i64(42));
        assert_eq!(ctx.locals.borrow().get("item").unwrap().as_i64(), Some(42));
        ctx.pop_local("item");
        assert!(ctx.locals.borrow().is_empty());
    }

    #[test]
    fn warnings_accumulate_and_drain() {
        use crate::expression::{DiagKind, Span};
        let ctx = make_ctx();
        ctx.warn(Diagnostic {
            kind: DiagKind::RuntimeWarning,
            message: "x".into(),
            span: Span::zero(),
        });
        assert_eq!(ctx.take_warnings().len(), 1);
        assert_eq!(ctx.take_warnings().len(), 0);
    }
}
