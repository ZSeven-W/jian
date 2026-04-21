//! Runtime state graph — holds all Signals organized by scope, page, and node.

pub mod path;
pub mod scope;

pub use path::{PathError, Segment, StatePath};
pub use scope::Scope;

use crate::signal::{scheduler::Scheduler, Signal};
use crate::value::RuntimeValue;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub type NodeId = String;
pub type PageId = String;

pub struct StateGraph {
    scheduler: Rc<Scheduler>,
    pub(crate) app: RefCell<BTreeMap<String, Signal<RuntimeValue>>>,
    pub(crate) page: RefCell<BTreeMap<PageId, BTreeMap<String, Signal<RuntimeValue>>>>,
    pub(crate) self_: RefCell<BTreeMap<NodeId, BTreeMap<String, Signal<RuntimeValue>>>>,
    pub(crate) route: RefCell<BTreeMap<String, Signal<RuntimeValue>>>,
    pub(crate) storage: RefCell<BTreeMap<String, Signal<RuntimeValue>>>,
    pub(crate) vars: RefCell<BTreeMap<String, Signal<RuntimeValue>>>,
}

impl StateGraph {
    pub fn new(scheduler: Rc<Scheduler>) -> Self {
        Self {
            scheduler,
            app: RefCell::new(BTreeMap::new()),
            page: RefCell::new(BTreeMap::new()),
            self_: RefCell::new(BTreeMap::new()),
            route: RefCell::new(BTreeMap::new()),
            storage: RefCell::new(BTreeMap::new()),
            vars: RefCell::new(BTreeMap::new()),
        }
    }

    /// Create or update a state variable in the app scope.
    pub fn app_set(&self, name: &str, value: Value) {
        let rv = RuntimeValue(value);
        let mut map = self.app.borrow_mut();
        if let Some(sig) = map.get(name) {
            sig.set(rv);
        } else {
            let sig = Signal::new(rv, self.scheduler.clone());
            map.insert(name.to_owned(), sig);
        }
    }

    pub fn app_get(&self, name: &str) -> Option<RuntimeValue> {
        self.app.borrow().get(name).map(|s| s.get())
    }

    pub fn app_signal(&self, name: &str) -> Option<Signal<RuntimeValue>> {
        self.app.borrow().get(name).cloned()
    }

    /// Create or update a design variable in the `$vars` scope.
    pub fn vars_set(&self, name: &str, value: Value) {
        let rv = RuntimeValue(value);
        let mut map = self.vars.borrow_mut();
        if let Some(sig) = map.get(name) {
            sig.set(rv);
        } else {
            let sig = Signal::new(rv, self.scheduler.clone());
            map.insert(name.to_owned(), sig);
        }
    }

    pub fn vars_get(&self, name: &str) -> Option<RuntimeValue> {
        self.vars.borrow().get(name).map(|s| s.get())
    }

    pub fn page_set(&self, page_id: &str, name: &str, value: Value) {
        let rv = RuntimeValue(value);
        let mut map = self.page.borrow_mut();
        let entry = map.entry(page_id.to_owned()).or_default();
        if let Some(sig) = entry.get(name) {
            sig.set(rv);
        } else {
            let sig = Signal::new(rv, self.scheduler.clone());
            entry.insert(name.to_owned(), sig);
        }
    }

    pub fn self_set(&self, node_id: &str, name: &str, value: Value) {
        let rv = RuntimeValue(value);
        let mut map = self.self_.borrow_mut();
        let entry = map.entry(node_id.to_owned()).or_default();
        if let Some(sig) = entry.get(name) {
            sig.set(rv);
        } else {
            let sig = Signal::new(rv, self.scheduler.clone());
            entry.insert(name.to_owned(), sig);
        }
    }

    /// Resolve a StatePath to the underlying RuntimeValue, walking segments.
    pub fn resolve(
        &self,
        path: &StatePath,
        context_page: Option<&str>,
        context_node: Option<&str>,
    ) -> Option<RuntimeValue> {
        let base: Signal<RuntimeValue> = match path.scope {
            Scope::App => self
                .app
                .borrow()
                .get(path.segments.first().and_then(seg_as_key)?)
                .cloned()?,
            Scope::Page => {
                let pid = context_page?;
                self.page
                    .borrow()
                    .get(pid)?
                    .get(path.segments.first().and_then(seg_as_key)?)
                    .cloned()?
            }
            Scope::SelfNode => {
                let nid = context_node?;
                self.self_
                    .borrow()
                    .get(nid)?
                    .get(path.segments.first().and_then(seg_as_key)?)
                    .cloned()?
            }
            Scope::Route => self
                .route
                .borrow()
                .get(path.segments.first().and_then(seg_as_key)?)
                .cloned()?,
            Scope::Storage => self
                .storage
                .borrow()
                .get(path.segments.first().and_then(seg_as_key)?)
                .cloned()?,
            Scope::Vars => self
                .vars
                .borrow()
                .get(path.segments.first().and_then(seg_as_key)?)
                .cloned()?,
        };
        let mut cur = base.get().0;
        for seg in &path.segments[1..] {
            cur = walk(&cur, seg)?;
        }
        Some(RuntimeValue(cur))
    }
}

fn seg_as_key(s: &Segment) -> Option<&str> {
    match s {
        Segment::Key(k) => Some(k.as_str()),
        _ => None,
    }
}

fn walk(v: &Value, seg: &Segment) -> Option<Value> {
    match (v, seg) {
        (Value::Object(m), Segment::Key(k)) => m.get(k).cloned(),
        (Value::Array(a), Segment::Index(i)) => a.get(*i).cloned(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn app_scope_crud() {
        let s = Rc::new(Scheduler::new());
        let g = StateGraph::new(s);
        g.app_set("count", json!(0));
        assert_eq!(g.app_get("count").unwrap().as_i64(), Some(0));
        g.app_set("count", json!(42));
        assert_eq!(g.app_get("count").unwrap().as_i64(), Some(42));
    }

    #[test]
    fn resolve_with_segments() {
        let s = Rc::new(Scheduler::new());
        let g = StateGraph::new(s);
        g.app_set("user", json!({"name":"Alice","age":30}));
        let p = StatePath::parse("$app.user.name").unwrap();
        let v = g.resolve(&p, None, None).unwrap();
        assert_eq!(v.as_str(), Some("Alice"));
    }

    #[test]
    fn resolve_array_index() {
        let s = Rc::new(Scheduler::new());
        let g = StateGraph::new(s);
        g.app_set("items", json!([{"id":1},{"id":2},{"id":3}]));
        let p = StatePath::parse("$app.items[1].id").unwrap();
        assert_eq!(g.resolve(&p, None, None).unwrap().as_i64(), Some(2));
    }

    #[test]
    fn resolve_missing_returns_none() {
        let s = Rc::new(Scheduler::new());
        let g = StateGraph::new(s);
        let p = StatePath::parse("$app.nope").unwrap();
        assert!(g.resolve(&p, None, None).is_none());
    }
}
