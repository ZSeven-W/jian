use super::Runtime;
use crate::gesture::{PointerEvent, SemanticEvent, SemanticEventEnvelope};

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
    ///
    /// Source-compatible wrapper over [`Self::dispatch_pointer_events`],
    /// which additionally carries factual pointer/gesture metadata.
    pub fn dispatch_pointer(&mut self, event: PointerEvent) -> Vec<SemanticEvent> {
        self.dispatch_pointer_events(event)
            .into_iter()
            .map(|envelope| envelope.event)
            .collect()
    }

    /// Envelope-returning pointer dispatch: same pipeline and delivery as
    /// [`Self::dispatch_pointer`], but each `SemanticEventEnvelope` keeps
    /// the factual `PointerFacts` captured at recognition time.
    ///
    /// Ordering contract (due Tap precedence):
    /// 1. a due pending Tap is flushed at `event.t_ms` and
    /// 2. its actions are delivered IMMEDIATELY — before any current
    ///    event's slider side effects, `gestures.disabled` predicate
    ///    evaluation, hover semantics or arena routing;
    /// 3. `document` / `input_frozen` are re-checked AFTER the due actions
    ///    (a due action may navigate or park input) — the current event is
    ///    rejected when either holds, but the due delivery itself never
    ///    depends on the freeze (matching `tick`'s frozen flush);
    /// 4. the current event is processed;
    /// 5. due envelopes are returned BEFORE current envelopes.
    ///
    /// The `gestures.disabled` predicate is state-aware here: the runtime
    /// pointer path supplies it to the router so dynamically disabled
    /// handlers participate in arbitration/config decisions (DoubleTap
    /// deferral, owner detection, Pan/LongPress/ContextMenu thresholds)
    /// and in delivery (handler skip, built-in activation, `$self` scope).
    pub fn dispatch_pointer_events(&mut self, event: PointerEvent) -> Vec<SemanticEventEnvelope> {
        self.note_time(event.t_ms);
        // (1)+(2) Flush a due pending Tap at this event's timestamp and
        // deliver its actions before ANY current side effect. The deadline
        // is order-independent: whether the host calls `tick(deadline)`
        // first or dispatches the next input at the deadline first, the
        // deferred Tap surfaces as a single Tap before the current
        // processing observes anything.
        let mut due = self.gestures.flush_pending_tap(event.t_ms);
        for ev in &due {
            self.deliver_enveloped(ev);
        }
        // (3) Re-check after the due actions: the current event is gated
        // by the post-due state, not the entry state, while the already-
        // delivered due envelopes are never dropped.
        if self.input_frozen() || self.document.is_none() {
            return due;
        }
        // (4) Current event. Slider drag is handled directly off the raw
        // pointer phases (the gesture arena only surfaces Tap on Down+Up):
        // Down over a slider arms a drag, Move scrubs the value,
        // Up/Cancel disarms it. This runs *before* the arena dispatch so
        // a drag and a tap don't double-set the value — a clean Down+Up
        // still lands as a Tap.
        self.handle_slider_drag(&event);

        let emitted = {
            let doc = self.document.as_ref().unwrap();
            let state_ref = &self.state;
            let expr_cache_ref = &self.expr_cache;
            let page_id = self.active_page_key.clone();
            let node_disabled = |key: crate::document::NodeKey| {
                super::async_runtime::node_gestures_disabled(
                    doc,
                    state_ref,
                    expr_cache_ref,
                    &page_id,
                    key,
                )
            };
            // Internal current-event path: the router does NOT flush a due
            // pending Tap here — we just flushed and delivered it above —
            // so it can never be collected twice.
            self.gestures
                .dispatch_current(event, doc, &self.spatial, &node_disabled)
        };
        // ONE semantic-delivery path (widget activation included) runs for
        // both pointer dispatch and `tick`; activation is inside it.
        for ev in &emitted {
            self.deliver_enveloped(ev);
        }
        // (5) Due envelopes first, then current envelopes.
        due.extend(emitted);
        due
    }

    /// Pointer-phase driven slider scrubbing. On `Down` over a slider,
    /// focus it and arm the drag (`Slider.dragging = true`). On `Move`
    /// while any slider is armed, set that slider's value from x and
    /// sync its `bind:value`. On `Up`/`Cancel`, disarm every slider.
    /// No-op when no slider is under the cursor / armed.
    ///
    /// Raw drag arming requires a provable primary interaction: Touch
    /// contact or a Down whose button bitmask is EXACTLY LEFT. A factual
    /// right-button (or ambiguous multi-button) Down must never focus,
    /// arm or change a Slider — the router treats right-only presses as
    /// closed sequences, and the drag path must not re-open them.
    ///
    /// A disabled Slider is inert: the drag path honors the same gate as
    /// the widget-activation path — static `gestures.disabledEvents`
    /// listing `onTap`, or a truthy `gestures.disabled` expression
    /// (malformed/non-bool stays fail-open). A disabled Down must not
    /// focus, arm, mutate or sync the slider; a Move that finds the
    /// armed slider disabled since its Down disarms it immediately and
    /// does not mutate or sync.
    fn handle_slider_drag(&mut self, event: &crate::gesture::pointer::PointerEvent) {
        use crate::gesture::pointer::{MouseButtons, PointerKind, PointerPhase};
        use crate::widget_state::WidgetState;
        use jian_ops_schema::node::PenNode;

        match event.phase {
            PointerPhase::Down => {
                let provable_primary =
                    matches!(event.kind, PointerKind::Touch) || event.buttons == MouseButtons::LEFT;
                if !provable_primary {
                    return;
                }
                // Topmost hit node that is a slider arms a drag.
                let Some(doc) = self.document.as_ref() else {
                    return;
                };
                let hit = crate::gesture::hit::hit_test(&self.spatial, doc, event.position);
                let slider = hit.0.iter().copied().find(|&k| {
                    matches!(
                        doc.tree.nodes.get(k).map(|n| &n.schema),
                        Some(PenNode::Slider(_))
                    )
                });
                if let Some(node) = slider {
                    // Evaluate the inert gate while every borrow is
                    // immutable; the check ends before any `&mut self`
                    // side effect below (focus, arm, scrub, sync).
                    let inert = {
                        let state = &self.state;
                        let expr_cache = &self.expr_cache;
                        let page_id = &self.active_page_key;
                        slider_drag_inert(doc, state, expr_cache, page_id, node)
                    };
                    if !inert {
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
                        if self.set_slider_from_x(node, event.position.x) {
                            self.sync_widget_binding(&id);
                        }
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
                let Some(node) = self.document.as_ref().and_then(|doc| doc.tree.get(&id)) else {
                    return;
                };
                // If the armed slider became disabled since its Down,
                // disarm it immediately and never scrub/sync it.
                let inert = {
                    let doc = self.document.as_ref().unwrap();
                    let state = &self.state;
                    let expr_cache = &self.expr_cache;
                    let page_id = &self.active_page_key;
                    slider_drag_inert(doc, state, expr_cache, page_id, node)
                };
                if inert {
                    self.with_widget_state(node, |st| {
                        if let WidgetState::Slider { dragging, .. } = st {
                            *dragging = false;
                        }
                        false
                    });
                    return;
                }
                if self.set_slider_from_x(node, event.position.x) {
                    self.sync_widget_binding(&id);
                }
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                // Disarm any armed slider exactly like an Up; a later Move
                // must not scrub a canceled pointer's drag.
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
    ///
    /// Lives on the single semantic-delivery path used by BOTH pointer
    /// dispatch and `tick`, so a deferred (double-tap-window) Tap still
    /// activates its widget when the deadline flushes it.
    pub(super) fn activate_widget_on_tap(
        &mut self,
        node: crate::document::NodeKey,
        position: crate::geometry::Point,
    ) {
        use crate::widget_state::WidgetState;
        use jian_ops_schema::node::PenNode;

        #[derive(Clone, Copy)]
        enum Act {
            Toggle,
            Slider,
            Tabs,
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
                PenNode::Tabs(_) => Act::Tabs,
                PenNode::TextInput(_)
                | PenNode::TextArea(_)
                | PenNode::NumberInput(_)
                | PenNode::Select(_)
                | PenNode::RadioGroup(_) => Act::FocusOnly,
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
            Act::Tabs => self.set_tabs_from_point(node, position),
            Act::FocusOnly | Act::NotWidget => false,
        };
        if changed {
            self.sync_widget_binding(&id);
            if matches!(act, Act::Tabs) {
                // Panels share the same laid-out grid cell, so switching does
                // not require layout. It does require hit and focus indexes to
                // drop the old subtree before the next input event.
                self.rebuild_spatial();
            }
        }
    }

    /// Activate the equal-width tab cell under `position` when it lies in the
    /// intrinsic 32px tab bar. The panel area below the bar is intentionally
    /// excluded so interacting with panel content cannot switch tabs.
    fn set_tabs_from_point(
        &mut self,
        node: crate::document::NodeKey,
        position: crate::geometry::Point,
    ) -> bool {
        use crate::widget_state::WidgetState;
        use jian_ops_schema::node::PenNode;

        let Some(doc) = self.document.as_ref() else {
            return false;
        };
        let Some(nd) = doc.tree.nodes.get(node) else {
            return false;
        };
        let PenNode::Tabs(tabs) = &nd.schema else {
            return false;
        };
        let values: Vec<String> = tabs
            .tabs
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|tab| tab.value.clone())
            .collect();
        if values.is_empty() {
            return false;
        }
        let Some(rect) = self.node_scene_rect(node) else {
            return false;
        };
        let bar_height = crate::layout::resolve::TABS_BAR_HEIGHT.min(rect.size.height);
        let in_bar = rect.contains(position) && position.y < rect.min_y() + bar_height;
        if !in_bar || rect.size.width <= 0.0 {
            return false;
        }
        let cell_width = rect.size.width / values.len() as f32;
        let index =
            (((position.x - rect.min_x()) / cell_width).floor() as usize).min(values.len() - 1);
        let next = values[index].as_str();
        self.with_widget_state(node, |state| {
            let WidgetState::Tabs { active, .. } = state else {
                return false;
            };
            // Resolve through the shared contract before comparing the raw
            // value. A stale/missing value paints/indexes panel zero, but a
            // click on that cell still normalizes and persists its real value.
            let _ = crate::widget_state::resolve_tab_index(
                values.iter().map(|value| Some(value.as_str())),
                active.as_deref(),
            );
            if active.as_deref() == Some(next) {
                false
            } else {
                *active = Some(next.to_owned());
                true
            }
        })
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

/// Raw slider-drag inert-ness: the slider is gated by the SAME test the
/// widget-activation path uses — statically slated (`gestures.disabledEvents`
/// lists `onTap`) or a truthy `gestures.disabled` expression. A malformed /
/// non-bool `disabled` expression disables nothing (`node_gestures_disabled`
/// is fail-open), consistent with bindings.
fn slider_drag_inert(
    doc: &crate::document::RuntimeDocument,
    state: &crate::state::StateGraph,
    expr_cache: &crate::expression::ExpressionCache,
    page_id: &str,
    key: crate::document::NodeKey,
) -> bool {
    crate::gesture::config::node_disables_handler(doc, key, "onTap")
        || super::async_runtime::node_gestures_disabled(doc, state, expr_cache, page_id, key)
}
