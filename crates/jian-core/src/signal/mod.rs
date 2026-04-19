//! Fine-grained reactive primitive.
//!
//! A `Signal<T>` wraps a value with interior mutability and a subscriber list.
//! Reading via `get()` inside an Effect auto-registers the effect; writing via
//! `set()` schedules all subscribers for re-run on the next microtask flush.

pub mod scheduler;
pub mod tracker;

use scheduler::Scheduler;
use smallvec::SmallVec;
use std::cell::RefCell;
use std::rc::Rc;
use tracker::SubscriberId;

/// A reactive cell.
///
/// Cloning `Signal<T>` shares the underlying storage (like `Rc`).
#[derive(Clone)]
pub struct Signal<T: 'static> {
    inner: Rc<SignalInner<T>>,
}

struct SignalInner<T> {
    value: RefCell<T>,
    subscribers: RefCell<SmallVec<[SubscriberId; 4]>>,
    version: std::cell::Cell<u64>,
    scheduler: Rc<Scheduler>,
}

impl<T: Clone + 'static> Signal<T> {
    pub fn new(value: T, scheduler: Rc<Scheduler>) -> Self {
        Signal {
            inner: Rc::new(SignalInner {
                value: RefCell::new(value),
                subscribers: RefCell::new(SmallVec::new()),
                version: std::cell::Cell::new(0),
                scheduler,
            }),
        }
    }

    /// Read the current value, auto-subscribing the current Effect if any.
    pub fn get(&self) -> T {
        if let Some(sub) = tracker::current() {
            let mut subs = self.inner.subscribers.borrow_mut();
            if !subs.iter().any(|id| *id == sub) {
                subs.push(sub);
            }
        }
        self.inner.value.borrow().clone()
    }

    /// Write a new value and schedule subscribers for re-run on next flush.
    pub fn set(&self, value: T) {
        *self.inner.value.borrow_mut() = value;
        self.inner.version.set(self.inner.version.get().wrapping_add(1));
        self.notify();
    }

    /// Update the value via a closure, then schedule subscribers.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.inner.value.borrow_mut());
        self.inner.version.set(self.inner.version.get().wrapping_add(1));
        self.notify();
    }

    pub fn version(&self) -> u64 {
        self.inner.version.get()
    }

    fn notify(&self) {
        let subs = self.inner.subscribers.borrow().clone();
        for sub in subs {
            self.inner.scheduler.schedule(sub);
        }
    }

    /// Remove a subscriber (called when Effect is disposed).
    #[allow(dead_code)]
    pub(crate) fn unsubscribe(&self, id: SubscriberId) {
        let mut subs = self.inner.subscribers.borrow_mut();
        subs.retain(|s| *s != id);
    }

    pub fn subscriber_count(&self) -> usize {
        self.inner.subscribers.borrow().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get() {
        let sched = Rc::new(Scheduler::new());
        let s = Signal::new(10, sched);
        assert_eq!(s.get(), 10);
        s.set(20);
        assert_eq!(s.get(), 20);
    }

    #[test]
    fn cloning_shares_storage() {
        let sched = Rc::new(Scheduler::new());
        let a = Signal::new(1, sched);
        let b = a.clone();
        a.set(42);
        assert_eq!(b.get(), 42);
    }

    #[test]
    fn version_increments() {
        let sched = Rc::new(Scheduler::new());
        let s = Signal::new(0, sched);
        let v0 = s.version();
        s.set(1);
        assert_eq!(s.version(), v0 + 1);
        s.update(|x| *x += 1);
        assert_eq!(s.version(), v0 + 2);
    }

    #[test]
    fn no_subscriber_when_no_tracker() {
        let sched = Rc::new(Scheduler::new());
        let s = Signal::new(42, sched);
        let _ = s.get();
        assert_eq!(s.subscriber_count(), 0);
    }

    #[test]
    fn subscriber_registered_under_tracker() {
        let sched = Rc::new(Scheduler::new());
        let s = Signal::new(42, sched);
        {
            let _g = tracker::TrackerGuard::push(7);
            let _ = s.get();
        }
        assert_eq!(s.subscriber_count(), 1);
    }

    #[test]
    fn duplicate_subscribe_not_double_counted() {
        let sched = Rc::new(Scheduler::new());
        let s = Signal::new(42, sched);
        {
            let _g = tracker::TrackerGuard::push(7);
            let _ = s.get();
            let _ = s.get();
        }
        assert_eq!(s.subscriber_count(), 1);
    }

    #[test]
    fn unsubscribe_removes() {
        let sched = Rc::new(Scheduler::new());
        let s = Signal::new(42, sched);
        {
            let _g = tracker::TrackerGuard::push(7);
            let _ = s.get();
        }
        assert_eq!(s.subscriber_count(), 1);
        s.unsubscribe(7);
        assert_eq!(s.subscriber_count(), 0);
    }
}
