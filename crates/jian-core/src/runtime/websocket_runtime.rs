use super::{ReportedActionOutcome, Runtime};
use crate::action::{ActionContext, ExecOutcome};

impl Runtime {
    /// Drain pending WebSocket messages and fire each session's `on_message`
    /// action list. Returns the number of handlers successfully scheduled.
    pub fn pump_websockets(&mut self) -> usize {
        if self.input_frozen() {
            return 0;
        }
        let snapshot: Vec<_> = self
            .ws_sessions
            .borrow()
            .iter()
            .map(|(id, handle)| (id.clone(), handle.session.clone(), handle.generation))
            .collect();
        for (id, session, generation) in snapshot {
            if generation != self.document_generation || !self.ws_receive_active.insert(id.clone())
            {
                continue;
            }
            let messages = self.ws_messages.clone();
            let receive_id = id.clone();
            self.task_queue.spawn_future(
                async move {
                    let batch = session.receive().await;
                    messages.borrow_mut().push((receive_id, generation, batch));
                    ExecOutcome {
                        result: Ok(()),
                        warnings: Vec::new(),
                    }
                },
                generation,
                Some(format!("websocket:receive:{id}")),
            );
        }
        self.collect_task_outcomes();
        let received = std::mem::take(&mut *self.ws_messages.borrow_mut());
        let mut fired = 0usize;
        for (id, generation, messages) in received {
            self.ws_receive_active.remove(&id);
            if generation != self.document_generation {
                continue;
            }
            let handler_json = self
                .ws_sessions
                .borrow()
                .get(&id)
                .and_then(|handle| {
                    (handle.generation == generation).then(|| handle.on_message.clone())
                })
                .flatten();
            let Some(handler_json) = handler_json else {
                continue;
            };
            for message in messages {
                if !self
                    .ws_sessions
                    .borrow()
                    .get(&id)
                    .is_some_and(|handle| handle.generation == generation)
                {
                    break;
                }
                let context = self.make_action_ctx_with_event(serde_json::json!({
                    "id": id,
                    "data": message,
                }));
                if let Err(error) = self.task_queue.spawn(
                    &self.actions,
                    &handler_json,
                    context,
                    self.document_generation,
                    Some(format!("websocket:{id}")),
                ) {
                    if self.action_reporting_enabled {
                        self.action_outcomes.push(ReportedActionOutcome {
                            outcome: ExecOutcome {
                                result: Err(error),
                                warnings: Vec::new(),
                            },
                            source: Some(format!("websocket:{id}")),
                        });
                    }
                    continue;
                }
                self.collect_task_outcomes();
                self.scheduler.flush();
                fired += 1;
            }
        }
        fired
    }

    fn make_action_ctx_with_event(&self, payload: serde_json::Value) -> ActionContext {
        let mut context = self.make_action_ctx();
        context.event = Some(crate::value::RuntimeValue::from(payload));
        context.handler = Some("onMessage".to_owned());
        context
    }
}
