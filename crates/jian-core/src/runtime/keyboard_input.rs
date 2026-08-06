use super::Runtime;
use crate::gesture::SemanticEvent;

impl Runtime {
    /// Dispatch a key event to the named node and route the semantic event.
    pub fn dispatch_key(
        &mut self,
        target: crate::document::NodeKey,
        key: impl Into<String>,
        modifiers: crate::gesture::pointer::Modifiers,
    ) -> Vec<SemanticEvent> {
        if self.input_frozen() || self.document.is_none() {
            return Vec::new();
        }
        let event = SemanticEvent::KeyDown {
            node: target,
            key: key.into(),
            modifiers,
        };
        self.dispatch_semantic(&event);
        vec![event]
    }

    /// High-level focus-aware keyboard entry point. Tab changes focus; editing
    /// keys update focused widgets; remaining keys reach authored handlers.
    pub fn dispatch_keyboard(
        &mut self,
        key: impl Into<String>,
        modifiers: crate::gesture::pointer::Modifiers,
    ) -> Vec<SemanticEvent> {
        if self.input_frozen() || self.document.is_none() {
            return Vec::new();
        }
        let key = key.into();
        if key == "Tab" {
            if modifiers.contains(crate::gesture::pointer::Modifiers::SHIFT) {
                return self.focus_previous().unwrap_or_default();
            }
            return self.focus_next().unwrap_or_default();
        }
        let now = self.now_ms;
        let focused_id = self.focused_widget_id();
        let focused_tabs = focused_id.as_deref().is_some_and(|id| {
            self.document
                .as_ref()
                .and_then(|doc| doc.tree.get(id).and_then(|key| doc.tree.nodes.get(key)))
                .is_some_and(|node| matches!(&node.schema, jian_ops_schema::node::PenNode::Tabs(_)))
        });
        let mut consumed = false;
        if let Some(st) = self.focused_text_state_for_keyboard() {
            use crate::gesture::pointer::Modifiers;
            consumed = match key.as_str() {
                "Backspace" => {
                    st.backspace(now);
                    true
                }
                "Delete" => {
                    st.delete_forward(now);
                    true
                }
                "ArrowLeft" => {
                    st.move_left(modifiers.contains(Modifiers::SHIFT), now);
                    true
                }
                "ArrowRight" => {
                    st.move_right(modifiers.contains(Modifiers::SHIFT), now);
                    true
                }
                "Home" => {
                    st.move_home(modifiers.contains(Modifiers::SHIFT), now);
                    true
                }
                "End" => {
                    st.move_end(modifiers.contains(Modifiers::SHIFT), now);
                    true
                }
                "a" | "A"
                    if modifiers.contains(Modifiers::CMD)
                        || modifiers.contains(Modifiers::CTRL) =>
                {
                    st.select_all();
                    true
                }
                _ => false,
            };
        }
        if !consumed {
            if let Some(id) = focused_id.as_deref() {
                consumed = self.route_widget_action_key(id, key.as_str());
            }
        }
        if consumed {
            if let Some(id) = focused_id.as_deref() {
                self.sync_widget_binding(id);
            }
            if focused_tabs {
                // Keyboard tab changes must retire the old panel from pointer
                // and focus indexes in the same turn, just like bar clicks.
                self.rebuild_spatial();
            }
            return Vec::new();
        }
        let Some(target) = self.focus.current() else {
            return Vec::new();
        };
        self.dispatch_key(target, key, modifiers)
    }

    fn focused_text_state_for_keyboard(
        &mut self,
    ) -> Option<&mut crate::text_input::TextInputState> {
        let target = self.focus.current()?;
        let node = self.document.as_ref()?.tree.nodes.get(target)?;
        match self.widget_states.get_or_init(&node.schema, &self.state)? {
            crate::widget_state::WidgetState::TextInput(st) => Some(st),
            _ => None,
        }
    }

    fn route_widget_action_key(&mut self, id: &str, key: &str) -> bool {
        use crate::widget_state::WidgetState;
        let (min, max, step) = self.slider_bounds(id);
        let options = self.widget_option_values(id);
        match self.widget_states.get_mut(id) {
            Some(WidgetState::Toggle { on }) => match key {
                "Enter" | " " | "Space" | "Spacebar" => {
                    *on = !*on;
                    true
                }
                _ => false,
            },
            Some(WidgetState::Slider { value, .. }) => {
                let new = match key {
                    "ArrowRight" | "ArrowUp" => (*value + step).min(max),
                    "ArrowLeft" | "ArrowDown" => (*value - step).max(min),
                    "Home" => min,
                    "End" => max,
                    _ => return false,
                };
                *value = new;
                true
            }
            Some(WidgetState::Select { value, .. }) | Some(WidgetState::Radio { value, .. }) => {
                match step_option(&options, value.as_deref(), key) {
                    Some(next) => {
                        *value = Some(next);
                        true
                    }
                    None => false,
                }
            }
            Some(WidgetState::Tabs { active, .. }) => {
                match step_option(&options, active.as_deref(), key) {
                    Some(next) => {
                        *active = Some(next);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    fn widget_option_values(&self, id: &str) -> Vec<String> {
        let Some(node) = self
            .document
            .as_ref()
            .and_then(|d| d.tree.get(id).and_then(|k| d.tree.nodes.get(k)))
        else {
            return Vec::new();
        };
        let Ok(json) = serde_json::to_value(&node.schema) else {
            return Vec::new();
        };
        json.get("options")
            .or_else(|| json.get("tabs"))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|o| o.get("value").and_then(|v| v.as_str()).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn slider_bounds(&self, id: &str) -> (f64, f64, f64) {
        use jian_ops_schema::node::PenNode;
        let node = self
            .document
            .as_ref()
            .and_then(|d| d.tree.get(id).map(|k| (d, k)))
            .and_then(|(d, k)| d.tree.nodes.get(k));
        if let Some(PenNode::Slider(s)) = node.map(|n| &n.schema) {
            (
                s.min.unwrap_or(0.0),
                s.max.unwrap_or(100.0),
                s.step.unwrap_or(1.0),
            )
        } else {
            (0.0, 100.0, 1.0)
        }
    }
}

fn step_option(options: &[String], current: Option<&str>, key: &str) -> Option<String> {
    if options.is_empty() {
        return None;
    }
    let delta: i32 = match key {
        "ArrowDown" | "ArrowRight" => 1,
        "ArrowUp" | "ArrowLeft" => -1,
        _ => return None,
    };
    let next = match current.and_then(|c| options.iter().position(|o| o == c)) {
        Some(i) => (i as i32 + delta).rem_euclid(options.len() as i32) as usize,
        None if delta > 0 => 0,
        None => options.len() - 1,
    };
    Some(options[next].clone())
}
