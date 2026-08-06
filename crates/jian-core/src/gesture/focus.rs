//! FocusManager — Tab-tree keyboard focus.
//!
//! Plan 5 Task 11. The focus chain is a flat `Vec<NodeKey>` rebuilt
//! from a DFS pre-order over the document's runtime tree, filtered by
//! `gestures.focusable` and `semantics.role`. The manager itself only
//! knows about chain mechanics; document-aware chain construction
//! lives in [`collect_focus_chain`] so unit tests can drive
//! `FocusManager` with synthetic chains.
//!
//! Wiring lives on `Runtime` — see `Runtime::dispatch_keyboard`,
//! `focus_next`, `focus_previous`, `focus_request`, `focus_clear`.

use crate::document::{NodeKey, RuntimeDocument};
use crate::widget_state::{WidgetState, WidgetStateStore};
use serde::{Deserialize, Serialize};

/// Window-level / widget-level focus transition event. Hosts emit
/// this for browser DOM `focus` / `blur` and winit `Focused` so
/// widget code can react without knowing about each platform's
/// native event shape. This is **not** the same as `FocusManager`'s
/// internal `FocusChange` (which tracks the Tab-ring chain) — the
/// public event sits at the host boundary, the chain mechanics stay
/// inside the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusEvent {
    pub gained: bool,
    /// Optional widget node id the host believes received focus.
    /// `None` = window-level focus / no specific node hint.
    pub node_id_hint: Option<u64>,
    /// Mirror of W3C `FocusEvent.relatedTarget`. The node id of the
    /// other half of the focus transition (the node losing focus on
    /// `gained=true`, or the node gaining focus on `gained=false`).
    /// `None` when the host cannot identify the counterpart (e.g. the
    /// page just gained focus from a non-DOM source).
    pub related_node_id_hint: Option<u64>,
}

/// Result of a focus mutation. The runtime turns this into a pair of
/// `SemanticEvent::FocusLost` (for `previous`) and
/// `SemanticEvent::FocusGained` (for `current`) — the tuple form keeps
/// the manager free of `SemanticEvent`/event-payload concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FocusChange {
    /// The node that lost focus. `None` when nothing was focused before
    /// (first Tab into the chain) or when the change was a no-op.
    pub previous: Option<NodeKey>,
    /// The node that gained focus. `None` when focus was cleared or the
    /// chain has no focusable nodes.
    pub current: Option<NodeKey>,
}

impl FocusChange {
    /// `true` when neither side moved (e.g. `next()` on an empty chain
    /// or a `request` for the already-focused node). Callers skip
    /// emitting `FocusLost`/`FocusGained` in that case.
    pub fn is_noop(self) -> bool {
        self.previous == self.current
    }
}

#[derive(Debug, Default, Clone)]
pub struct FocusManager {
    /// Ordered focus ring (DFS pre-order over the document tree,
    /// filtered to focusable nodes). The runtime rebuilds this on
    /// every document load via [`collect_focus_chain`] and
    /// [`Self::set_chain`].
    chain: Vec<NodeKey>,
    current: Option<NodeKey>,
}

impl FocusManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the focus chain. Any previously-focused node that is no
    /// longer in the chain is cleared silently — the runtime emits no
    /// `FocusLost` because the handler couldn't run on a stale key.
    ///
    /// **Caveat for hot-reload:** SlotMap keys are *not* globally
    /// unique — two different `NodeTree` instances can produce keys
    /// that compare equal (same index, same starting generation). So
    /// `chain.contains(&cur)` is not sufficient to detect a stale
    /// key. `Runtime::replace_document` calls [`Self::clear`]
    /// before `set_chain` for that reason; in-tree mutations (where
    /// `set_chain` is called against the *same* SlotMap) are safe to
    /// rely on the contains-check alone.
    pub fn set_chain(&mut self, chain: Vec<NodeKey>) {
        self.chain = chain;
        if let Some(cur) = self.current {
            if !self.chain.contains(&cur) {
                self.current = None;
            }
        }
    }

    pub fn chain(&self) -> &[NodeKey] {
        &self.chain
    }

    pub fn current(&self) -> Option<NodeKey> {
        self.current
    }

    /// Move focus onto an explicit node. Returns the change record;
    /// `is_noop()` is true when `node` is the already-focused node, or
    /// when `node` isn't in the focus chain (rejected — the manager's
    /// invariant is `current ∈ chain ∪ {None}`).
    pub fn request(&mut self, node: NodeKey) -> FocusChange {
        if !self.chain.contains(&node) {
            return FocusChange {
                previous: self.current,
                current: self.current,
            };
        }
        if self.current == Some(node) {
            return FocusChange {
                previous: Some(node),
                current: Some(node),
            };
        }
        let prev = self.current;
        self.current = Some(node);
        FocusChange {
            previous: prev,
            current: Some(node),
        }
    }

    /// Drop focus. Returns the previously-focused node (if any) so the
    /// runtime can emit `FocusLost`.
    pub fn clear(&mut self) -> FocusChange {
        let prev = self.current.take();
        FocusChange {
            previous: prev,
            current: None,
        }
    }

    /// Move forward in the chain (`Tab`). Wraps from the last node back
    /// to the first. No-op when the chain is empty.
    #[allow(clippy::should_implement_trait)] // FocusManager isn't an Iterator
    pub fn next(&mut self) -> FocusChange {
        self.step(1)
    }

    /// Move backward in the chain (`Shift+Tab`). Wraps from the first
    /// node back to the last.
    pub fn previous(&mut self) -> FocusChange {
        self.step(-1)
    }

    fn step(&mut self, dir: i32) -> FocusChange {
        if self.chain.is_empty() {
            return FocusChange::default();
        }
        let len = self.chain.len() as i32;
        let next_idx: i32 = match self.current {
            None => {
                if dir > 0 {
                    0
                } else {
                    len - 1
                }
            }
            Some(cur) => {
                let idx = self
                    .chain
                    .iter()
                    .position(|&k| k == cur)
                    .map(|p| p as i32)
                    // current went stale (shouldn't happen post-set_chain
                    // sanitisation, but be defensive) — treat as if no
                    // focus and start from the chain edge.
                    .unwrap_or(if dir > 0 { -1 } else { len });
                let stepped = idx + dir;
                ((stepped % len) + len) % len
            }
        };
        let next = self.chain[next_idx as usize];
        let prev = self.current;
        self.current = Some(next);
        FocusChange {
            previous: prev,
            current: Some(next),
        }
    }
}

/// Collect the focus chain from `doc` in DFS pre-order. A node enters
/// the chain when:
///
/// 1. `gestures.focusable == Some(true)` (explicit opt-in always wins),
///    OR
/// 2. `gestures.focusable` is unset AND `semantics.role` is one of the
///    interactive defaults (`button`, `link`, `input`).
///
/// `gestures.focusable == Some(false)` always excludes the node, even
/// when its role would otherwise auto-include it — authors use this to
/// keep a decorative `Input` out of the Tab ring without rewriting the
/// role.
///
/// Reads the typed `gestures` / `semantics` blocks via
/// `PenNode::gestures_and_semantics` (defined in
/// `jian_ops_schema::node`) — historically this routed through
/// `serde_json::to_value(schema)` which re-serialised every
/// container's recursive `children` subtree on each lookup, so a
/// 1000-node document with nested frames was effectively quadratic
/// during a chain rebuild.
pub fn collect_focus_chain(doc: &RuntimeDocument) -> Vec<NodeKey> {
    collect_focus_chain_impl(doc, None)
}

/// Collect the focus chain using live widget state for tab selection.
///
/// The document-only public entry point above is used while a runtime is being
/// constructed. Once widget state exists, layout/spatial rebuilds use this
/// variant so changing a tab immediately removes the inactive panel from the
/// keyboard ring as well as from pointer hit-testing.
pub(crate) fn collect_focus_chain_with_states(
    doc: &RuntimeDocument,
    states: &WidgetStateStore,
) -> Vec<NodeKey> {
    collect_focus_chain_impl(doc, Some(states))
}

fn collect_focus_chain_impl(
    doc: &RuntimeDocument,
    states: Option<&WidgetStateStore>,
) -> Vec<NodeKey> {
    active_tree_nodes(doc, states)
        .into_iter()
        .filter(|&key| is_focusable(doc, key))
        .collect()
}

/// DFS pre-order containing only the currently active child subtree of each
/// tabs container. The tabs node itself remains in the result. A stale/missing
/// value selects the first declared tab, while an empty tab list exposes no
/// panel subtree.
pub(crate) fn active_tree_nodes(
    doc: &RuntimeDocument,
    states: Option<&WidgetStateStore>,
) -> Vec<NodeKey> {
    let mut out = Vec::with_capacity(doc.tree.nodes.len());
    for &root in &doc.tree.roots {
        walk_active(doc, root, states, &mut out);
    }
    out
}

fn walk_active(
    doc: &RuntimeDocument,
    key: NodeKey,
    states: Option<&WidgetStateStore>,
    out: &mut Vec<NodeKey>,
) {
    out.push(key);
    let node = &doc.tree.nodes[key];
    if let jian_ops_schema::node::PenNode::Tabs(tabs) = &node.schema {
        let selected = states
            .and_then(|store| match store.get(&tabs.base.id) {
                Some(WidgetState::Tabs { active, .. }) => Some(active.as_deref()),
                _ => None,
            })
            .unwrap_or(tabs.value.as_deref());
        let index = crate::widget_state::resolve_tab_index(
            tabs.tabs
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|tab| Some(tab.value.as_str())),
            selected,
        );
        if let Some(child) = index.and_then(|index| node.children.get(index)).copied() {
            walk_active(doc, child, states, out);
        }
        return;
    }
    for &child in &node.children {
        walk_active(doc, child, states, out);
    }
}

fn is_focusable(doc: &RuntimeDocument, key: NodeKey) -> bool {
    use jian_ops_schema::semantics::SemanticRole;
    let schema = &doc.tree.nodes[key].schema;
    let (gestures, semantics) = schema.gestures_and_semantics();
    // Explicit `gestures.focusable` overrides everything.
    if let Some(b) = gestures.and_then(|g| g.focusable) {
        return b;
    }
    // Interactive widget node types are focusable by type — no
    // `semantics.role` needed (the node type already carries the
    // meaning). `progress` is display-only and excluded. Authors who
    // want a widget out of the Tab ring set `gestures.focusable: false`
    // (handled above). NOTE: dynamic `semantics.disabled` /
    // `bindings.disabled` is NOT yet honoured here — the focus chain is
    // built without a `StateGraph`, so disabled-expression evaluation is
    // a deliberate follow-up (tracked in the widgets plan).
    if is_interactive_widget(schema) {
        return true;
    }
    // Otherwise consult semantics.role for the interactive defaults.
    matches!(
        semantics.and_then(|s| s.role),
        Some(SemanticRole::Button | SemanticRole::Link | SemanticRole::Input)
    )
}

/// Widget node variants that take keyboard focus (the whole family
/// except display-only `progress`).
fn is_interactive_widget(node: &jian_ops_schema::node::PenNode) -> bool {
    use jian_ops_schema::node::PenNode;
    matches!(
        node,
        PenNode::TextInput(_)
            | PenNode::TextArea(_)
            | PenNode::Select(_)
            | PenNode::Switch(_)
            | PenNode::Checkbox(_)
            | PenNode::Slider(_)
            | PenNode::RadioGroup(_)
            | PenNode::NumberInput(_)
            | PenNode::Tabs(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    fn synth(n: usize) -> Vec<NodeKey> {
        let mut sm: SlotMap<NodeKey, u32> = SlotMap::with_key();
        (0..n).map(|i| sm.insert(i as u32)).collect()
    }

    #[test]
    fn empty_chain_next_is_noop() {
        let mut f = FocusManager::new();
        assert!(f.next().is_noop());
        assert!(f.previous().is_noop());
        assert!(f.current().is_none());
    }

    #[test]
    fn next_wraps_around() {
        let keys = synth(3);
        let mut f = FocusManager::new();
        f.set_chain(keys.clone());

        let c = f.next();
        assert_eq!(c.previous, None);
        assert_eq!(c.current, Some(keys[0]));

        let c = f.next();
        assert_eq!(c.previous, Some(keys[0]));
        assert_eq!(c.current, Some(keys[1]));

        f.next();
        let c = f.next();
        // wrap from index 2 → 0
        assert_eq!(c.previous, Some(keys[2]));
        assert_eq!(c.current, Some(keys[0]));
    }

    #[test]
    fn previous_wraps_around() {
        let keys = synth(3);
        let mut f = FocusManager::new();
        f.set_chain(keys.clone());
        let c = f.previous();
        assert_eq!(c.previous, None);
        // First Shift+Tab from "no focus" lands on the *last* chain node.
        assert_eq!(c.current, Some(keys[2]));
        let c = f.previous();
        assert_eq!(c.current, Some(keys[1]));
    }

    #[test]
    fn request_explicit_focus() {
        let keys = synth(3);
        let mut f = FocusManager::new();
        f.set_chain(keys.clone());

        let c = f.request(keys[2]);
        assert_eq!(c.previous, None);
        assert_eq!(c.current, Some(keys[2]));

        // Re-request the same node — no-op.
        let c = f.request(keys[2]);
        assert!(c.is_noop());

        // Tab from explicit position → wraps to first.
        let c = f.next();
        assert_eq!(c.previous, Some(keys[2]));
        assert_eq!(c.current, Some(keys[0]));
    }

    #[test]
    fn request_outside_chain_is_rejected() {
        let keys = synth(2);
        let stranger = synth(1)[0];
        let mut f = FocusManager::new();
        f.set_chain(keys.clone());
        f.request(keys[0]);
        let c = f.request(stranger);
        assert_eq!(c.previous, Some(keys[0]));
        assert_eq!(c.current, Some(keys[0]));
        assert_eq!(f.current(), Some(keys[0]));
    }

    #[test]
    fn clear_emits_focus_lost() {
        let keys = synth(2);
        let mut f = FocusManager::new();
        f.set_chain(keys.clone());
        f.next();
        let c = f.clear();
        assert_eq!(c.previous, Some(keys[0]));
        assert_eq!(c.current, None);
        assert!(f.current().is_none());
    }

    #[test]
    fn set_chain_drops_stale_focus() {
        // Use a single SlotMap so the "old" and "new" keys are
        // guaranteed disjoint — `synth(n)` twice would happen to
        // reuse the same first slot on a fresh map and the test
        // would falsely accept a current==Some(reused_key) result.
        let mut sm: SlotMap<NodeKey, u32> = SlotMap::with_key();
        let old_keys: Vec<NodeKey> = (0..3).map(|i| sm.insert(i)).collect();
        let new_keys: Vec<NodeKey> = (0..2).map(|i| sm.insert(100 + i)).collect();
        assert!(old_keys.iter().all(|k| !new_keys.contains(k)));

        let mut f = FocusManager::new();
        f.set_chain(old_keys.clone());
        f.request(old_keys[1]);
        assert_eq!(f.current(), Some(old_keys[1]));
        // Hot-reload: a totally different chain.
        f.set_chain(new_keys.clone());
        assert!(f.current().is_none());
    }

    #[test]
    fn collect_focus_chain_picks_explicit_and_role() {
        use crate::document::loader;
        use jian_ops_schema::document::PenDocument;
        let doc: PenDocument = serde_json::from_str(
            r#"{
              "version":"0.8.0",
              "children":[
                { "type":"frame","id":"root","children":[
                  { "type":"rectangle","id":"plain" },
                  { "type":"rectangle","id":"btn",
                    "semantics":{"role":"button","label":"OK"} },
                  { "type":"rectangle","id":"explicit",
                    "gestures":{"focusable":true} },
                  { "type":"rectangle","id":"opt-out",
                    "semantics":{"role":"input"},
                    "gestures":{"focusable":false} },
                  { "type":"rectangle","id":"link-deep","children":[
                    { "type":"rectangle","id":"inner-link",
                      "semantics":{"role":"link"} }
                  ]}
                ]}
              ]
            }"#,
        )
        .unwrap();
        let state = std::rc::Rc::new(crate::state::StateGraph::new(std::rc::Rc::new(
            crate::signal::scheduler::Scheduler::new(),
        )));
        let rt_doc = loader::build(doc, &state).unwrap();
        let chain = collect_focus_chain(&rt_doc);
        // Pre-order DFS: btn (role=button) → explicit (focusable=true) →
        // inner-link (role=link, nested under a non-focusable rect).
        // `plain` and `opt-out` skipped.
        let ids: Vec<&str> = chain
            .iter()
            .map(|k| crate::document::tree::node_schema_id(&rt_doc.tree.nodes[*k].schema))
            .collect();
        assert_eq!(ids, vec!["btn", "explicit", "inner-link"]);
    }

    #[test]
    fn widget_node_types_are_focusable_by_type() {
        use crate::document::loader;
        use jian_ops_schema::document::PenDocument;
        // No semantics/gestures on any of these — they enter the chain
        // purely by node type. `progress` (display-only) stays out.
        let doc: PenDocument = serde_json::from_str(
            r#"{
              "version":"1.1","formatVersion":"1.1",
              "children":[
                { "type":"frame","id":"root","children":[
                  { "type":"text_input","id":"ti" },
                  { "type":"switch","id":"sw" },
                  { "type":"slider","id":"sl" },
                  { "type":"tabs","id":"tb" },
                  { "type":"progress","id":"pg" }
                ]}
              ]
            }"#,
        )
        .unwrap();
        let state = std::rc::Rc::new(crate::state::StateGraph::new(std::rc::Rc::new(
            crate::signal::scheduler::Scheduler::new(),
        )));
        let rt_doc = loader::build(doc, &state).unwrap();
        let ids: Vec<&str> = collect_focus_chain(&rt_doc)
            .iter()
            .map(|k| crate::document::tree::node_schema_id(&rt_doc.tree.nodes[*k].schema))
            .collect();
        assert_eq!(ids, vec!["ti", "sw", "sl", "tb"]);
    }

    #[test]
    fn tabs_focus_chain_only_walks_the_active_panel() {
        use crate::document::loader;
        use jian_ops_schema::document::PenDocument;
        let doc: PenDocument = serde_json::from_str(
            r#"{
              "version":"1.1","formatVersion":"1.1",
              "children":[
                { "type":"tabs","id":"tabs","value":"second",
                  "tabs":[
                    {"value":"first","label":"First"},
                    {"value":"second","label":"Second"}
                  ],
                  "children":[
                    {"type":"frame","id":"first-panel","children":[
                      {"type":"rectangle","id":"first-action",
                       "gestures":{"focusable":true}}
                    ]},
                    {"type":"frame","id":"second-panel","children":[
                      {"type":"rectangle","id":"second-action",
                       "gestures":{"focusable":true}}
                    ]}
                  ]
                }
              ]
            }"#,
        )
        .unwrap();
        let state = std::rc::Rc::new(crate::state::StateGraph::new(std::rc::Rc::new(
            crate::signal::scheduler::Scheduler::new(),
        )));
        let rt_doc = loader::build(doc, &state).unwrap();
        let ids: Vec<&str> = collect_focus_chain(&rt_doc)
            .iter()
            .map(|key| crate::document::tree::node_schema_id(&rt_doc.tree.nodes[*key].schema))
            .collect();
        assert_eq!(ids, vec!["tabs", "second-action"]);
    }

    #[test]
    fn tabs_focus_chain_falls_back_to_first_and_empty_tabs_are_safe() {
        use crate::document::loader;
        use jian_ops_schema::document::PenDocument;
        let doc: PenDocument = serde_json::from_str(
            r#"{
              "version":"1.1","formatVersion":"1.1",
              "children":[
                { "type":"tabs","id":"fallback","value":"stale",
                  "tabs":[{"value":"first","label":"First"}],
                  "children":[
                    {"type":"rectangle","id":"first-action",
                     "gestures":{"focusable":true}},
                    {"type":"rectangle","id":"orphan-action",
                     "gestures":{"focusable":true}}
                  ]
                },
                { "type":"tabs","id":"empty","tabs":[],"children":[
                    {"type":"rectangle","id":"hidden-action",
                     "gestures":{"focusable":true}}
                  ]
                }
              ]
            }"#,
        )
        .unwrap();
        let state = std::rc::Rc::new(crate::state::StateGraph::new(std::rc::Rc::new(
            crate::signal::scheduler::Scheduler::new(),
        )));
        let rt_doc = loader::build(doc, &state).unwrap();
        let ids: Vec<&str> = collect_focus_chain(&rt_doc)
            .iter()
            .map(|key| crate::document::tree::node_schema_id(&rt_doc.tree.nodes[*key].schema))
            .collect();
        assert_eq!(ids, vec!["fallback", "first-action", "empty"]);
    }

    #[test]
    fn collect_focus_chain_empty_doc() {
        use crate::document::loader;
        use jian_ops_schema::document::PenDocument;
        let doc: PenDocument =
            serde_json::from_str(r#"{"version":"0.8.0","children":[]}"#).unwrap();
        let state = std::rc::Rc::new(crate::state::StateGraph::new(std::rc::Rc::new(
            crate::signal::scheduler::Scheduler::new(),
        )));
        let rt_doc = loader::build(doc, &state).unwrap();
        assert!(collect_focus_chain(&rt_doc).is_empty());
    }
}
