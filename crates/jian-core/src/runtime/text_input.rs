use super::Runtime;
use crate::error::{CoreError, CoreResult};
use crate::geometry::{point, Point, Rect};
use crate::render::{byte_to_utf16_offset, utf16_len, utf16_to_byte_offset};
use crate::text_input::Selection;
use jian_ops_schema::node::PenNode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditableInputKind {
    Text,
    Number,
    Secure,
}

/// Host-facing snapshot of the currently focused editable widget. Text and
/// byte ranges include the active platform preedit.
#[derive(Clone, Debug, PartialEq)]
pub struct EditableTextSnapshot {
    pub page_id: String,
    pub field_id: String,
    pub input_kind: EditableInputKind,
    pub return_key_hint: String,
    pub text: String,
    pub selection: Selection,
    pub composing_range: Option<(usize, usize)>,
    pub bounds: Rect,
    pub text_origin: Point,
    pub max_width: f32,
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: u16,
    pub multiline: bool,
}

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

    /// Snapshot used by native text-input hosts and shaped geometry services.
    pub fn focused_editable_snapshot(&mut self) -> Option<EditableTextSnapshot> {
        let target = self.focus.current()?;
        let (schema, bounds) = {
            let document = self.document.as_ref()?;
            let node = document.tree.nodes.get(target)?;
            (node.schema.clone(), self.node_scene_rect(target)?)
        };
        let (field_id, input_kind, return_key_hint, multiline) = match &schema {
            PenNode::TextInput(node) => (
                node.base.id.clone(),
                if node.secure.unwrap_or(false) {
                    EditableInputKind::Secure
                } else {
                    EditableInputKind::Text
                },
                node.return_key_hint.clone().unwrap_or_default(),
                false,
            ),
            PenNode::TextArea(node) => (
                node.base.id.clone(),
                EditableInputKind::Text,
                node.return_key_hint.clone().unwrap_or_default(),
                true,
            ),
            PenNode::NumberInput(node) => (
                node.base.id.clone(),
                EditableInputKind::Number,
                node.return_key_hint.clone().unwrap_or_default(),
                false,
            ),
            _ => return None,
        };
        let json = serde_json::to_value(&schema).ok()?;
        let font_size = json
            .get("fontSize")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(14.0) as f32;
        let font_family = json
            .get("fontFamily")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let font_weight = json
            .get("fontWeight")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(400) as u16;
        let state = match self.widget_states.get_or_init(&schema, &self.state)? {
            crate::widget_state::WidgetState::TextInput(state) => state,
            _ => return None,
        };
        let text = state.effective_text();
        let selection = state.effective_selection();
        let composing_range = state.effective_composing_range();
        let text_origin = if multiline {
            point(bounds.min_x() + 6.0, bounds.min_y() + 6.0)
        } else {
            point(
                bounds.min_x() + 6.0,
                bounds.min_y() + (bounds.size.height - font_size) * 0.5,
            )
        };
        Some(EditableTextSnapshot {
            page_id: self.active_page_key.clone(),
            field_id,
            input_kind,
            return_key_hint,
            text,
            selection,
            composing_range,
            bounds,
            text_origin,
            max_width: (bounds.size.width - 12.0).max(1.0),
            font_family,
            font_size,
            font_weight,
            multiline,
        })
    }

    pub fn edit_insert(&mut self, text: &str) -> bool {
        let now = self.now_ms;
        let id = self.focused_widget_id();
        let Some(state) = self.focused_text_state() else {
            return false;
        };
        state.commit_text(text, now);
        if let Some(id) = id.as_deref() {
            self.sync_widget_binding(id);
        }
        true
    }

    pub fn edit_replace_range(&mut self, start: usize, end: usize, text: &str) -> bool {
        let now = self.now_ms;
        let id = self.focused_widget_id();
        let Some(state) = self.focused_text_state() else {
            return false;
        };
        state.replace_effective_range(start, end, text, now);
        if let Some(id) = id.as_deref() {
            self.sync_widget_binding(id);
        }
        true
    }

    pub fn edit_set_selection(&mut self, start: usize, end: usize) -> bool {
        let now = self.now_ms;
        let Some(state) = self.focused_text_state() else {
            return false;
        };
        state.set_effective_selection(start, end, now);
        true
    }

    pub fn edit_set_composing_region(&mut self, start: usize, end: usize) -> bool {
        let now = self.now_ms;
        let Some(state) = self.focused_text_state() else {
            return false;
        };
        if state.composition().is_some() {
            state.commit_composition(now);
        }
        state.set_composing_region(start, end);
        true
    }

    pub fn edit_set_composing_text(
        &mut self,
        text: &str,
        selection_start: usize,
        selection_end: usize,
    ) -> bool {
        let now = self.now_ms;
        let Some(state) = self.focused_text_state() else {
            return false;
        };
        state.set_composing_text(text, selection_start, selection_end, now);
        true
    }

    pub fn edit_commit(&mut self, text: &str, new_cursor_position: i32) -> bool {
        let now = self.now_ms;
        let id = self.focused_widget_id();
        let Some(state) = self.focused_text_state() else {
            return false;
        };
        let before = state.effective_text();
        let insertion_byte = state
            .effective_composing_range()
            .map(|range| range.0)
            .unwrap_or_else(|| state.effective_selection().ordered().0);
        let insertion_utf16 = byte_to_utf16_offset(&before, insertion_byte) as i64;
        state.commit_text(text, now);
        let replacement_len = i64::from(utf16_len(text));
        let requested = if new_cursor_position > 0 {
            insertion_utf16 + replacement_len + i64::from(new_cursor_position) - 1
        } else {
            insertion_utf16 + i64::from(new_cursor_position)
        };
        let requested = requested.clamp(0, i64::from(utf16_len(state.text()))) as u32;
        let cursor = utf16_to_byte_offset(state.text(), requested);
        state.set_effective_selection(cursor, cursor, now);
        if let Some(id) = id.as_deref() {
            self.sync_widget_binding(id);
        }
        true
    }

    pub fn edit_cancel(&mut self) -> bool {
        let now = self.now_ms;
        let id = self.focused_widget_id();
        let Some(state) = self.focused_text_state() else {
            return false;
        };
        let _ = state.cancel_composition(now);
        if let Some(id) = id.as_deref() {
            self.sync_widget_binding(id);
        }
        true
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

    pub(super) fn widget_value(&self, node_id: &str) -> Option<serde_json::Value> {
        use crate::widget_state::WidgetState;
        let numeric = self.widget_is_number_input(node_id);
        Some(match self.widget_states.get(node_id) {
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
            None => return None,
        })
    }

    pub(super) fn dispatch_widget_change(&mut self, node_id: &str) {
        let Some(node) = self
            .document
            .as_ref()
            .and_then(|document| document.tree.get(node_id))
        else {
            return;
        };
        let Some(value) = self.widget_value(node_id) else {
            return;
        };
        self.dispatch_semantic_secondary(&crate::gesture::SemanticEvent::Change { node, value });
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
