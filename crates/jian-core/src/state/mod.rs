//! Runtime state graph — holds all Signals organized by scope, page, and node.

pub mod conformance;
pub mod path;
pub mod scope;
pub mod storage_cache;

pub use path::{PathError, Segment, StatePath};
pub use scope::Scope;

use crate::signal::{scheduler::Scheduler, Signal};
use crate::value::RuntimeValue;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

pub type NodeId = String;
pub type PageId = String;
pub type PageKey = String;
type FieldSignals = BTreeMap<String, Signal<RuntimeValue>>;
type SelfStateMap = BTreeMap<(PageKey, NodeId), FieldSignals>;

pub struct StateGraph {
    scheduler: Rc<Scheduler>,
    mutation_counter: Rc<Cell<u64>>,
    pub(crate) app: RefCell<BTreeMap<String, Signal<RuntimeValue>>>,
    pub(crate) page: RefCell<BTreeMap<PageId, BTreeMap<String, Signal<RuntimeValue>>>>,
    pub(crate) self_: RefCell<SelfStateMap>,
    pub(crate) route: RefCell<BTreeMap<String, Signal<RuntimeValue>>>,
    pub(crate) storage: RefCell<BTreeMap<String, Signal<RuntimeValue>>>,
    pub storage_cache: Rc<storage_cache::StorageCache>,
    responsive: Cell<bool>,
    pub(crate) vars: RefCell<BTreeMap<String, Signal<RuntimeValue>>>,
    pub(crate) viewport: RefCell<BTreeMap<String, Signal<RuntimeValue>>>,
    image_keys: RefCell<BTreeMap<String, String>>,
    now_ms: Cell<u64>,
}

impl StateGraph {
    pub fn app_snapshot(&self) -> BTreeMap<String, Value> {
        self.app
            .borrow()
            .iter()
            .map(|(key, signal)| (key.clone(), signal.get().0))
            .collect()
    }

    pub fn page_snapshot(&self, page_key: &str) -> BTreeMap<String, Value> {
        self.page
            .borrow()
            .get(page_key)
            .into_iter()
            .flat_map(|fields| fields.iter())
            .map(|(key, signal)| (key.clone(), signal.get().0))
            .collect()
    }
    pub fn page_keys(&self) -> Vec<String> {
        self.page.borrow().keys().cloned().collect()
    }
    pub fn self_keys(&self) -> Vec<(String, String)> {
        self.self_.borrow().keys().cloned().collect()
    }
    pub fn self_snapshot(&self, page_key: &str, node_id: &str) -> BTreeMap<String, Value> {
        self.self_
            .borrow()
            .get(&(page_key.to_owned(), node_id.to_owned()))
            .into_iter()
            .flat_map(|fields| fields.iter())
            .map(|(key, signal)| (key.clone(), signal.get().0))
            .collect()
    }
    pub fn vars_snapshot(&self) -> BTreeMap<String, Value> {
        self.vars
            .borrow()
            .iter()
            .map(|(key, signal)| (key.clone(), signal.get().0))
            .collect()
    }
    pub fn route_snapshot(&self) -> BTreeMap<String, Value> {
        self.route
            .borrow()
            .iter()
            .map(|(key, signal)| (key.clone(), signal.get().0))
            .collect()
    }
    pub fn storage_snapshot(&self) -> BTreeMap<String, Value> {
        self.storage
            .borrow()
            .iter()
            .map(|(key, signal)| (key.clone(), signal.get().0))
            .collect()
    }
    pub fn viewport_snapshot(&self) -> BTreeMap<String, Value> {
        self.viewport
            .borrow()
            .iter()
            .map(|(key, signal)| (key.clone(), signal.get().0))
            .collect()
    }

    pub fn replace_app(&self, values: &BTreeMap<String, Value>) {
        self.app
            .borrow_mut()
            .retain(|key, _| values.contains_key(key));
        for (key, value) in values {
            self.app_set(key, value.clone());
        }
    }
    pub fn replace_page(&self, page_key: &str, values: &BTreeMap<String, Value>) {
        if let Some(fields) = self.page.borrow_mut().get_mut(page_key) {
            fields.retain(|key, _| values.contains_key(key));
        }
        for (key, value) in values {
            self.page_set(page_key, key, value.clone());
        }
    }
    pub fn replace_self(&self, page_key: &str, node_id: &str, values: &BTreeMap<String, Value>) {
        if let Some(fields) = self
            .self_
            .borrow_mut()
            .get_mut(&(page_key.to_owned(), node_id.to_owned()))
        {
            fields.retain(|key, _| values.contains_key(key));
        }
        for (key, value) in values {
            self.self_set(page_key, node_id, key, value.clone());
        }
    }
    pub fn replace_vars(&self, values: &BTreeMap<String, Value>) {
        self.vars
            .borrow_mut()
            .retain(|key, _| values.contains_key(key));
        for (key, value) in values {
            self.vars_set(key, value.clone());
        }
    }
    pub fn replace_route(&self, values: &BTreeMap<String, Value>) {
        self.route
            .borrow_mut()
            .retain(|key, _| values.contains_key(key));
        for (key, value) in values {
            self.route_set(key, value.clone());
        }
    }
    pub fn replace_storage(&self, values: &BTreeMap<String, Value>) {
        self.storage
            .borrow_mut()
            .retain(|key, _| values.contains_key(key));
        for (key, value) in values {
            self.storage_set(key, value.clone());
        }
    }
    pub fn replace_viewport(&self, values: &BTreeMap<String, Value>) {
        self.viewport
            .borrow_mut()
            .retain(|key, _| values.contains_key(key));
        for (key, value) in values {
            let runtime = RuntimeValue(value.clone());
            let mut viewport = self.viewport.borrow_mut();
            if let Some(signal) = viewport.get(key) {
                signal.set(runtime);
            } else {
                viewport.insert(key.clone(), Signal::new(runtime, self.scheduler.clone()));
            }
        }
    }

    pub fn new(scheduler: Rc<Scheduler>) -> Self {
        Self::new_with_counter(scheduler, Rc::new(Cell::new(0)))
    }

    pub fn new_with_counter(scheduler: Rc<Scheduler>, mutation_counter: Rc<Cell<u64>>) -> Self {
        Self {
            scheduler: scheduler.clone(),
            mutation_counter,
            app: RefCell::new(BTreeMap::new()),
            page: RefCell::new(BTreeMap::new()),
            self_: RefCell::new(BTreeMap::new()),
            route: RefCell::new(BTreeMap::new()),
            storage: RefCell::new(BTreeMap::new()),
            storage_cache: Rc::new(storage_cache::StorageCache::new(scheduler.clone())),
            responsive: Cell::new(false),
            vars: RefCell::new(BTreeMap::new()),
            viewport: RefCell::new(BTreeMap::new()),
            image_keys: RefCell::new(BTreeMap::new()),
            now_ms: Cell::new(0),
        }
    }

    pub fn set_now_ms(&self, now_ms: u64) {
        self.now_ms.set(self.now_ms.get().max(now_ms));
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms.get()
    }

    pub fn set_image_key(&self, authored: &str, canonical: &str) {
        self.image_keys
            .borrow_mut()
            .insert(authored.to_owned(), canonical.to_owned());
    }

    pub fn image_key(&self, authored: &str) -> Option<String> {
        self.image_keys.borrow().get(authored).cloned()
    }

    pub fn clear_image_keys(&self) {
        self.image_keys.borrow_mut().clear();
    }

    /// Create or update a state variable in the app scope.
    pub fn app_set(&self, name: &str, value: Value) {
        self.bump_mutation();
        let rv = RuntimeValue(value.clone());
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
        self.bump_mutation();
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
        self.bump_mutation();
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

    pub fn page_get(&self, page_id: &str, name: &str) -> Option<RuntimeValue> {
        self.page.borrow().get(page_id)?.get(name).map(Signal::get)
    }

    pub fn self_set(&self, page_key: &str, node_id: &str, name: &str, value: Value) {
        self.bump_mutation();
        let rv = RuntimeValue(value);
        let mut map = self.self_.borrow_mut();
        let entry = map
            .entry((page_key.to_owned(), node_id.to_owned()))
            .or_default();
        if let Some(sig) = entry.get(name) {
            sig.set(rv);
        } else {
            let sig = Signal::new(rv, self.scheduler.clone());
            entry.insert(name.to_owned(), sig);
        }
    }

    pub fn self_get(&self, page_key: &str, node_id: &str, name: &str) -> Option<RuntimeValue> {
        self.self_
            .borrow()
            .get(&(page_key.to_owned(), node_id.to_owned()))?
            .get(name)
            .map(Signal::get)
    }

    pub fn self_signal(
        &self,
        page_key: &str,
        node_id: &str,
        name: &str,
    ) -> Option<Signal<RuntimeValue>> {
        self.self_
            .borrow()
            .get(&(page_key.to_owned(), node_id.to_owned()))?
            .get(name)
            .cloned()
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
        for ((page_key, node), fields) in self.self_.borrow().iter() {
            if !page_key.is_empty() {
                continue;
            }
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
                self.self_set("", node_id, name, value.clone());
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
            if sig.get() == rv {
                return;
            }
            self.bump_mutation();
            sig.set(rv);
        } else {
            self.bump_mutation();
            let sig = Signal::new(rv, self.scheduler.clone());
            map.insert(name.to_owned(), sig);
        }
    }

    /// `$storage.<name>` setter mirroring [`Self::app_set`]. Used by
    /// [`Self::restore_default_state`]. Hosts that wire a real
    /// storage backend layer their reads on top.
    pub fn storage_set(&self, name: &str, value: Value) {
        self.bump_mutation();
        let rv = RuntimeValue(value.clone());
        let mut map = self.storage.borrow_mut();
        if let Some(sig) = map.get(name) {
            sig.set(rv);
        } else {
            let sig = Signal::new(rv, self.scheduler.clone());
            map.insert(name.to_owned(), sig);
        }
        if self.responsive.get() {
            self.storage_cache.set_local(name, value);
        }
    }

    pub fn set_responsive(&self, responsive: bool) {
        self.responsive.set(responsive);
    }

    pub fn is_responsive(&self) -> bool {
        self.responsive.get()
    }

    pub fn set_viewport(&self, width: f32, height: f32, dpr: f32) {
        for (key, value) in [("width", width), ("height", height), ("dpr", dpr)] {
            let runtime = RuntimeValue(serde_json::json!(value));
            let mut viewport = self.viewport.borrow_mut();
            if let Some(signal) = viewport.get(key) {
                signal.set(runtime);
            } else {
                viewport.insert(key.into(), Signal::new(runtime, self.scheduler.clone()));
            }
        }
    }

    pub(crate) fn bump_mutation(&self) {
        self.mutation_counter
            .set(self.mutation_counter.get().wrapping_add(1));
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
                    .get(&(context_page.unwrap_or("").to_owned(), nid.to_owned()))?
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
            Scope::Viewport if self.responsive.get() => self
                .viewport
                .borrow()
                .get(path.segments.first().and_then(seg_as_key)?)
                .cloned()?,
            Scope::Viewport => return None,
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
    fn equal_route_write_does_not_mutate_or_notify() {
        let scheduler = Rc::new(Scheduler::new());
        let mutations = Rc::new(Cell::new(0));
        let graph = StateGraph::new_with_counter(scheduler, mutations.clone());
        graph.route_set("path", json!("/detail"));
        let first_mutation = mutations.get();
        let first_version = graph.route.borrow()["path"].version();

        graph.route_set("path", json!("/detail"));

        assert_eq!(mutations.get(), first_mutation);
        assert_eq!(graph.route.borrow()["path"].version(), first_version);
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
        g.self_set("", "btn", "hover", json!(false));
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

    #[test]
    fn self_state_isolated_per_page_key() {
        let g = StateGraph::new(Rc::new(Scheduler::new()));
        g.self_set("home-m@0-480", "card", "count", json!(3));
        g.self_set("home", "card", "count", json!(9));
        assert_eq!(
            g.self_get("home-m@0-480", "card", "count").unwrap().0,
            json!(3)
        );
        assert_eq!(g.self_get("home", "card", "count").unwrap().0, json!(9));
    }
}
