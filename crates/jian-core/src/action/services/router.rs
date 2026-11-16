use crate::motion::screen_transition::ScreenTransitionSpec;
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteState {
    pub path: String,
    pub params: BTreeMap<String, String>,
    pub query: BTreeMap<String, String>,
    pub stack: Vec<String>,
}

pub trait Router {
    fn current(&self) -> RouteState;
    fn push(&self, path: &str);
    fn replace(&self, path: &str);
    fn pop(&self);
    fn reset(&self, path: &str);
    fn push_with(&self, path: &str, _transition: Option<ScreenTransitionSpec>) {
        self.push(path);
    }
    fn replace_with(&self, path: &str, _transition: Option<ScreenTransitionSpec>) {
        self.replace(path);
    }
    fn pop_with(&self, _transition: Option<ScreenTransitionSpec>) {
        self.pop();
    }
    fn last_transition(&self) -> Option<ScreenTransitionSpec> {
        None
    }
    fn restore(&self, state: RouteState, valid_paths: &[String]) {
        let path = if valid_paths.iter().any(|path| path == &state.path) {
            state.path
        } else {
            valid_paths.first().cloned().unwrap_or_else(|| "/".into())
        };
        self.reset(&path);
    }
}

/// In-process history for routers that support transitions. Each entry keeps
/// the specification used to arrive there so Back can reuse it.
#[derive(Debug)]
pub struct NavigationHistory {
    inner: RefCell<HistoryState>,
}

#[derive(Debug)]
struct HistoryState {
    entries: Vec<(String, Option<ScreenTransitionSpec>)>,
    last_transition: Option<ScreenTransitionSpec>,
}

impl NavigationHistory {
    pub fn new(initial: String) -> Self {
        Self {
            inner: RefCell::new(HistoryState {
                entries: vec![(initial, None)],
                last_transition: None,
            }),
        }
    }

    pub fn paths(&self) -> Vec<String> {
        self.inner
            .borrow()
            .entries
            .iter()
            .map(|entry| entry.0.clone())
            .collect()
    }

    pub fn push(&self, path: &str, transition: Option<ScreenTransitionSpec>) {
        let mut inner = self.inner.borrow_mut();
        inner.entries.push((path.into(), transition.clone()));
        inner.last_transition = transition;
    }

    pub fn replace(&self, path: &str, transition: Option<ScreenTransitionSpec>) {
        let mut inner = self.inner.borrow_mut();
        inner.entries.pop();
        inner.entries.push((path.into(), transition.clone()));
        inner.last_transition = transition;
    }

    pub fn pop(&self, transition: Option<ScreenTransitionSpec>) {
        let mut inner = self.inner.borrow_mut();
        if inner.entries.len() > 1 {
            let entry = inner.entries.pop().expect("nonempty route history");
            inner.last_transition = transition.or(entry.1);
        }
    }

    pub fn restore(&self, paths: Vec<String>) {
        *self.inner.borrow_mut() = HistoryState {
            entries: paths.into_iter().map(|path| (path, None)).collect(),
            last_transition: None,
        };
    }

    pub fn last_transition(&self) -> Option<ScreenTransitionSpec> {
        self.inner.borrow().last_transition.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::screen_transition::ScreenTransitionKind;

    #[test]
    fn pop_reuses_the_outgoing_entry_spec_or_an_explicit_override() {
        let fade = ScreenTransitionSpec {
            kind: ScreenTransitionKind::Fade,
            ..Default::default()
        };
        let cut = ScreenTransitionSpec {
            kind: ScreenTransitionKind::None,
            ..Default::default()
        };
        let history = NavigationHistory::new("/".into());
        history.push("/a", Some(fade.clone()));
        history.push("/b", None);
        assert_eq!(history.last_transition(), None);
        history.pop(None);
        assert_eq!(history.last_transition(), None);
        history.pop(None);
        assert_eq!(history.last_transition(), Some(fade.clone()));
        history.push("/a", Some(fade));
        history.pop(Some(cut.clone()));
        assert_eq!(history.last_transition(), Some(cut));
        history.restore(vec!["/".into()]);
        assert_eq!(history.last_transition(), None);
    }
}
