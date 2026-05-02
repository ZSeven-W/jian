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

    /// Plan 19 D1 cold-start: capture the current value of every
    /// signal in every scope into a `DefaultStateSnapshot` for
    /// serialisation into `aot/default_state.bin`. The pack writer
    /// calls this immediately after `Runtime::new_from_document`
    /// finishes seeding, so the snapshot contains exactly the
    /// schema-default values without any user mutation.
    ///
    /// `BTreeMap`-keyed scopes preserve their iteration order, which
    /// in turn keeps `write_bytes` output deterministic (a content-
    /// addressed pack hash relies on that determinism).
    pub fn dump_default_state(&self) -> jian_ops_schema::pack::DefaultStateSnapshot {
        use jian_ops_schema::pack::DefaultStateSnapshot;
        let mut snap = DefaultStateSnapshot::default();
        for (k, sig) in self.app.borrow().iter() {
            snap.app.insert(k.clone(), sig.get().0);
        }
        for (page, fields) in self.page.borrow().iter() {
            let mut m = std::collections::BTreeMap::new();
            for (k, sig) in fields {
                m.insert(k.clone(), sig.get().0);
            }
            snap.page.insert(page.clone(), m);
        }
        for (node, fields) in self.self_.borrow().iter() {
            let mut m = std::collections::BTreeMap::new();
            for (k, sig) in fields {
                m.insert(k.clone(), sig.get().0);
            }
            snap.self_node.insert(node.clone(), m);
        }
        for (k, sig) in self.route.borrow().iter() {
            snap.route.insert(k.clone(), sig.get().0);
        }
        for (k, sig) in self.storage.borrow().iter() {
            snap.storage.insert(k.clone(), sig.get().0);
        }
        for (k, sig) in self.vars.borrow().iter() {
            snap.vars.insert(k.clone(), sig.get().0);
        }
        snap
    }

    /// Plan 19 D1 cold-start fast path: re-seed every scope from a
    /// pre-baked `aot/default_state.bin` snapshot. Existing keys
    /// are overwritten via `Signal::set` (preserving the signal
    /// identity so subscribers don't lose their wiring); new keys
    /// allocate fresh signals. Hosts call this between the runtime
    /// constructor and the first `tick`, replacing the schema-default
    /// scan that `SeedStateGraph` would otherwise do.
    pub fn restore_default_state(&self, snap: &jian_ops_schema::pack::DefaultStateSnapshot) {
        for (name, value) in &snap.app {
            self.app_set(name, value.clone());
        }
        for (page_id, fields) in &snap.page {
            for (name, value) in fields {
                self.page_set(page_id, name, value.clone());
            }
        }
        for (node_id, fields) in &snap.self_node {
            for (name, value) in fields {
                self.self_set(node_id, name, value.clone());
            }
        }
        for (name, value) in &snap.route {
            self.route_set(name, value.clone());
        }
        for (name, value) in &snap.storage {
            self.storage_set(name, value.clone());
        }
        for (name, value) in &snap.vars {
            self.vars_set(name, value.clone());
        }
    }

    /// `$route.<name>` setter mirroring [`Self::app_set`]. Used by
    /// [`Self::restore_default_state`] (pack-side AOT seed) and by
    /// hosts that own the route table.
    pub fn route_set(&self, name: &str, value: Value) {
        let rv = RuntimeValue(value);
        let mut map = self.route.borrow_mut();
        if let Some(sig) = map.get(name) {
            sig.set(rv);
        } else {
            let sig = Signal::new(rv, self.scheduler.clone());
            map.insert(name.to_owned(), sig);
        }
    }

    /// `$storage.<name>` setter mirroring [`Self::app_set`]. Used by
    /// [`Self::restore_default_state`]. Hosts that wire a real
    /// storage backend layer their reads on top.
    pub fn storage_set(&self, name: &str, value: Value) {
        let rv = RuntimeValue(value);
        let mut map = self.storage.borrow_mut();
        if let Some(sig) = map.get(name) {
            sig.set(rv);
        } else {
            let sig = Signal::new(rv, self.scheduler.clone());
            map.insert(name.to_owned(), sig);
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

    // Plan 19 D1: AOT default-state dump / restore round-trip.

    #[test]
    fn dump_captures_every_scope() {
        let s = Rc::new(Scheduler::new());
        let g = StateGraph::new(s);
        g.app_set("count", json!(7));
        g.page_set("home", "scrollTop", json!(120));
        g.self_set("btn", "hover", json!(false));
        g.route_set("path", json!("/foo"));
        g.storage_set("theme", json!("dark"));
        g.vars_set("primary", json!("#3b82f6"));

        let snap = g.dump_default_state();
        assert_eq!(snap.app.get("count"), Some(&json!(7)));
        assert_eq!(
            snap.page.get("home").and_then(|m| m.get("scrollTop")),
            Some(&json!(120))
        );
        assert_eq!(
            snap.self_node.get("btn").and_then(|m| m.get("hover")),
            Some(&json!(false))
        );
        assert_eq!(snap.route.get("path"), Some(&json!("/foo")));
        assert_eq!(snap.storage.get("theme"), Some(&json!("dark")));
        assert_eq!(snap.vars.get("primary"), Some(&json!("#3b82f6")));
    }

    #[test]
    fn restore_round_trip_through_bytes() {
        let s = Rc::new(Scheduler::new());
        let g = StateGraph::new(s);
        g.app_set("count", json!(7));
        g.app_set("user", json!({"name":"Alice","age":30}));
        g.page_set("home", "scrollTop", json!(120));

        let bytes = g.dump_default_state().write_bytes().expect("encode");
        let restored =
            jian_ops_schema::pack::DefaultStateSnapshot::read_bytes(&bytes).expect("decode");

        let s2 = Rc::new(Scheduler::new());
        let g2 = StateGraph::new(s2);
        g2.restore_default_state(&restored);
        assert_eq!(g2.app_get("count").unwrap().as_i64(), Some(7));
        let p = StatePath::parse("$app.user.name").unwrap();
        assert_eq!(g2.resolve(&p, None, None).unwrap().as_str(), Some("Alice"));
    }

    #[test]
    fn restore_overwrites_existing_signal_in_place() {
        // The restored signal must reuse the same `Signal` instance so
        // subscribers keep their wiring; a fresh `Signal::new` would
        // detach binding effects. We verify by capturing the existing
        // signal handle BEFORE restore, then reading through it AFTER
        // — if the handle returns the new value, the underlying inner
        // is shared (Signal is `Clone` over an `Rc<SignalInner>`).
        let s = Rc::new(Scheduler::new());
        let g = StateGraph::new(s);
        g.app_set("count", json!(0));
        let pre_signal = g.app_signal("count").expect("pre-restore signal");
        assert_eq!(pre_signal.get().0, json!(0));

        let mut snap = jian_ops_schema::pack::DefaultStateSnapshot::default();
        snap.app.insert("count".into(), json!(99));
        g.restore_default_state(&snap);

        // The pre-restore handle now returns the post-restore value
        // — only possible if the underlying `Rc<SignalInner>` was
        // reused. (Codex follow-up reminder: a `Signal::new` here
        // would orphan binding subscribers across an AOT seed.)
        assert_eq!(pre_signal.get().0, json!(99));
    }
}
