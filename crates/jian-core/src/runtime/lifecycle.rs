use super::Runtime;

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.task_queue.cancel_all();
        for handle in self.ws_sessions.borrow().values() {
            handle.session.abort();
        }
        self.ws_sessions.borrow_mut().clear();
    }
}
