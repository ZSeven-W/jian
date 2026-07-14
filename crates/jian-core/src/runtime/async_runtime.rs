use super::{ReportedActionOutcome, Runtime};
use crate::action::{ActionContext, CancellationToken, ExecOutcome};
use crate::binding::BindingEffect;
use crate::gesture::SemanticEvent;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

impl Runtime {
    /// Drive timer-based recognizers such as LongPress.
    pub fn tick(&mut self, now_ms: u64) -> Vec<SemanticEvent> {
        self.note_time(now_ms);
        let emitted = self.gestures.tick(self.now_ms);
        if self.input_frozen() {
            return Vec::new();
        }
        for event in &emitted {
            self.dispatch_semantic(event);
        }
        emitted
    }

    pub(super) fn dispatch_semantic(&mut self, event: &SemanticEvent) {
        let (source_node_id, list) = {
            let document = self.document.as_ref().expect("no document loaded");
            let source = document
                .tree
                .nodes
                .get(event.node())
                .map(|node| crate::document::tree::node_schema_id(&node.schema).to_owned());
            (
                source,
                crate::gesture::dispatcher::resolve_handler(document, event),
            )
        };
        let mut context = self.make_action_ctx();
        if let Some(payload) = event_payload(event) {
            context.event = Some(crate::value::RuntimeValue::from(payload));
        }
        context.node_id = source_node_id;
        if let Some(list) = list {
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
            logic: self.logic.clone(),
            expr_cache: self.expr_cache.clone(),
            cancel: CancellationToken::new(),
            warnings: RefCell::new(Vec::new()),
        }
    }
}

fn event_payload(event: &SemanticEvent) -> Option<serde_json::Value> {
    match event {
        SemanticEvent::KeyDown { key, modifiers, .. } => {
            let modifiers: Vec<&str> = [
                (crate::gesture::pointer::Modifiers::SHIFT, "shift"),
                (crate::gesture::pointer::Modifiers::CTRL, "ctrl"),
                (crate::gesture::pointer::Modifiers::ALT, "alt"),
                (crate::gesture::pointer::Modifiers::CMD, "cmd"),
            ]
            .iter()
            .filter_map(|(flag, name)| modifiers.contains(*flag).then_some(*name))
            .collect();
            Some(serde_json::json!({
                "key": key,
                "modifiers": modifiers,
            }))
        }
        SemanticEvent::ScaleStart { focal, .. } => Some(serde_json::json!({
            "focal": { "x": focal.x, "y": focal.y },
        })),
        SemanticEvent::ScaleUpdate { scale, focal, .. } => Some(serde_json::json!({
            "scale": *scale,
            "focal": { "x": focal.x, "y": focal.y },
        })),
        SemanticEvent::RotateUpdate { radians, .. } => Some(serde_json::json!({
            "radians": *radians,
        })),
        _ => None,
    }
}
