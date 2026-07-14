use super::Runtime;
use crate::error::{CoreError, CoreResult};

impl Runtime {
    /// `&mut TextInputState` for the currently-focused editable widget
    /// (text_input / text_area / number_input), or `None` when nothing
    /// editable is focused.
    fn focused_text_state(&mut self) -> Option<&mut crate::text_input::TextInputState> {
        let target = self.focus.current()?;
        let node = self.document.as_ref()?.tree.nodes.get(target)?;
        match self.widget_states.get_or_init(&node.schema, &self.state)? {
            crate::widget_state::WidgetState::TextInput(st) => Some(st),
            _ => None,
        }
    }

    /// Printable text from the host (keypress chars, paste). Routed to
    /// the focused editable widget; returns `true` when consumed.
    pub fn dispatch_text_input(&mut self, text: &str) -> CoreResult<bool> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        if text.is_empty() {
            return Ok(false);
        }
        let now = self.now_ms;
        let id = self.focused_widget_id();
        {
            let Some(st) = self.focused_text_state() else {
                return Ok(false);
            };
            st.insert_str(text, now);
        }
        if let Some(id) = id.as_deref() {
            self.sync_widget_binding(id);
        }
        Ok(true)
    }

    /// IME composition events (`gesture::ime::ImeEvent`). Routed to the
    /// focused editable widget; returns `true` when consumed.
    pub fn dispatch_ime(&mut self, ev: crate::gesture::ime::ImeEvent) -> CoreResult<bool> {
        if self.input_frozen() {
            return Err(CoreError::Busy);
        }
        use crate::gesture::ime::ImeKind;
        let now = self.now_ms;
        let id = self.focused_widget_id();
        let committed;
        {
            let Some(st) = self.focused_text_state() else {
                return Ok(false);
            };
            match ev.kind {
                ImeKind::CompositionStart => {
                    st.set_composition(ev.text, 0, now);
                    committed = false;
                }
                ImeKind::CompositionUpdate { selection } => {
                    let cursor = selection.map(|r| r.end).unwrap_or(ev.text.len());
                    st.set_composition(ev.text, cursor, now);
                    committed = false;
                }
                ImeKind::CompositionEnd => {
                    let len = ev.text.len();
                    st.set_composition(ev.text, len, now);
                    st.commit_composition(now);
                    committed = true;
                }
            }
        }
        if committed {
            if let Some(id) = id.as_deref() {
                self.sync_widget_binding(id);
            }
        }
        Ok(true)
    }

    /// Push the focused/edited widget's current value into the state graph via
    /// its `bindings["bind:value"]` target.
    pub(super) fn sync_widget_binding(&mut self, node_id: &str) {
        use crate::widget_state::WidgetState;
        let numeric = self.widget_is_number_input(node_id);
        let value = match self.widget_states.get_mut(node_id) {
            Some(WidgetState::TextInput(st)) if numeric => st
                .text()
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Some(WidgetState::TextInput(st)) => serde_json::Value::String(st.text().to_owned()),
            Some(WidgetState::Toggle { on }) => serde_json::Value::Bool(*on),
            Some(WidgetState::Slider { value, .. }) => serde_json::json!(*value),
            Some(WidgetState::Select { value, .. }) | Some(WidgetState::Radio { value, .. }) => {
                value
                    .clone()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null)
            }
            Some(WidgetState::Tabs { active, .. }) => active
                .clone()
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
            None => return,
        };
        let Some(key) = self.widget_bind_key(node_id) else {
            return;
        };
        self.state.app_set(&key, value);
        self.scheduler.flush();
    }

    fn widget_bind_key(&self, node_id: &str) -> Option<String> {
        let key = self.document.as_ref()?.tree.get(node_id)?;
        let node = self.document.as_ref()?.tree.nodes.get(key)?;
        let json = serde_json::to_value(&node.schema).ok()?;
        let raw = json.get("bindings")?.get("bind:value")?.as_str()?;
        let rest = raw.trim().strip_prefix("$state.")?;
        if rest.is_empty() || rest.contains(['.', '[']) || rest.contains(char::is_whitespace) {
            return None;
        }
        Some(rest.to_owned())
    }

    fn widget_is_number_input(&self, node_id: &str) -> bool {
        self.document
            .as_ref()
            .and_then(|d| d.tree.get(node_id).map(|k| (d, k)))
            .and_then(|(d, k)| d.tree.nodes.get(k))
            .map(|n| matches!(n.schema, jian_ops_schema::node::PenNode::NumberInput(_)))
            .unwrap_or(false)
    }
}
