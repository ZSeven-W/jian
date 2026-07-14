use super::Runtime;
use std::collections::BTreeSet;

impl Runtime {
    pub fn cancel_all_tasks(&mut self) {
        self.task_queue.cancel_all();
        self.ws_receive_active.clear();
        self.ws_messages.borrow_mut().clear();
        self.image_requests.clear();
        self.image_completions.borrow_mut().clear();
        self.state.storage_cache.cancel_hydrations();
        self.document_generation = self.document_generation.wrapping_add(1);
    }

    pub(super) fn cancel_non_image_tasks_for_reload(&mut self) {
        let retained = self.reload_retained_task_ids();
        self.task_queue.cancel_all_except(&retained);
        self.ws_receive_active.clear();
        self.ws_messages.borrow_mut().clear();
        self.state.storage_cache.cancel_hydrations();
        self.document_generation = self.document_generation.wrapping_add(1);
    }

    pub(super) fn reload_retained_task_ids(&self) -> BTreeSet<u64> {
        self.image_requests
            .values()
            .map(|request| request.task_id)
            .collect()
    }

    pub(super) fn transfer_reload_image_requests(&mut self) {
        let generation = self.document_generation;
        for (key, request) in &self.image_requests {
            if self.image_store.state(key) == Some(crate::render::image_store::ImageState::Pending)
            {
                request.owner_generation.set(generation);
                self.task_queue.retag_task(request.task_id, generation);
            }
        }
        let stale: Vec<String> = self
            .image_requests
            .iter()
            .filter(|(key, _)| {
                self.image_store.state(key) != Some(crate::render::image_store::ImageState::Pending)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in stale {
            if let Some(request) = self.image_requests.remove(&key) {
                self.task_queue.cancel_task(request.task_id);
            }
        }
    }
}
