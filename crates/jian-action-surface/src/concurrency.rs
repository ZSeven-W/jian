//! Same-action serialization — spec §6.1.
//!
//! Within a session, an action whose previous call hasn't returned
//! gets `Busy(already_running)` immediately. Implemented as a small
//! HashSet of in-flight action names; entries clear on completion.

use std::collections::HashSet;

#[derive(Debug, Default)]
pub(crate) struct ConcurrencyTracker {
    in_flight: HashSet<String>,
}

impl ConcurrencyTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `name` as running. Returns `true` if we acquired the
    /// slot, `false` if it was already in flight (caller surfaces
    /// `Busy::AlreadyRunning`).
    pub fn try_acquire(&mut self, name: &str) -> bool {
        if self.in_flight.contains(name) {
            false
        } else {
            self.in_flight.insert(name.to_owned());
            true
        }
    }

    pub fn release(&mut self, name: &str) {
        self.in_flight.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_and_release_round_trip() {
        let mut t = ConcurrencyTracker::new();
        assert!(t.try_acquire("home.sign_in_a3f7"));
        assert!(!t.try_acquire("home.sign_in_a3f7"));
        t.release("home.sign_in_a3f7");
        assert!(t.try_acquire("home.sign_in_a3f7"));
    }

    #[test]
    fn different_actions_dont_block_each_other() {
        let mut t = ConcurrencyTracker::new();
        assert!(t.try_acquire("a"));
        assert!(t.try_acquire("b"));
        assert!(!t.try_acquire("a"));
        assert!(!t.try_acquire("b"));
    }
}
