//! Screen routing glue: a validating `Router` + the per-frame
//! reconcile helper that swaps the mounted screen when the route
//! changes. Shared by the OP editor preview and `jian player` so both
//! run the same code path.

use crate::action::services::{RouteState, Router};
use std::cell::RefCell;
use std::collections::BTreeSet;

/// A navigation the router refused because the target path is not a
/// known screen. Drained by `reconcile_screens` and surfaced as a
/// host-side warning ('Router' has no warn hook of its own).
#[derive(Debug, Clone, PartialEq)]
pub struct RejectedNav {
    pub verb: &'static str,
    pub path: String,
}

/// Route-table-validating router: mutates only toward known screen
/// paths, so the mounted-screen reconcile never has to roll back.
pub struct ScreenRouter {
    known: BTreeSet<String>,
    stack: RefCell<Vec<String>>,
    rejections: RefCell<Vec<RejectedNav>>,
}

impl ScreenRouter {
    pub fn new(entry: &str, known: impl IntoIterator<Item = String>) -> Self {
        Self {
            known: known.into_iter().collect(),
            stack: RefCell::new(vec![entry.to_owned()]),
            rejections: RefCell::new(Vec::new()),
        }
    }

    pub fn take_rejections(&self) -> Vec<RejectedNav> {
        std::mem::take(&mut self.rejections.borrow_mut())
    }

    fn check(&self, verb: &'static str, path: &str) -> bool {
        if self.known.contains(path) {
            true
        } else {
            self.rejections
                .borrow_mut()
                .push(RejectedNav {
                    verb,
                    path: path.to_owned(),
                });
            false
        }
    }
}

impl Router for ScreenRouter {
    fn current(&self) -> RouteState {
        let stack = self.stack.borrow();
        RouteState {
            path: stack.last().cloned().unwrap_or_else(|| "/".to_owned()),
            params: Default::default(),
            query: Default::default(),
            stack: stack.clone(),
        }
    }

    fn push(&self, path: &str) {
        if self.check("push", path) {
            self.stack.borrow_mut().push(path.to_owned());
        }
    }

    fn replace(&self, path: &str) {
        if self.check("replace", path) {
            let mut s = self.stack.borrow_mut();
            if let Some(last) = s.last_mut() {
                *last = path.to_owned();
            } else {
                s.push(path.to_owned());
            }
        }
    }

    fn pop(&self) {
        let mut s = self.stack.borrow_mut();
        if s.len() > 1 {
            s.pop();
        }
    }

    fn reset(&self, path: &str) {
        if self.check("reset", path) {
            let mut s = self.stack.borrow_mut();
            s.clear();
            s.push(path.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_path_push_mutates_stack() {
        let r = ScreenRouter::new("/", ["/".to_owned(), "/detail".to_owned()]);
        r.push("/detail");
        assert_eq!(r.current().path, "/detail");
        assert_eq!(r.current().stack, vec!["/", "/detail"]);
        assert!(r.take_rejections().is_empty());
    }

    #[test]
    fn unknown_path_does_not_mutate_and_records_rejection() {
        let r = ScreenRouter::new("/", ["/".to_owned()]);
        r.push("/nope");
        r.replace("/also-nope");
        assert_eq!(r.current().path, "/");
        let rej = r.take_rejections();
        assert_eq!(rej.len(), 2);
        assert_eq!(rej[0].verb, "push");
        assert_eq!(rej[0].path, "/nope");
        assert!(r.take_rejections().is_empty(), "take drains");
    }

    #[test]
    fn pop_never_empties_stack() {
        let r = ScreenRouter::new("/", ["/".to_owned()]);
        r.pop();
        assert_eq!(r.current().path, "/");
    }
}
