use super::{Runtime, SwapState};
use crate::widget_state::WidgetState;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameDirective {
    pub needs_paint: bool,
    pub next_wake_ms: Option<u64>,
}

impl Runtime {
    /// Update the host clock used by widgets, actions, and scheduled tasks.
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.note_time(now_ms);
    }

    pub fn note_time(&mut self, now_ms: u64) {
        self.now_ms = self.now_ms.max(now_ms);
        self.task_clock.advance_to(self.now_ms);
        self.state.set_now_ms(self.now_ms);
    }

    pub fn last_now_ms(&self) -> u64 {
        self.now_ms
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Install the host's R3 effect sink (replaces the default
    /// `NullEffectSink`). Every effect-producing action then hands its
    /// request to `sink`.
    pub fn set_effect_sink(
        &mut self,
        sink: Rc<dyn crate::action::services::effect_sink::EffectSink>,
    ) {
        self.effect_sink = sink;
    }

    /// Install the host-owned sink for typed visibility and scroll mutations.
    pub fn set_ui_mutation_sink(&mut self, sink: Rc<dyn crate::action::services::UiMutationSink>) {
        self.ui_mutation_sink = sink;
    }

    pub fn set_animation_sink(&mut self, sink: Rc<dyn crate::action::services::AnimationSink>) {
        self.animation_sink = sink;
    }

    pub fn set_action_observer(
        &mut self,
        observer: Rc<dyn crate::action::services::ActionObserver>,
    ) {
        self.observer = observer;
    }

    /// Install the R3 action policy (e.g. the Preview allowlist). `None`
    /// restores "every registered action executes".
    pub fn set_policy(&mut self, policy: Option<Rc<dyn crate::action::policy::ActionPolicy>>) {
        self.policy = policy;
    }

    /// Certify fresh user intent for the NEXT synchronous action chain:
    /// `make_action_ctx` TAKES the id, so the activation applies to that
    /// chain only and is expired for every later delayed/async chain.
    pub fn set_activation(&mut self, activation: Option<u64>) {
        self.pending_activation.set(activation);
    }

    /// Consume the pending activation for the input being dispatched NOW.
    /// The input paths call this once per physical event; contexts built
    /// anywhere else (timers, websockets, lifecycle) never see the id.
    pub fn take_activation(&self) -> Option<u64> {
        self.pending_activation.take()
    }

    pub fn frame_presented(&mut self) {
        self.dirty = false;
    }

    pub fn debug_action_task_count(&self) -> usize {
        self.task_queue.len()
    }

    pub fn debug_active_gesture_count(&self) -> usize {
        self.gestures.active_gesture_count()
    }

    pub fn set_debug_paused(&mut self, paused: bool) {
        self.debug_paused = paused;
    }

    pub fn pump(&mut self, now_ms: u64) -> FrameDirective {
        self.note_time(now_ms);
        self.task_clock.advance_to(self.now_ms);
        self.dispatch_image_requests();
        for (key, generation) in self.state.storage_cache.take_requests() {
            if !self.capabilities.check(
                crate::action::Capability::Storage,
                "storage_hydrate",
                self.now_ms,
            ) {
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
        if self.collect_task_outcomes() {
            self.scheduler.flush();
            self.mark_dirty();
        }
        // AFTER the poll above, which is what resumes a resolver future and
        // pushes its completion. Draining earlier would leave the bytes for
        // the NEXT pump, and nothing asks for one: `FrameDirective.needs_paint`
        // is not carried to hosts that only schedule on `next_wake_ms`, so a
        // resolved image would sit invisible until unrelated input woke a
        // frame. Draining here puts the bytes on screen in this same frame.
        let completions = std::mem::take(&mut *self.image_completions.borrow_mut());
        for completion in completions {
            let current = self
                .image_requests
                .get(&completion.key)
                .is_some_and(|request| {
                    completion.owner_generation.get() == self.document_generation
                        && Rc::ptr_eq(&request.owner_generation, &completion.owner_generation)
                });
            if !current {
                continue;
            }
            let key = completion.key;
            self.image_requests.remove(&key);
            match completion.result {
                Ok(bytes) => {
                    if let Err(error) = self.image_store.resolve(&key, bytes) {
                        self.load_warnings.push(format!("image `{key}`: {error}"));
                    }
                }
                Err(error) => {
                    self.image_store.fail(&key, &error);
                    self.load_warnings.push(format!("image `{key}`: {error}"));
                }
            }
            self.mark_dirty();
        }
        // Spec §6.5 font-invalidation fanout: a process-global registration
        // (possibly by ANOTHER engine) invalidates shaped geometry everywhere.
        // Drift re-runs measurement + layout + spatial before the next paint.
        // On failure the previous layout stays on screen and the retry waits
        // for the NEXT generation/viewport change (rebuild-once), because a
        // successful relayout re-baselines `font_generation_seen` itself.
        let font_generation = self.layout.measure.font_generation();
        if font_generation != self.font_generation_seen {
            if let Err(error) = self.relayout() {
                self.push_layout_error(format!("font-generation relayout failed: {error}"));
                // Rebuild-once: a persistently failing layout must not retry
                // every pump; the next generation change retries.
                self.font_generation_seen = font_generation;
            }
            // On success `build_layout` baselined `font_generation_seen` to
            // the generation it measured under; a registration racing THIS
            // relayout leaves it behind, and the next pump repairs it.
            self.mark_dirty();
        }
        let mutation = self.mutation_counter.get();
        if mutation != self.layout_mutation_seen && self.has_responsive_layout_bindings() {
            if let Err(error) = self.relayout() {
                self.push_layout_error(format!("binding-driven relayout failed: {error}"));
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

    /// Read-only next-wake query (R4 Canonical PreviewInput): the same
    /// minimum deadline [`Runtime::pump`] reports as
    /// `FrameDirective::next_wake_ms` — caret blink, parked IME swap,
    /// gesture timers (deferred Tap / LongPress), scheduled action tasks,
    /// and the task clock — WITHOUT pumping. Hosts that own their frame
    /// loop (OpenPencil's PreviewSession) schedule this deadline and call
    /// `pump` when it arrives, even with no new input.
    pub fn next_wake_ms(&self) -> Option<u64> {
        self.next_runtime_wake_ms()
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
