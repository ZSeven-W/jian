//! AuditLog — ring buffer of every CapabilityGate check.
//!
//! Every `DeclaredCapabilityGate::check` call writes an `AuditEntry` when
//! an `AuditLog` is attached. The buffer is bounded (`max_size`) and
//! oldest-drops-first so audit never grows unbounded in long-running
//! sessions.

use super::gate::Capability;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    Denied,
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub at: Instant,
    pub action: &'static str,
    pub needed: Capability,
    pub verdict: Verdict,
    pub node_id: Option<String>,
}

pub struct AuditLog {
    entries: RefCell<VecDeque<AuditEntry>>,
    max_size: usize,
}

impl AuditLog {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: RefCell::new(VecDeque::with_capacity(max_size.min(1024))),
            max_size,
        }
    }

    pub fn record(&self, entry: AuditEntry) {
        let mut q = self.entries.borrow_mut();
        if q.len() >= self.max_size {
            q.pop_front();
        }
        q.push_back(entry);
    }

    pub fn snapshot(&self) -> Vec<AuditEntry> {
        self.entries.borrow().iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }

    pub fn denied_count(&self) -> usize {
        self.entries
            .borrow()
            .iter()
            .filter(|e| e.verdict == Verdict::Denied)
            .count()
    }

    pub fn allowed_count(&self) -> usize {
        self.entries
            .borrow()
            .iter()
            .filter(|e| e.verdict == Verdict::Allowed)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(action: &'static str, verdict: Verdict) -> AuditEntry {
        AuditEntry {
            at: Instant::now(),
            action,
            needed: Capability::Network,
            verdict,
            node_id: None,
        }
    }

    #[test]
    fn record_then_snapshot_preserves_order() {
        let log = AuditLog::new(10);
        log.record(entry("a", Verdict::Allowed));
        log.record(entry("b", Verdict::Denied));
        let snap = log.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].action, "a");
        assert_eq!(snap[1].action, "b");
    }

    #[test]
    fn overflow_drops_oldest() {
        let log = AuditLog::new(3);
        for a in ["a", "b", "c", "d", "e"] {
            log.record(entry(a, Verdict::Allowed));
        }
        let snap = log.snapshot();
        assert_eq!(
            snap.iter().map(|e| e.action).collect::<Vec<_>>(),
            ["c", "d", "e"]
        );
    }

    #[test]
    fn counts_partition_entries() {
        let log = AuditLog::new(10);
        log.record(entry("a", Verdict::Allowed));
        log.record(entry("b", Verdict::Denied));
        log.record(entry("c", Verdict::Denied));
        assert_eq!(log.allowed_count(), 1);
        assert_eq!(log.denied_count(), 2);
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn empty_log_is_empty() {
        let log = AuditLog::new(10);
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert_eq!(log.snapshot().len(), 0);
    }
}
