//! In-process `HistoryRouter` — a simple stack of route paths.
//!
//! No HTTP history integration (that belongs to the web host, Plan 12).
//! `push` / `replace` / `pop` / `reset` all operate on an internal
//! `Vec<String>` guarded by a `RefCell` so `&self` mutation stays
//! compatible with the `Router` trait signature.

use jian_core::action::services::{RouteState, Router};
use std::cell::RefCell;
use std::collections::BTreeMap;

pub struct HistoryRouter {
    stack: RefCell<Vec<String>>,
}

impl HistoryRouter {
    pub fn new(initial: impl Into<String>) -> Self {
        Self {
            stack: RefCell::new(vec![initial.into()]),
        }
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.stack.borrow().clone()
    }
}

impl Default for HistoryRouter {
    fn default() -> Self {
        Self::new("/")
    }
}

impl Router for HistoryRouter {
    fn current(&self) -> RouteState {
        let stack = self.stack.borrow();
        let path = stack.last().cloned().unwrap_or_else(|| "/".to_owned());
        RouteState {
            path,
            params: BTreeMap::new(),
            query: BTreeMap::new(),
            stack: stack.clone(),
        }
    }

    fn push(&self, path: &str) {
        self.stack.borrow_mut().push(path.to_owned());
    }

    fn replace(&self, path: &str) {
        let mut s = self.stack.borrow_mut();
        if let Some(last) = s.last_mut() {
            *last = path.to_owned();
        } else {
            s.push(path.to_owned());
        }
    }

    fn pop(&self) {
        let mut s = self.stack.borrow_mut();
        if s.len() > 1 {
            s.pop();
        }
    }

    fn reset(&self, path: &str) {
        let mut s = self.stack.borrow_mut();
        s.clear();
        s.push(path.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_extends_stack() {
        let r = HistoryRouter::new("/");
        r.push("/detail/1");
        assert_eq!(r.current().path, "/detail/1");
        assert_eq!(r.snapshot(), vec!["/", "/detail/1"]);
    }

    #[test]
    fn pop_does_not_empty_stack() {
        let r = HistoryRouter::new("/");
        r.pop();
        assert_eq!(r.snapshot(), vec!["/"]);
    }

    #[test]
    fn replace_swaps_tip() {
        let r = HistoryRouter::new("/");
        r.push("/a");
        r.replace("/b");
        assert_eq!(r.snapshot(), vec!["/", "/b"]);
    }

    #[test]
    fn reset_clears_then_pushes() {
        let r = HistoryRouter::new("/");
        r.push("/a");
        r.push("/b");
        r.reset("/home");
        assert_eq!(r.snapshot(), vec!["/home"]);
    }
}
