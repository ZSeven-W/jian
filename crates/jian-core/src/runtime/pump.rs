use super::{Runtime, SwapState};
use crate::widget_state::WidgetState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameDirective {
    pub needs_paint: bool,
    pub next_wake_ms: Option<u64>,
}

impl Runtime {
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn frame_presented(&mut self) {
        self.dirty = false;
    }

    pub fn pump(&mut self, now_ms: u64) -> FrameDirective {
        self.note_time(now_ms);
        self.task_clock.advance_to(self.now_ms);
        for (key, generation) in self.state.storage_cache.take_requests() {
            if !self
                .capabilities
                .check(crate::action::Capability::Storage, "storage_hydrate")
            {
                self.state.storage_cache.complete(
                    &key,
                    generation,
                    Err("storage capability denied".into()),
                );
                self.load_warnings
                    .push(format!("storage hydration denied for `{key}`"));
                continue;
            }
            let backend = self.storage.clone();
            let cache = self.state.storage_cache.clone();
            let source = format!("storage:hydrate:{key}");
            self.task_queue.spawn_future(
                async move {
                    let result = backend.get(&key).await.map_err(|error| error.to_string());
                    cache.complete(&key, generation, result);
                    crate::action::ExecOutcome {
                        result: Ok(()),
                        warnings: Vec::new(),
                    }
                },
                self.document_generation,
                Some(source),
            );
        }
        let timed_out = match &self.swap_state {
            SwapState::AwaitingIme { request_id, parked }
                if self.now_ms.saturating_sub(parked.started_at_ms) >= 500 =>
            {
                Some(*request_id)
            }
            _ => None,
        };
        if let Some(request_id) = timed_out {
            let _ = self.confirm_ime_cancel(request_id);
            self.mark_dirty();
        }
        if !self.tick(self.now_ms).is_empty() {
            self.mark_dirty();
        }
        if !self.task_queue.poll_all(self.now_ms).is_empty() {
            self.scheduler.flush();
            self.mark_dirty();
        }
        let mutation = self.mutation_counter.get();
        if mutation != self.layout_mutation_seen && self.has_responsive_layout_bindings() {
            if let Err(error) = self.relayout() {
                self.load_warnings
                    .push(format!("binding-driven relayout failed: {error}"));
                self.layout_mutation_seen = mutation;
            }
        }
        FrameDirective {
            needs_paint: self.dirty,
            next_wake_ms: self.next_runtime_wake_ms(),
        }
    }

    fn next_runtime_wake_ms(&self) -> Option<u64> {
        let caret = self
            .focused_widget_id()
            .and_then(|id| self.widget_states.get(&id))
            .and_then(|state| match state {
                WidgetState::TextInput(text) => Some(text.next_blink_flip_ms(self.now_ms)),
                _ => None,
            });
        let swap = match &self.swap_state {
            SwapState::AwaitingIme { parked, .. } => Some(parked.started_at_ms.saturating_add(500)),
            SwapState::Idle => None,
        };
        [
            caret,
            swap,
            self.gestures.next_wake_ms(),
            self.task_queue.next_wake_ms(self.now_ms),
            self.task_clock.next_deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn has_responsive_layout_bindings(&self) -> bool {
        self.document.as_ref().is_some_and(|document| {
            document.schema.is_responsive()
                && document
                    .tree
                    .nodes
                    .iter()
                    .any(|(_, node)| crate::binding::has_install_binding(&node.schema))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_survives_pumps_until_a_frame_is_presented() {
        let mut runtime = Runtime::new();
        runtime.frame_presented();
        assert!(!runtime.pump(10).needs_paint);
        runtime.mark_dirty();
        assert!(runtime.pump(11).needs_paint);
        assert!(
            runtime.pump(12).needs_paint,
            "failed paint was not acknowledged"
        );
        runtime.frame_presented();
        assert!(!runtime.pump(13).needs_paint);
    }
}
