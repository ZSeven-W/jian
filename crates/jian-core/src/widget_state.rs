//! Per-node runtime state for interactive widget nodes, keyed by node
//! id. Lazily created on first access, seeded from the node's authored
//! props; survives `replace_document` for ids that still exist.

use crate::state::StateGraph;
use crate::text_input::TextInputState;
use jian_ops_schema::node::{BoolOrExpression, NumberOrExpression, PenNode};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum WidgetState {
    /// Shared by text_input, text_area and number_input (the latter
    /// edits its numeric value as text, gated to digits by the host).
    TextInput(TextInputState),
    /// Shared by switch and checkbox.
    Toggle {
        on: bool,
    },
    Select {
        open: bool,
        value: Option<String>,
        hover_index: Option<usize>,
    },
    Slider {
        value: f64,
        dragging: bool,
    },
    /// Radio group — selected option value + keyboard hover row.
    Radio {
        value: Option<String>,
        hover_index: Option<usize>,
    },
    /// Tabs — active tab value + keyboard hover row.
    Tabs {
        active: Option<String>,
        hover_index: Option<usize>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WidgetKind {
    TextInput,
    TextArea,
    NumberInput,
    Switch,
    Checkbox,
    Select,
    RadioGroup,
    Slider,
    Tabs,
}

#[derive(Debug, Clone)]
pub struct WidgetStateStore {
    map: HashMap<(String, String), WidgetState>,
    kinds: HashMap<(String, String), WidgetKind>,
    page_key: String,
    mutation_counter: Rc<Cell<u64>>,
}

impl Default for WidgetStateStore {
    fn default() -> Self {
        Self::with_counter(Rc::new(Cell::new(0)))
    }
}

impl WidgetStateStore {
    pub fn with_counter(mutation_counter: Rc<Cell<u64>>) -> Self {
        Self {
            map: HashMap::new(),
            kinds: HashMap::new(),
            page_key: String::new(),
            mutation_counter,
        }
    }

    pub fn clone_with_counter(&self, mutation_counter: Rc<Cell<u64>>) -> Self {
        Self {
            map: self.map.clone(),
            kinds: self.kinds.clone(),
            page_key: self.page_key.clone(),
            mutation_counter,
        }
    }

    pub fn set_mutation_counter(&mut self, mutation_counter: Rc<Cell<u64>>) {
        self.mutation_counter = mutation_counter;
    }

    fn bump_mutation(&self) {
        self.mutation_counter
            .set(self.mutation_counter.get().wrapping_add(1));
    }

    pub fn set_page_key(&mut self, page_key: impl Into<String>) {
        self.bump_mutation();
        self.page_key = page_key.into();
    }

    pub fn page_key(&self) -> &str {
        &self.page_key
    }
    /// Seed-from-node on first access. Returns `None` for nodes that
    /// carry no interactive runtime state (e.g. `progress`, which is
    /// display-only and reads its value straight from the state graph).
    ///
    /// When the node's `bindings["bind:value"]` targets `$state.<key>`
    /// and the state graph already holds a persisted value for that
    /// key (e.g. after a screen switch re-mounts a bound widget), the
    /// freshly-built seed is overridden with that value instead of the
    /// authored prop — this is how bound input values survive
    /// navigation. Live `Occupied` state of a matching variant is
    /// never touched here.
    pub fn get_or_init(&mut self, node: &PenNode, state: &StateGraph) -> Option<&mut WidgetState> {
        self.bump_mutation();
        let (id, kind, mut init) = match node {
            PenNode::TextInput(n) => (
                &n.base.id,
                WidgetKind::TextInput,
                WidgetState::TextInput(TextInputState::with_text(
                    n.value.clone().unwrap_or_default(),
                )),
            ),
            PenNode::TextArea(n) => (
                &n.base.id,
                WidgetKind::TextArea,
                WidgetState::TextInput(TextInputState::with_text(
                    n.value.clone().unwrap_or_default(),
                )),
            ),
            PenNode::NumberInput(n) => (
                &n.base.id,
                WidgetKind::NumberInput,
                WidgetState::TextInput(TextInputState::with_text(number_to_text(&n.value))),
            ),
            PenNode::Switch(n) => (
                &n.base.id,
                WidgetKind::Switch,
                WidgetState::Toggle {
                    on: bool_default(&n.checked),
                },
            ),
            PenNode::Checkbox(n) => (
                &n.base.id,
                WidgetKind::Checkbox,
                WidgetState::Toggle {
                    on: bool_default(&n.checked),
                },
            ),
            PenNode::Select(n) => (
                &n.base.id,
                WidgetKind::Select,
                WidgetState::Select {
                    open: false,
                    value: n.value.clone(),
                    hover_index: None,
                },
            ),
            PenNode::RadioGroup(n) => (
                &n.base.id,
                WidgetKind::RadioGroup,
                WidgetState::Radio {
                    value: n.value.clone(),
                    hover_index: None,
                },
            ),
            PenNode::Slider(n) => (
                &n.base.id,
                WidgetKind::Slider,
                WidgetState::Slider {
                    value: number_default(&n.value, n.min.unwrap_or(0.0)),
                    dragging: false,
                },
            ),
            PenNode::Tabs(n) => (
                &n.base.id,
                WidgetKind::Tabs,
                WidgetState::Tabs {
                    active: n.value.clone(),
                    hover_index: None,
                },
            ),
            _ => return None,
        };
        if let Some(v) = bound_app_value(node, state) {
            apply_bound(&mut init, &v);
        }
        let key = (self.page_key.clone(), id.clone());
        if self.kinds.get(&key) != Some(&kind) || !self.map.contains_key(&key) {
            self.map.insert(key.clone(), init);
            self.kinds.insert(key.clone(), kind);
        }
        self.map.get_mut(&key)
    }

    pub fn get(&self, id: &str) -> Option<&WidgetState> {
        self.map.get(&(self.page_key.clone(), id.to_owned()))
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut WidgetState> {
        self.bump_mutation();
        self.map.get_mut(&(self.page_key.clone(), id.to_owned()))
    }

    pub fn get_for_page(&self, page_key: &str, id: &str) -> Option<&WidgetState> {
        self.map.get(&(page_key.to_owned(), id.to_owned()))
    }

    pub fn get_for_page_mut(&mut self, page_key: &str, id: &str) -> Option<&mut WidgetState> {
        self.bump_mutation();
        self.map.get_mut(&(page_key.to_owned(), id.to_owned()))
    }

    /// Iterate `(id, state)` pairs — used to locate the slider currently
    /// being dragged without re-resolving every node from the document.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &WidgetState)> {
        let page_key = self.page_key.as_str();
        self.map.iter().filter_map(move |((page, id), state)| {
            (page == page_key).then_some((id.as_str(), state))
        })
    }

    /// Mutable iterator over the states — used to clear transient flags
    /// (e.g. a slider's `dragging`) on pointer up.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut WidgetState> {
        self.bump_mutation();
        let page_key = self.page_key.clone();
        self.map
            .iter_mut()
            .filter_map(move |((page, _), state)| (page == &page_key).then_some(state))
    }

    /// Drop state for nodes that no longer exist (document swap).
    pub fn retain_ids(&mut self, live: &dyn Fn(&str) -> bool) {
        self.bump_mutation();
        let page_key = self.page_key.as_str();
        self.map
            .retain(|(page, id), _| page != page_key || live(id));
        self.kinds
            .retain(|(page, id), _| page != page_key || live(id));
    }

    pub fn clear(&mut self) {
        self.bump_mutation();
        self.map.clear();
        self.kinds.clear();
    }

    pub fn reset_transients(&mut self) {
        self.bump_mutation();
        for state in self.map.values_mut() {
            match state {
                WidgetState::TextInput(text) => text.reset_transient(),
                WidgetState::Select {
                    open, hover_index, ..
                } => {
                    *open = false;
                    *hover_index = None;
                }
                WidgetState::Slider { dragging, .. } => *dragging = false,
                WidgetState::Radio { hover_index, .. } | WidgetState::Tabs { hover_index, .. } => {
                    *hover_index = None
                }
                WidgetState::Toggle { .. } => {}
            }
        }
    }

    pub fn revalidate(&mut self, document: &crate::document::RuntimeDocument, state: &StateGraph) {
        let keys: Vec<(String, String)> = self
            .map
            .keys()
            .filter(|(page, _)| page == &self.page_key)
            .cloned()
            .collect();
        for key in keys {
            let Some(node_key) = document.tree.get(&key.1) else {
                continue;
            };
            let schema = &document.tree.nodes[node_key].schema;
            let Some(kind) = widget_kind(schema) else {
                self.map.remove(&key);
                self.kinds.remove(&key);
                continue;
            };
            let mut fresh_store = WidgetStateStore::default();
            fresh_store.set_page_key(key.0.clone());
            let fresh = fresh_store.get_or_init(schema, state).cloned();
            if self.kinds.get(&key) != Some(&kind) {
                if let Some(fresh) = fresh {
                    self.map.insert(key.clone(), fresh);
                    self.kinds.insert(key, kind);
                }
                continue;
            }
            let Some(current) = self.map.get_mut(&key) else {
                continue;
            };
            let json = serde_json::to_value(schema).unwrap_or_default();
            match current {
                WidgetState::Select { value, .. } | WidgetState::Radio { value, .. } => {
                    let valid = json
                        .get("options")
                        .and_then(|options| options.as_array())
                        .is_some_and(|options| {
                            value.as_ref().map_or(true, |selected| {
                                options.iter().any(|option| {
                                    option.get("value").and_then(|value| value.as_str())
                                        == Some(selected)
                                })
                            })
                        });
                    if !valid {
                        if let Some(fresh) = fresh.clone() {
                            *current = fresh;
                        }
                    }
                }
                WidgetState::Tabs { active, .. } => {
                    let valid = json
                        .get("tabs")
                        .and_then(|tabs| tabs.as_array())
                        .is_some_and(|tabs| {
                            active.as_ref().map_or(true, |selected| {
                                tabs.iter().any(|tab| {
                                    tab.get("value").and_then(|value| value.as_str())
                                        == Some(selected)
                                })
                            })
                        });
                    if !valid {
                        if let Some(fresh) = fresh.clone() {
                            *current = fresh;
                        }
                    }
                }
                WidgetState::Slider { value, .. } => {
                    let min = json
                        .get("min")
                        .and_then(|value| value.as_f64())
                        .unwrap_or(0.0);
                    let max = json
                        .get("max")
                        .and_then(|value| value.as_f64())
                        .unwrap_or(100.0);
                    *value = value.min(max).max(min);
                }
                WidgetState::TextInput(text) if matches!(schema, PenNode::NumberInput(_)) => {
                    let min = json
                        .get("min")
                        .and_then(|value| value.as_f64())
                        .unwrap_or(f64::NEG_INFINITY);
                    let max = json
                        .get("max")
                        .and_then(|value| value.as_f64())
                        .unwrap_or(f64::INFINITY);
                    if let Ok(value) = text.text().parse::<f64>() {
                        text.set_text(fmt_num(value.min(max).max(min)));
                    } else if let Some(fresh) = fresh.clone() {
                        *current = fresh;
                    }
                }
                _ => {}
            }
        }
    }
}

fn widget_kind(node: &PenNode) -> Option<WidgetKind> {
    match node {
        PenNode::TextInput(_) => Some(WidgetKind::TextInput),
        PenNode::TextArea(_) => Some(WidgetKind::TextArea),
        PenNode::NumberInput(_) => Some(WidgetKind::NumberInput),
        PenNode::Switch(_) => Some(WidgetKind::Switch),
        PenNode::Checkbox(_) => Some(WidgetKind::Checkbox),
        PenNode::Select(_) => Some(WidgetKind::Select),
        PenNode::RadioGroup(_) => Some(WidgetKind::RadioGroup),
        PenNode::Slider(_) => Some(WidgetKind::Slider),
        PenNode::Tabs(_) => Some(WidgetKind::Tabs),
        _ => None,
    }
}

/// The node's `bind:value` target as an app-scope key (`$state.<key>`
/// binds into app scope — see `Runtime::sync_bound_value`), when the
/// state graph currently holds a value for it.
///
/// Once the bound key exists in app scope — whether from a declared
/// document-state default seeded at load, or a prior write — the state
/// graph is the source of truth for the widget's seed. This applies to
/// the FIRST mount as much as to re-mounts after `replace_document`:
/// a declared default deliberately wins over a conflicting authored
/// widget prop.
fn bound_app_value(node: &PenNode, state: &StateGraph) -> Option<serde_json::Value> {
    let bindings = match node {
        PenNode::TextInput(n) => n.bindings.as_ref(),
        PenNode::TextArea(n) => n.bindings.as_ref(),
        PenNode::NumberInput(n) => n.bindings.as_ref(),
        PenNode::Switch(n) => n.bindings.as_ref(),
        PenNode::Checkbox(n) => n.bindings.as_ref(),
        PenNode::Select(n) => n.bindings.as_ref(),
        PenNode::RadioGroup(n) => n.bindings.as_ref(),
        PenNode::Slider(n) => n.bindings.as_ref(),
        PenNode::Tabs(n) => n.bindings.as_ref(),
        _ => None,
    }?;
    let key = bindings
        .get("bind:value")?
        .as_str()
        .strip_prefix("$state.")?;
    state.app_get(key).map(|rv| rv.0)
}

/// Overwrite a freshly-built seed with the bound persisted value.
fn apply_bound(init: &mut WidgetState, v: &serde_json::Value) {
    match init {
        WidgetState::TextInput(t) => {
            if let Some(s) = v.as_str() {
                *t = TextInputState::with_text(s);
            } else if let Some(n) = v.as_f64() {
                *t = TextInputState::with_text(n.to_string());
            }
        }
        WidgetState::Toggle { on } => {
            if let Some(b) = v.as_bool() {
                *on = b;
            }
        }
        WidgetState::Select { value, .. } | WidgetState::Radio { value, .. } => {
            if let Some(s) = v.as_str() {
                *value = Some(s.to_owned());
            }
        }
        WidgetState::Tabs { active, .. } => {
            if let Some(s) = v.as_str() {
                *active = Some(s.to_owned());
            }
        }
        WidgetState::Slider { value, .. } => {
            if let Some(n) = v.as_f64() {
                *value = n;
            }
        }
    }
}

fn bool_default(v: &Option<BoolOrExpression>) -> bool {
    matches!(v, Some(BoolOrExpression::Bool(true)))
}

fn number_default(v: &Option<NumberOrExpression>, fallback: f64) -> f64 {
    match v {
        Some(NumberOrExpression::Number(n)) => *n,
        _ => fallback,
    }
}

fn number_to_text(v: &Option<NumberOrExpression>) -> String {
    match v {
        Some(NumberOrExpression::Number(n)) => fmt_num(*n),
        _ => String::new(),
    }
}

/// Format a seed number without a trailing `.0` for whole values.
fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(json: &str) -> PenNode {
        serde_json::from_str(json).unwrap()
    }

    /// Fresh, empty state graph for tests that don't care about
    /// bound-value read-back.
    fn empty_state() -> StateGraph {
        StateGraph::new(std::rc::Rc::new(crate::signal::scheduler::Scheduler::new()))
    }

    #[test]
    fn widget_state_isolated_per_page_key() {
        let mut store = WidgetStateStore::default();
        let state = empty_state();
        let input = node(r#"{"type":"text_input","id":"same","value":"a"}"#);
        store.set_page_key("mobile");
        if let Some(WidgetState::TextInput(text)) = store.get_or_init(&input, &state) {
            text.insert_str("!", 0);
        }
        store.set_page_key("desktop");
        match store.get_or_init(&input, &state) {
            Some(WidgetState::TextInput(text)) => assert_eq!(text.text(), "a"),
            _ => panic!(),
        }
        store.set_page_key("mobile");
        match store.get("same") {
            Some(WidgetState::TextInput(text)) => assert_eq!(text.text(), "a!"),
            _ => panic!(),
        }
    }

    #[test]
    fn mutable_access_bumps_shared_counter_and_transients_reset_across_pages() {
        use std::cell::Cell;
        let counter = std::rc::Rc::new(Cell::new(0));
        let mut store = WidgetStateStore::with_counter(counter.clone());
        let state = empty_state();
        let input = node(r#"{"type":"text_input","id":"field","value":"durable"}"#);
        store.set_page_key("a");
        let _ = store.get_or_init(&input, &state);
        assert!(counter.get() > 0);
        if let Some(WidgetState::TextInput(text)) = store.get_mut("field") {
            text.set_composition("preedit", 7, 0);
            text.select_all();
        }
        store.set_page_key("b");
        let _ = store.get_or_init(&input, &state);
        store.reset_transients();
        match store.get_for_page("a", "field") {
            Some(WidgetState::TextInput(text)) => {
                assert_eq!(text.text(), "durable");
                assert!(text.composition().is_none());
                assert!(text.highlight_range().is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn text_input_seeds_from_value_and_persists_edits() {
        let mut store = WidgetStateStore::default();
        let state = empty_state();
        let n = node(r#"{"type":"text_input","id":"i","value":"hi"}"#);
        match store.get_or_init(&n, &state).unwrap() {
            WidgetState::TextInput(st) => assert_eq!(st.text(), "hi"),
            _ => panic!(),
        }
        // Mutate, then re-init: existing state is kept (not reset).
        if let Some(WidgetState::TextInput(st)) = store.get_mut("i") {
            st.insert_str("!", 0);
        }
        match store.get_or_init(&n, &state).unwrap() {
            WidgetState::TextInput(st) => assert_eq!(st.text(), "hi!"),
            _ => panic!(),
        }
    }

    #[test]
    fn number_input_seeds_text_without_trailing_zero() {
        let mut store = WidgetStateStore::default();
        let state = empty_state();
        let n = node(r#"{"type":"number_input","id":"n","value":3}"#);
        match store.get_or_init(&n, &state).unwrap() {
            WidgetState::TextInput(st) => assert_eq!(st.text(), "3"),
            _ => panic!(),
        }
    }

    #[test]
    fn toggle_and_select_and_radio_and_slider_and_tabs_seed() {
        let mut store = WidgetStateStore::default();
        let state = empty_state();
        let sw = node(r#"{"type":"switch","id":"s","checked":true}"#);
        assert!(matches!(
            store.get_or_init(&sw, &state),
            Some(WidgetState::Toggle { on: true })
        ));
        let sel = node(r#"{"type":"select","id":"sel","value":"a"}"#);
        assert!(
            matches!(store.get_or_init(&sel, &state), Some(WidgetState::Select { value, .. }) if value.as_deref() == Some("a"))
        );
        let rg = node(r#"{"type":"radio_group","id":"rg","value":"b"}"#);
        assert!(
            matches!(store.get_or_init(&rg, &state), Some(WidgetState::Radio { value, .. }) if value.as_deref() == Some("b"))
        );
        let sl = node(r#"{"type":"slider","id":"sl","min":10,"max":20}"#);
        assert!(
            matches!(store.get_or_init(&sl, &state), Some(WidgetState::Slider { value, .. }) if (*value - 10.0).abs() < f64::EPSILON)
        );
        let tb = node(r#"{"type":"tabs","id":"tb","value":"one"}"#);
        assert!(
            matches!(store.get_or_init(&tb, &state), Some(WidgetState::Tabs { active, .. }) if active.as_deref() == Some("one"))
        );
    }

    #[test]
    fn progress_and_non_widget_have_no_state() {
        let mut store = WidgetStateStore::default();
        let state = empty_state();
        let pg = node(r#"{"type":"progress","id":"pg","value":40}"#);
        assert!(store.get_or_init(&pg, &state).is_none());
        let frame = node(r#"{"type":"frame","id":"f"}"#);
        assert!(store.get_or_init(&frame, &state).is_none());
    }

    #[test]
    fn get_or_init_reseeds_when_node_type_changes_at_same_id() {
        let mut store = WidgetStateStore::default();
        let state = empty_state();
        store.get_or_init(
            &node(r#"{"type":"text_input","id":"x","value":"hi"}"#),
            &state,
        );
        // Same id reused as a switch after a doc swap: state must re-seed
        // to the new variant, not hand back stale TextInput state.
        let st = store.get_or_init(
            &node(r#"{"type":"switch","id":"x","checked":true}"#),
            &state,
        );
        assert!(matches!(st, Some(WidgetState::Toggle { on: true })));
    }

    #[test]
    fn retain_ids_drops_dead_entries() {
        let mut store = WidgetStateStore::default();
        let state = empty_state();
        store.get_or_init(&node(r#"{"type":"text_input","id":"keep"}"#), &state);
        store.get_or_init(&node(r#"{"type":"text_input","id":"drop"}"#), &state);
        store.retain_ids(&|id| id == "keep");
        assert!(store.get_mut("keep").is_some());
        assert!(store.get_mut("drop").is_none());
    }

    #[test]
    fn bound_widget_seeds_from_app_state_when_present() {
        let state = empty_state();
        state.app_set("email", serde_json::json!("persisted@x.y"));
        let n: PenNode = serde_json::from_str(
            r#"{"type":"text_input","id":"email-input","value":"authored",
                "bindings":{"bind:value":"$state.email"}}"#,
        )
        .unwrap();
        let mut store = WidgetStateStore::default();
        match store.get_or_init(&n, &state).unwrap() {
            WidgetState::TextInput(t) => assert_eq!(t.text(), "persisted@x.y"),
            other => panic!("unexpected variant {other:?}"),
        }
    }

    #[test]
    fn declared_state_default_overrides_authored_value_at_first_mount() {
        // Crux semantics, locked as INTENDED: when a document DECLARES
        // a state key with a default, the loader seeds app scope at
        // load — so a bound widget's very first mount seeds from that
        // declared default, NOT from its conflicting authored prop.
        // State graph is the source of truth once the key exists.
        let rt = crate::Runtime::new_from_document(
            serde_json::from_str::<jian_ops_schema::document::PenDocument>(
                r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"choice":{"type":"string","default":"b"}},
                  "children":[
                    {"type":"frame","id":"root","children":[
                      {"type":"select","id":"se","value":"a",
                       "options":[{"value":"a","label":"A"},{"value":"b","label":"B"}],
                       "bindings":{"bind:value":"$state.choice"}}]}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let doc = rt.document.as_ref().unwrap();
        let key = doc.tree.get("se").unwrap();
        let schema = doc.tree.nodes[key].schema.clone();
        let mut store = WidgetStateStore::default();
        match store.get_or_init(&schema, &rt.state).unwrap() {
            WidgetState::Select { value, .. } => assert_eq!(value.as_deref(), Some("b")),
            other => panic!("unexpected variant {other:?}"),
        }
    }

    #[test]
    fn unbound_widget_seeds_from_authored_props() {
        let state = empty_state();
        let n: PenNode =
            serde_json::from_str(r#"{"type":"text_input","id":"plain","value":"authored"}"#)
                .unwrap();
        let mut store = WidgetStateStore::default();
        match store.get_or_init(&n, &state).unwrap() {
            WidgetState::TextInput(t) => assert_eq!(t.text(), "authored"),
            other => panic!("unexpected variant {other:?}"),
        }
    }
}
