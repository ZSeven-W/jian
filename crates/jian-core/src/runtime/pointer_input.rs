use super::Runtime;
use crate::gesture::{PointerEvent, SemanticEvent};

impl Runtime {
    /// Route a wheel event to the topmost hit node carrying an onScroll handler.
    pub fn dispatch_wheel(
        &mut self,
        event: crate::gesture::pointer::WheelEvent,
    ) -> Vec<SemanticEvent> {
        self.note_time(event.t_ms);
        if self.input_frozen() {
            return Vec::new();
        }
        let Some(doc) = self.document.as_ref() else {
            return Vec::new();
        };
        let mut emitted = Vec::new();
        let path = crate::gesture::hit::hit_test(&self.spatial, doc, event.position);
        for key in path.0.iter().copied() {
            let schema = &doc.tree.nodes[key].schema;
            if json_has_event_handler(schema, "onScroll") {
                emitted.push(SemanticEvent::Scroll {
                    node: key,
                    delta: event.delta,
                });
                break;
            }
        }
        for event in &emitted {
            self.dispatch_semantic(event);
        }
        emitted
    }
}

impl Runtime {
    /// Feed a pointer event through the gesture pipeline; any emitted
    /// semantic events are routed to the matching `events.*` handlers.
    /// Returns the semantic events for host inspection/tests.
    pub fn dispatch_pointer(&mut self, event: PointerEvent) -> Vec<SemanticEvent> {
        self.note_time(event.t_ms);
        if self.input_frozen() {
            return Vec::new();
        }
        if self.document.is_none() {
            return Vec::new();
        }
        // Slider drag is handled directly off the raw pointer phases
        // (the gesture arena only surfaces Tap on Down+Up): Down over a
        // slider arms a drag, Move scrubs the value, Up disarms it. This
        // runs *before* the arena dispatch so a drag and a tap don't
        // double-set the value — a clean Down+Up still lands as a Tap.
        let (phase, position) = (event.phase, event.position);
        self.handle_slider_drag(phase, position);

        let emitted = {
            let doc = self.document.as_ref().unwrap();
            self.gestures.dispatch(event, doc, &self.spatial)
        };
        // A tap on an interactive widget focuses it and performs its
        // primary action (toggle / slider set-by-x) before the generic
        // onTap action dispatch.
        for ev in &emitted {
            if let SemanticEvent::Tap { node, position } = ev {
                self.activate_widget_on_tap(*node, *position);
            }
        }
        for ev in &emitted {
            self.dispatch_semantic(ev);
        }
        emitted
    }

    /// Pointer-phase driven slider scrubbing. On `Down` over a slider,
    /// focus it and arm the drag (`Slider.dragging = true`). On `Move`
    /// while any slider is armed, set that slider's value from x and
    /// sync its `bind:value`. On `Up`, disarm every slider. No-op when
    /// no slider is under the cursor / armed.
    fn handle_slider_drag(
        &mut self,
        phase: crate::gesture::pointer::PointerPhase,
        position: crate::geometry::Point,
    ) {
        use crate::gesture::pointer::PointerPhase;
        use crate::widget_state::WidgetState;
        use jian_ops_schema::node::PenNode;

        match phase {
            PointerPhase::Down => {
                // Topmost hit node that is a slider arms a drag.
                let Some(doc) = self.document.as_ref() else {
                    return;
                };
                let hit = crate::gesture::hit::hit_test(&self.spatial, doc, position);
                let slider = hit.0.iter().copied().find(|&k| {
                    matches!(
                        doc.tree.nodes.get(k).map(|n| &n.schema),
                        Some(PenNode::Slider(_))
                    )
                });
                if let Some(node) = slider {
                    let id = {
                        let schema = &doc.tree.nodes[node].schema;
                        crate::document::tree::node_schema_id(schema).to_owned()
                    };
                    let _ = self.focus_request(node);
                    self.with_widget_state(node, |st| {
                        if let WidgetState::Slider { dragging, .. } = st {
                            *dragging = true;
                        }
                        false
                    });
                    if self.set_slider_from_x(node, position.x) {
                        self.sync_widget_binding(&id);
                    }
                }
            }
            PointerPhase::Move => {
                // Find the id of whichever slider is currently armed, then
                // resolve its node key. Two steps so the widget-state read
                // and the document read don't overlap-borrow `self`.
                let armed_id = self.widget_states.iter().find_map(|(id, st)| {
                    matches!(st, WidgetState::Slider { dragging: true, .. }).then(|| id.to_owned())
                });
                let Some(id) = armed_id else { return };
                let node = self.document.as_ref().and_then(|doc| doc.tree.get(&id));
                if let Some(node) = node {
                    if self.set_slider_from_x(node, position.x) {
                        self.sync_widget_binding(&id);
                    }
                }
            }
            PointerPhase::Up => {
                // Disarm any armed slider.
                for st in self.widget_states.values_mut() {
                    if let WidgetState::Slider { dragging, .. } = st {
                        *dragging = false;
                    }
                }
            }
            _ => {}
        }
    }

    /// Tap-driven widget activation: focus the tapped widget and, for a
    /// switch/checkbox, flip it; for a slider, set its value from the
    /// tap x within the track. Syncs `bind:value` afterwards. Other
    /// widgets just take focus (text editing / popups come via keys).
    fn activate_widget_on_tap(
        &mut self,
        node: crate::document::NodeKey,
        position: crate::geometry::Point,
    ) {
        use crate::widget_state::WidgetState;
        use jian_ops_schema::node::PenNode;

        enum Act {
            Toggle,
            Slider,
            FocusOnly,
            NotWidget,
        }

        let (id, act) = {
            let Some(doc) = self.document.as_ref() else {
                return;
            };
            let Some(nd) = doc.tree.nodes.get(node) else {
                return;
            };
            let schema = &nd.schema;
            let id = crate::document::tree::node_schema_id(schema).to_owned();
            let act = match schema {
                PenNode::Switch(_) | PenNode::Checkbox(_) => Act::Toggle,
                PenNode::Slider(_) => Act::Slider,
                PenNode::TextInput(_)
                | PenNode::TextArea(_)
                | PenNode::NumberInput(_)
                | PenNode::Select(_)
                | PenNode::RadioGroup(_)
                | PenNode::Tabs(_) => Act::FocusOnly,
                _ => Act::NotWidget,
            };
            (id, act)
        };

        if matches!(act, Act::NotWidget) {
            return;
        }
        let _ = self.focus_request(node);

        let changed = match act {
            Act::Toggle => self.with_widget_state(node, |st| {
                if let WidgetState::Toggle { on } = st {
                    *on = !*on;
                    true
                } else {
                    false
                }
            }),
            Act::Slider => self.set_slider_from_x(node, position.x),
            Act::FocusOnly | Act::NotWidget => false,
        };
        if changed {
            self.sync_widget_binding(&id);
        }
    }

    /// Set a slider's value from a pointer x within its track, using the
    /// node's `min`/`max`/`step` and the same quantization the tap path
    /// uses. Returns `true` when the slider's runtime value changed.
    /// Shared by both the tap path and the drag path. Does NOT sync the
    /// `bind:value` target — the caller does that (so a drag can sync
    /// once per move without re-resolving the id here).
    fn set_slider_from_x(&mut self, node: crate::document::NodeKey, x: f32) -> bool {
        use crate::widget_state::WidgetState;
        use jian_ops_schema::node::PenNode;

        let Some(doc) = self.document.as_ref() else {
            return false;
        };
        let Some(nd) = doc.tree.nodes.get(node) else {
            return false;
        };
        let PenNode::Slider(s) = &nd.schema else {
            return false;
        };
        let (min, max, step) = (
            s.min.unwrap_or(0.0),
            s.max.unwrap_or(100.0),
            s.step.unwrap_or(1.0),
        );
        let Some(r) = self.node_scene_rect(node) else {
            return false;
        };
        let (min_x, width) = (r.min_x(), r.size.width);
        let frac = if width > 0.0 {
            (((x - min_x) / width) as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let raw = min + frac * (max - min);
        let quantized = if step > 0.0 {
            min + ((raw - min) / step).round() * step
        } else {
            raw
        };
        let v = quantized.clamp(min, max);
        self.with_widget_state(node, |st| {
            if let WidgetState::Slider { value, .. } = st {
                if (*value - v).abs() > f64::EPSILON {
                    *value = v;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        })
    }

    /// Run `f` against the lazily-seeded widget state for `node`.
    /// Returns `f`'s result, or `false` when the node has no state.
    pub(super) fn with_widget_state(
        &mut self,
        node: crate::document::NodeKey,
        f: impl FnOnce(&mut crate::widget_state::WidgetState) -> bool,
    ) -> bool {
        let Some(doc) = self.document.as_ref() else {
            return false;
        };
        let Some(nd) = doc.tree.nodes.get(node) else {
            return false;
        };
        match self.widget_states.get_or_init(&nd.schema, &self.state) {
            Some(st) => f(st),
            None => false,
        }
    }
}

fn json_has_event_handler(node: &jian_ops_schema::node::PenNode, key: &str) -> bool {
    use serde_json::Value;
    let value = match serde_json::to_value(node) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let handler = value
        .as_object()
        .and_then(|object| object.get("events"))
        .and_then(|events| events.as_object())
        .and_then(|events| events.get(key));
    match handler {
        Some(Value::Array(actions)) => !actions.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}
