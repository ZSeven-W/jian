use super::{ReportedActionOutcome, Runtime};
use crate::action::{ActionContext, CancellationToken, ExecOutcome};
use crate::binding::BindingEffect;
use crate::expression::Expression;
use crate::geometry::Point;
use crate::gesture::config;
use crate::gesture::dispatcher;
use crate::gesture::{SemanticEvent, SemanticEventEnvelope};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

impl Runtime {
    /// Drive timer-based recognizers such as LongPress and flush a due
    /// deferred Tap at its double-tap deadline.
    pub fn tick(&mut self, now_ms: u64) -> Vec<SemanticEvent> {
        self.note_time(now_ms);
        // Arena timers advance only when input is NOT frozen: a LongPress
        // deadline crossing a parked variant swap does not claim inside the
        // freeze (existing behavior). A deferred Tap whose deadline passed
        // is flushed regardless — it was derived from already-accepted
        // input, and consuming it without delivery would silently lose it.
        // `dispatch_pointer` rejects NEW input; this only preserves timer
        // output.
        let emitted = if self.input_frozen() {
            self.gestures.flush_pending_tap(self.now_ms)
        } else {
            self.gestures.tick_enveloped(self.now_ms)
        };
        for event in &emitted {
            // Expired timers replay OLD input; certifying them with the
            // pending id would spend the user's current intent on it.
            self.deliver_enveloped(event, false);
        }
        emitted.into_iter().map(|e| e.event).collect()
    }

    /// Deliver a plain, non-envelope semantic event (key/scroll/focus)
    /// through the same delivery path.
    pub(super) fn dispatch_semantic(&mut self, event: &SemanticEvent) {
        self.deliver_enveloped(&SemanticEventEnvelope::plain(event.clone()), true);
    }

    /// The ONE semantic-delivery path, shared by pointer dispatch and tick:
    /// 1) built-in widget activation for Taps (Switch/Checkbox/Tabs/Slider/
    ///    Input focus) — skipped for a disabled target widget, 2) handler
    ///    resolution with `disabledEvents` and `gestures.disabled`
    ///    skipping, 3) the single `$event` payload construction, 4)
    ///    ActionList execution. Handler resolution, the node-local
    ///    coordinate origin AND the `ActionContext.node_id` all use the
    ///    SAME resolved handler owner: `$self` and `$event.local` are
    ///    relative to the owner, never the hit child.
    pub(super) fn deliver_enveloped(
        &mut self,
        envelope: &SemanticEventEnvelope,
        may_consume_activation: bool,
    ) {
        let event = &envelope.event;
        // A (possibly deferred) Tap must still perform built-in widget
        // activation before the authored onTap actions run — unless the
        // target widget is dynamically disabled (`gestures.disabled`
        // truthy) or statically slated (`disabledEvents` lists onTap):
        // a disabled widget is inert, not just handler-less.
        let tap_activation = match event {
            SemanticEvent::Tap { node, position } => {
                let document = self.document.as_ref().expect("no document loaded");
                let target_disabled = config::node_disables_handler(document, *node, "onTap")
                    || node_gestures_disabled(
                        document,
                        &self.state,
                        &self.expr_cache,
                        &self.active_page_key,
                        *node,
                    );
                (!target_disabled).then_some((*node, *position))
            }
            _ => None,
        };
        if let Some((node, position)) = tap_activation {
            self.activate_widget_on_tap(node, position);
        }

        // Resolve the handler owner (bubbling, disabled-aware) and compute
        // the node-local coordinate origin + `$self` scope from the
        // handler's layout rect / node id. The payload path itself stays
        // one function: `envelope.payload`.
        let (source_node_id, payload) = {
            let document = self.document.as_ref().expect("no document loaded");
            let state_ref = &self.state;
            let expr_cache_ref = &self.expr_cache;
            let page_id = self.active_page_key.clone();
            let node_disabled = |key: crate::document::NodeKey| {
                node_gestures_disabled(document, state_ref, expr_cache_ref, &page_id, key)
            };
            // A claimed Swipe is owner-anchored: `resolve_swipe_owner`
            // never bubbles to an ancestor, because the thresholds that
            // qualified the claim belonged to the captured owner — and a
            // same-batch PressCancel action may have just disabled it
            // (see also the batch re-validation in
            // `dispatch_pointer_events`). Every other semantic keeps the
            // general bubbled resolution.
            let resolved = match event {
                SemanticEvent::Swipe { .. } => {
                    dispatcher::resolve_swipe_owner(document, event, node_disabled)
                }
                _ => dispatcher::resolve_handler(document, event, node_disabled),
            };
            let handler_owner = resolved.as_ref().map(|(owner, _)| *owner);
            // The ActionContext node id follows the resolved handler
            // owner (bubbling target), NOT the hit node — `$self` writes
            // land on the owner. With no handler, fall back to the
            // event's target, which is what the host sees in the payload.
            let scope_node = handler_owner.unwrap_or(event.node());
            let source_node_id = document
                .tree
                .nodes
                .get(scope_node)
                .map(|node| crate::document::tree::node_schema_id(&node.schema).to_owned());
            let local_origin: Option<Point> = handler_owner
                .and_then(|owner| self.node_scene_rect(owner))
                .map(|rect| crate::geometry::point(rect.min_x(), rect.min_y()));
            let payload = envelope.payload(local_origin);
            (
                source_node_id,
                (resolved.as_ref().map(|(_, list)| list.clone()), payload),
            )
        };
        let (handler_list, payload) = payload;

        let mut context = self.make_action_ctx();
        // The id certifies the input being dispatched NOW, and it is
        // spent by the FIRST chain that actually runs on that input's
        // delivery path — a Down whose PressStart resolves no handler
        // must not burn the id the Up's Tap was certified for.
        if may_consume_activation && handler_list.is_some() {
            context.activation = self.take_activation();
        }
        if let Some(payload) = payload {
            context.event = Some(crate::value::RuntimeValue::from(payload));
        }
        context.node_id = source_node_id;
        if let Some(list) = handler_list {
            match self.task_queue.spawn(
                &self.actions,
                &list,
                context,
                self.document_generation,
                Some(event.handler_key().to_owned()),
            ) {
                Ok(_) => {
                    self.collect_task_outcomes();
                }
                Err(error) if self.action_reporting_enabled => {
                    self.action_outcomes.push(ReportedActionOutcome {
                        outcome: ExecOutcome {
                            result: Err(error),
                            warnings: Vec::new(),
                        },
                        source: Some(event.handler_key().to_owned()),
                    });
                }
                Err(_) => {}
            }
        }
        self.scheduler.flush();
    }

    /// Drain deferred bindings into registered reactive effects.
    #[must_use = "keep the returned BindingEffect handles alive; dropping them \
                  deregisters the drained bindings and silently disables reactivity"]
    pub fn drain_deferred_bindings(&mut self) -> Vec<BindingEffect> {
        self.deferred_bindings.drain_into_effects(
            &self.effects,
            Rc::clone(&self.expr_cache),
            Rc::clone(&self.state),
        )
    }

    /// Pre-compile every queued binding source into the expression cache.
    pub fn warm_expression_cache(&self) -> usize {
        let mut compiled = 0usize;
        for source in self.deferred_bindings.sources() {
            if self.expr_cache.get_or_compile(source).is_ok() {
                compiled += 1;
            }
        }
        compiled
    }

    /// Build an ActionContext tied to this runtime's services.
    pub fn make_action_ctx(&self) -> ActionContext {
        ActionContext {
            state: self.state.clone(),
            scheduler: self.scheduler.clone(),
            clock: Some(self.task_clock.clone()),
            document_generation: self.document_generation,
            event: None,
            locals: RefCell::new(BTreeMap::new()),
            page_id: Some(self.active_page_key.clone()),
            node_id: None,
            network: self.network.clone(),
            ws_sessions: self.ws_sessions.clone(),
            storage: self.storage.clone(),
            router: self.nav.clone(),
            feedback: self.feedback.clone(),
            async_fb: self.async_feedback.clone(),
            clipboard: self.clipboard.clone(),
            platform: self.platform.clone(),
            capabilities: self.capabilities.clone(),
            policy: self.policy.clone(),
            effect_sink: self.effect_sink.clone(),
            // Never taken here: `make_action_ctx` also builds contexts
            // for due timers, websocket pumps and lifecycle hooks, and a
            // take at this altitude let the FIRST of those burn the id
            // before the user's own chain was built (a pointer dispatch
            // delivers due envelopes before the current one). The input
            // paths inject the id explicitly via [`Runtime::
            // take_activation`]; everything else honestly carries `None`.
            activation: None,
            logic: self.logic.clone(),
            expr_cache: self.expr_cache.clone(),
            cancel: CancellationToken::new(),
            warnings: RefCell::new(Vec::new()),
        }
    }
}

/// Evaluate a node's `gestures.disabled` expression against the state
/// graph. Compilation goes through the runtime cache; a malformed
/// expression disables nothing (fail-open, consistent with bindings).
pub(super) fn node_gestures_disabled(
    document: &crate::document::RuntimeDocument,
    state: &crate::state::StateGraph,
    expr_cache: &crate::expression::ExpressionCache,
    page_id: &str,
    key: crate::document::NodeKey,
) -> bool {
    let Some(source) = config::node_gesture_disabled_source(document, key) else {
        return false;
    };
    let Ok(chunk) = expr_cache.get_or_compile(&source) else {
        return false;
    };
    let node_id = document
        .tree
        .nodes
        .get(key)
        .map(|node| crate::document::tree::node_schema_id(&node.schema).to_owned());
    let expr = Expression { source, chunk };
    let (value, _warnings) = expr.eval(state, Some(page_id), node_id.as_deref());
    // `gestures.disabled` is a boolean expression; a non-bool result or a
    // runtime error disables nothing (fail-open, consistent with bindings).
    value.as_bool().unwrap_or(false)
}
