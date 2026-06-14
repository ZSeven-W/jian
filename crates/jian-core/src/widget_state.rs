//! Per-node runtime state for interactive widget nodes, keyed by node
//! id. Lazily created on first access, seeded from the node's authored
//! props; survives `replace_document` for ids that still exist.

use crate::text_input::TextInputState;
use jian_ops_schema::node::{BoolOrExpression, NumberOrExpression, PenNode};
use std::collections::HashMap;

#[derive(Debug)]
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

#[derive(Debug, Default)]
pub struct WidgetStateStore {
    map: HashMap<String, WidgetState>,
}

impl WidgetStateStore {
    /// Seed-from-node on first access. Returns `None` for nodes that
    /// carry no interactive runtime state (e.g. `progress`, which is
    /// display-only and reads its value straight from the state graph).
    pub fn get_or_init(&mut self, node: &PenNode) -> Option<&mut WidgetState> {
        let (id, init) = match node {
            PenNode::TextInput(n) => (
                &n.base.id,
                WidgetState::TextInput(TextInputState::with_text(
                    n.value.clone().unwrap_or_default(),
                )),
            ),
            PenNode::TextArea(n) => (
                &n.base.id,
                WidgetState::TextInput(TextInputState::with_text(
                    n.value.clone().unwrap_or_default(),
                )),
            ),
            PenNode::NumberInput(n) => (
                &n.base.id,
                WidgetState::TextInput(TextInputState::with_text(number_to_text(&n.value))),
            ),
            PenNode::Switch(n) => (
                &n.base.id,
                WidgetState::Toggle {
                    on: bool_default(&n.checked),
                },
            ),
            PenNode::Checkbox(n) => (
                &n.base.id,
                WidgetState::Toggle {
                    on: bool_default(&n.checked),
                },
            ),
            PenNode::Select(n) => (
                &n.base.id,
                WidgetState::Select {
                    open: false,
                    value: n.value.clone(),
                    hover_index: None,
                },
            ),
            PenNode::RadioGroup(n) => (
                &n.base.id,
                WidgetState::Radio {
                    value: n.value.clone(),
                    hover_index: None,
                },
            ),
            PenNode::Slider(n) => (
                &n.base.id,
                WidgetState::Slider {
                    value: number_default(&n.value, n.min.unwrap_or(0.0)),
                    dragging: false,
                },
            ),
            PenNode::Tabs(n) => (
                &n.base.id,
                WidgetState::Tabs {
                    active: n.value.clone(),
                    hover_index: None,
                },
            ),
            _ => return None,
        };
        Some(self.map.entry(id.clone()).or_insert(init))
    }

    pub fn get(&self, id: &str) -> Option<&WidgetState> {
        self.map.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut WidgetState> {
        self.map.get_mut(id)
    }

    /// Drop state for nodes that no longer exist (document swap).
    pub fn retain_ids(&mut self, live: &dyn Fn(&str) -> bool) {
        self.map.retain(|id, _| live(id));
    }

    pub fn clear(&mut self) {
        self.map.clear();
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

    #[test]
    fn text_input_seeds_from_value_and_persists_edits() {
        let mut store = WidgetStateStore::default();
        let n = node(r#"{"type":"text_input","id":"i","value":"hi"}"#);
        match store.get_or_init(&n).unwrap() {
            WidgetState::TextInput(st) => assert_eq!(st.text(), "hi"),
            _ => panic!(),
        }
        // Mutate, then re-init: existing state is kept (not reset).
        if let Some(WidgetState::TextInput(st)) = store.get_mut("i") {
            st.insert_str("!", 0);
        }
        match store.get_or_init(&n).unwrap() {
            WidgetState::TextInput(st) => assert_eq!(st.text(), "hi!"),
            _ => panic!(),
        }
    }

    #[test]
    fn number_input_seeds_text_without_trailing_zero() {
        let mut store = WidgetStateStore::default();
        let n = node(r#"{"type":"number_input","id":"n","value":3}"#);
        match store.get_or_init(&n).unwrap() {
            WidgetState::TextInput(st) => assert_eq!(st.text(), "3"),
            _ => panic!(),
        }
    }

    #[test]
    fn toggle_and_select_and_radio_and_slider_and_tabs_seed() {
        let mut store = WidgetStateStore::default();
        let sw = node(r#"{"type":"switch","id":"s","checked":true}"#);
        assert!(matches!(
            store.get_or_init(&sw),
            Some(WidgetState::Toggle { on: true })
        ));
        let sel = node(r#"{"type":"select","id":"sel","value":"a"}"#);
        assert!(
            matches!(store.get_or_init(&sel), Some(WidgetState::Select { value, .. }) if value.as_deref() == Some("a"))
        );
        let rg = node(r#"{"type":"radio_group","id":"rg","value":"b"}"#);
        assert!(
            matches!(store.get_or_init(&rg), Some(WidgetState::Radio { value, .. }) if value.as_deref() == Some("b"))
        );
        let sl = node(r#"{"type":"slider","id":"sl","min":10,"max":20}"#);
        assert!(
            matches!(store.get_or_init(&sl), Some(WidgetState::Slider { value, .. }) if (*value - 10.0).abs() < f64::EPSILON)
        );
        let tb = node(r#"{"type":"tabs","id":"tb","value":"one"}"#);
        assert!(
            matches!(store.get_or_init(&tb), Some(WidgetState::Tabs { active, .. }) if active.as_deref() == Some("one"))
        );
    }

    #[test]
    fn progress_and_non_widget_have_no_state() {
        let mut store = WidgetStateStore::default();
        let pg = node(r#"{"type":"progress","id":"pg","value":40}"#);
        assert!(store.get_or_init(&pg).is_none());
        let frame = node(r#"{"type":"frame","id":"f"}"#);
        assert!(store.get_or_init(&frame).is_none());
    }

    #[test]
    fn retain_ids_drops_dead_entries() {
        let mut store = WidgetStateStore::default();
        store.get_or_init(&node(r#"{"type":"text_input","id":"keep"}"#));
        store.get_or_init(&node(r#"{"type":"text_input","id":"drop"}"#));
        store.retain_ids(&|id| id == "keep");
        assert!(store.get_mut("keep").is_some());
        assert!(store.get_mut("drop").is_none());
    }
}
