//! Scheduler — deduplicates and batches subscriber re-runs.
//!
//! MVP: pending set is drained synchronously via `flush()`. No real microtask
//! queue here; the runtime driver calls `flush()` after each event dispatch.

use super::tracker::SubscriberId;
use std::cell::RefCell;
use std::collections::HashSet;

pub struct Scheduler {
    pending: RefCell<HashSet<SubscriberId>>,
    is_flushing: RefCell<bool>,
    flush_fn: RefCell<Option<Box<dyn Fn(SubscriberId)>>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            pending: RefCell::new(HashSet::new()),
            is_flushing: RefCell::new(false),
            flush_fn: RefCell::new(None),
        }
    }

    /// Install the callback that knows how to re-run a subscriber by id.
    /// Typically set by the Runtime once Effects are registered.
    pub fn set_flush_fn(&self, f: impl Fn(SubscriberId) + 'static) {
        *self.flush_fn.borrow_mut() = Some(Box::new(f));
    }

    pub fn schedule(&self, id: SubscriberId) {
        self.pending.borrow_mut().insert(id);
    }

    pub fn pending_len(&self) -> usize {
        self.pending.borrow().len()
    }

    /// Drain pending subscribers and run each via the registered flush fn.
    /// Noop if no flush fn is set.
    pub fn flush(&self) {
        if *self.is_flushing.borrow() {
            return;
        }
        *self.is_flushing.borrow_mut() = true;
        let drained: Vec<_> = self.pending.borrow_mut().drain().collect();
        if let Some(ref f) = *self.flush_fn.borrow() {
            for id in drained {
                f(id);
            }
        }
        *self.is_flushing.borrow_mut() = false;
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn schedule_dedup() {
        let s = Scheduler::new();
        s.schedule(1);
        s.schedule(1);
        s.schedule(2);
        assert_eq!(s.pending_len(), 2);
    }

    #[test]
    fn flush_runs_registered_fn() {
        let hits = Rc::new(Cell::new(0));
        let s = Scheduler::new();
        {
            let h = hits.clone();
            s.set_flush_fn(move |_| h.set(h.get() + 1));
        }
        s.schedule(1);
        s.schedule(2);
        s.flush();
        assert_eq!(hits.get(), 2);
        assert_eq!(s.pending_len(), 0);
    }

    #[test]
    fn flush_is_reentrancy_guarded() {
        let s = Rc::new(Scheduler::new());
        let s2 = s.clone();
        s.set_flush_fn(move |id| {
            if id == 1 {
                s2.schedule(2);
                s2.flush();
            }
        });
        s.schedule(1);
        s.flush();
        // 2 remains pending because the re-entrant flush was ignored.
        assert_eq!(s.pending_len(), 1);
    }
}
