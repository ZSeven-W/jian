//! `ActionSurfaceAuditEntry` ring buffer — spec §8.1.
//!
//! Sibling to `jian_core::capability::AuditLog`. Distinct because the
//! shape differs: spec §8.1 carries `action_name`, a hashed
//! `params_hash` (PII shield), the originating `source_node_id` (kept
//! internal, never returned to AI clients), a `reason_code`, the
//! verdict, and the session id.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditVerdict {
    /// Action ran to completion and the dispatcher returned Ok.
    Allowed,
    /// One of the gates rejected it (NotAvailable / Busy / ...).
    Denied,
    /// Dispatcher returned an `ExecutionFailed` error.
    Error,
}

/// `reason_code` covers both success and failure paths so a single
/// audit-log scan tells the operator why each action ran or didn't.
/// Mirrors §8.1's "成功也记,enum 带成功/失败分支" requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    Ok,
    UnknownAction,
    StaticHidden,
    StateGated,
    ConfirmGated,
    RateLimited,
    AlreadyRunning,
    MissingRequired,
    TypeMismatch,
    SchemaViolation,
    CapabilityDenied,
    HandlerError,
    Timeout,
    UnknownError,
    /// `aiAlias` was used to resolve to the canonical action — useful
    /// to track migration progress after a rename. Verdict on this
    /// entry is `Allowed` (the action did run); the `Ok` reason fires
    /// in addition.
    AliasUsed,
}

/// Author-chosen string id for whoever called the surface. Hosts
/// generate one per AI client connection so audit entries trace back
/// to the source.
pub type SessionId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSurfaceAuditEntry {
    #[serde(skip)]
    pub at: Option<Instant>,
    pub action_name: String,
    /// First 8 bytes of a 64-bit FNV-1a over the JSON-serialised
    /// params. Prevents PII (email addresses, free-form text) from
    /// leaking into log files while still letting an operator
    /// correlate "same payload, different verdict" runs.
    pub params_hash: [u8; 8],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_node_id: Option<String>,
    pub reason_code: ReasonCode,
    pub outcome: AuditVerdict,
    /// `true` when the AI client called this action by an `aiAlias`
    /// rather than its current canonical name. Lets operators track
    /// rename-migration progress without scanning two log entries.
    #[serde(default)]
    pub alias_used: bool,
    pub session_id: SessionId,
}

#[derive(Debug)]
pub struct ActionAuditLog {
    entries: RefCell<VecDeque<ActionSurfaceAuditEntry>>,
    max_size: usize,
}

impl ActionAuditLog {
    /// Spec §8.1 default ring buffer: 10000 entries, oldest evicts
    /// first. Hosts can size it differently if disk-spilling is
    /// available — the ring is in-memory only.
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: RefCell::new(VecDeque::with_capacity(max_size.min(1024))),
            max_size,
        }
    }

    pub fn record(&self, entry: ActionSurfaceAuditEntry) {
        let mut q = self.entries.borrow_mut();
        if q.len() >= self.max_size {
            q.pop_front();
        }
        q.push_back(entry);
    }

    pub fn snapshot(&self) -> Vec<ActionSurfaceAuditEntry> {
        self.entries.borrow().iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }
}

impl Default for ActionAuditLog {
    fn default() -> Self {
        Self::new(10_000)
    }
}

/// First 8 bytes of `blake3(serde_json(params))` — spec §8.1's PII
/// shield. Cryptographic strength means an operator reading the log
/// can correlate same-payload runs without recovering the original
/// email / free-text input. Switched from FNV-1a per codex review.
pub fn hash_params(params: &serde_json::Value) -> [u8; 8] {
    let bytes = serde_json::to_vec(params).unwrap_or_default();
    let digest = blake3::hash(&bytes);
    let full = digest.as_bytes();
    let mut out = [0u8; 8];
    out.copy_from_slice(&full[..8]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(action: &str, outcome: AuditVerdict, reason: ReasonCode) -> ActionSurfaceAuditEntry {
        ActionSurfaceAuditEntry {
            at: Some(Instant::now()),
            action_name: action.to_owned(),
            params_hash: [0u8; 8],
            source_node_id: None,
            reason_code: reason,
            outcome,
            alias_used: false,
            session_id: "s".to_owned(),
        }
    }

    #[test]
    fn record_round_trip() {
        let log = ActionAuditLog::new(10);
        log.record(entry("home.tap", AuditVerdict::Allowed, ReasonCode::Ok));
        let snap = log.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].action_name, "home.tap");
    }

    #[test]
    fn ring_evicts_oldest() {
        let log = ActionAuditLog::new(3);
        for n in 0..5 {
            log.record(entry(
                &format!("a{}", n),
                AuditVerdict::Denied,
                ReasonCode::StateGated,
            ));
        }
        let snap = log.snapshot();
        assert_eq!(
            snap.iter()
                .map(|e| e.action_name.clone())
                .collect::<Vec<_>>(),
            vec!["a2".to_owned(), "a3".into(), "a4".into()]
        );
    }

    #[test]
    fn hash_params_is_deterministic_and_8_bytes() {
        let a = hash_params(&json!({ "value": "secret-email@example.com" }));
        let b = hash_params(&json!({ "value": "secret-email@example.com" }));
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
        // Different payload → different hash (effectively).
        let c = hash_params(&json!({ "value": "other" }));
        assert_ne!(a, c);
    }

    #[test]
    fn entry_skips_at_field_in_json() {
        // `at: Instant` isn't serialisable; ensure we use `Option` +
        // `serde(skip)` so JSON dumps still work.
        let e = entry("x", AuditVerdict::Allowed, ReasonCode::Ok);
        let v = serde_json::to_value(&e).unwrap();
        assert!(v.get("at").is_none());
        assert_eq!(v["action_name"], "x");
    }
}
