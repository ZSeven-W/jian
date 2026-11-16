//! In-process `HistoryRouter` — a simple stack of route paths.
//!
//! No HTTP history integration (that belongs to the web host, Plan 12).
//! `push` / `replace` / `pop` / `reset` all operate on an internal
//! navigation history so `&self` mutation stays
//! compatible with the `Router` trait signature.

use jian_core::action::services::router::NavigationHistory;
use jian_core::action::services::{RouteState, Router};
use jian_core::motion::screen_transition::ScreenTransitionSpec;
use std::collections::BTreeMap;

pub struct HistoryRouter {
    history: NavigationHistory,
}

impl HistoryRouter {
    pub fn new(initial: impl Into<String>) -> Self {
        Self {
            history: NavigationHistory::new(initial.into()),
        }
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.history.paths()
    }
}

impl Default for HistoryRouter {
    fn default() -> Self {
        Self::new("/")
    }
}

impl Router for HistoryRouter {
    fn current(&self) -> RouteState {
        let stack = self.history.paths();
        let path = stack.last().cloned().unwrap_or_else(|| "/".to_owned());
        RouteState {
            path,
            params: BTreeMap::new(),
            query: BTreeMap::new(),
            stack,
        }
    }

    fn push(&self, path: &str) {
        self.push_with(path, None);
    }

    fn replace(&self, path: &str) {
        self.replace_with(path, None);
    }

    fn pop(&self) {
        self.pop_with(None);
    }

    fn push_with(&self, path: &str, transition: Option<ScreenTransitionSpec>) {
        self.history.push(path, transition);
    }

    fn replace_with(&self, path: &str, transition: Option<ScreenTransitionSpec>) {
        self.history.replace(path, transition);
    }

    fn pop_with(&self, transition: Option<ScreenTransitionSpec>) {
        self.history.pop(transition);
    }

    fn last_transition(&self) -> Option<ScreenTransitionSpec> {
        self.history.last_transition()
    }

    fn reset(&self, path: &str) {
        self.history.restore(vec![path.to_owned()]);
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
