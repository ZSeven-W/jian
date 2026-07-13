//! Cancellation token — clone-shared boolean flag consumed by long-running
//! actions (fetch, delay, ws loops) so that unmounting a node / aborting a
//! chain stops them promptly.

use crate::state::{Scope, Segment, StateGraph, StatePath};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Clone, Default)]
pub struct CancellationToken {
    flag: Rc<Cell<bool>>,
    next_compensation: Rc<Cell<u64>>,
    compensations: Rc<RefCell<BTreeMap<u64, StateCompensation>>>,
}

#[derive(Clone)]
struct StateCompensation {
    path: StatePath,
    page_id: Option<String>,
    node_id: Option<String>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.flag.set(true)
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.get()
    }

    pub(crate) fn register_false_write(
        &self,
        path: StatePath,
        page_id: Option<String>,
        node_id: Option<String>,
    ) -> u64 {
        let id = self.next_compensation.get().wrapping_add(1).max(1);
        self.next_compensation.set(id);
        self.compensations.borrow_mut().insert(
            id,
            StateCompensation {
                path,
                page_id,
                node_id,
            },
        );
        id
    }

    pub(crate) fn unregister_compensation(&self, id: u64) {
        self.compensations.borrow_mut().remove(&id);
    }

    pub(crate) fn preview_compensations(&self, state: &StateGraph) {
        for compensation in self.compensations.borrow().values() {
            compensation.apply(state);
        }
    }
}

impl StateCompensation {
    fn apply(&self, state: &StateGraph) {
        if self.path.segments.len() != 1 {
            return;
        }
        let Segment::Key(key) = &self.path.segments[0] else {
            return;
        };
        match self.path.scope {
            Scope::App => state.app_set(key, Value::Bool(false)),
            Scope::Vars => state.vars_set(key, Value::Bool(false)),
            Scope::Page => {
                if let Some(page) = &self.page_id {
                    state.page_set(page, key, Value::Bool(false));
                }
            }
            Scope::SelfNode => {
                if let Some(node) = &self.node_id {
                    state.self_set(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_shares_flag() {
        let a = CancellationToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }
}
