//! Thread-local stack tracking which Effect is currently evaluating,
//! so that `Signal::get()` can auto-subscribe the effect.

use std::cell::RefCell;

pub type SubscriberId = usize;

thread_local! {
    pub(crate) static CURRENT_TRACKER: RefCell<Vec<SubscriberId>> = const { RefCell::new(Vec::new()) };
}

/// Returns the currently-evaluating subscriber, if any.
pub fn current() -> Option<SubscriberId> {
    CURRENT_TRACKER.with(|t| t.borrow().last().copied())
}

/// RAII guard for pushing/popping the tracker stack.
pub struct TrackerGuard;

impl TrackerGuard {
    pub fn push(id: SubscriberId) -> Self {
        CURRENT_TRACKER.with(|t| t.borrow_mut().push(id));
        TrackerGuard
    }
}

impl Drop for TrackerGuard {
    fn drop(&mut self) {
        CURRENT_TRACKER.with(|t| {
            t.borrow_mut().pop();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop() {
        assert_eq!(current(), None);
        {
            let _g = TrackerGuard::push(42);
            assert_eq!(current(), Some(42));
            {
                let _g = TrackerGuard::push(99);
                assert_eq!(current(), Some(99));
            }
            assert_eq!(current(), Some(42));
        }
        assert_eq!(current(), None);
    }
}
