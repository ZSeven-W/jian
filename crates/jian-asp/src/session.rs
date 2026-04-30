//! ASP session state (Plan 18 Task 6).
//!
//! Holds the per-connection state the server main loop carries
//! across requests: the negotiated permission tier, the audit
//! ring buffer, the agent's chosen response format. Token
//! validation itself is delegated to a host-supplied closure
//! — the host knows the bootstrap channel (file / Keychain /
//! Keystore / postMessage) the token came in on, so this module
//! deliberately doesn't bake one in.
//!
//! Session lifecycle:
//! 1. Client opens a transport, sends `Verb::Handshake { token,
//!    client, version }`.
//! 2. Server calls [`TokenValidator::validate`] with the token
//!    bytes. On success, returns the permission tier the bearer
//!    earned; on failure, returns the rejection reason for the
//!    audit ring.
//! 3. Subsequent verbs check `session.permits(verb)` before
//!    dispatching. `Observe` allows read-only verbs; `Act` adds
//!    pointer-synth verbs; `Full` adds direct state mutation.
//! 4. Every dispatched verb appends one [`AuditEntry`] to the
//!    ring (oldest entries drop out when the cap is reached so
//!    the buffer is bounded — agents that need history beyond
//!    that span persist via the runtime's storage service).

use std::collections::VecDeque;

use crate::protocol::{AuditEntry, OutcomePayload};

/// Permission tier the handshake assigns to a session.
///
/// **Monotonic ordering**: `Full` ≥ `Act` ≥ `Observe`. A verb's
/// `requires` returns the minimum tier the verb needs to fire;
/// the session passes when its tier is at least that level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    /// Read-only: `find` / `inspect` / `wait_for` / `assert` /
    /// `audit` / `snapshot`. The agent observes but cannot mutate.
    Observe,
    /// Synthetic pointer / keyboard input + navigation. Can change
    /// the runtime's state by triggering normal user-facing
    /// actions, but cannot bypass them with direct state writes.
    Act,
    /// Direct state-graph writes (`set_state`). The widest tier;
    /// reserved for tests and authoring tools that intentionally
    /// poke the runtime past its UI boundary.
    Full,
}

impl Permission {
    pub fn covers(self, needed: Permission) -> bool {
        self >= needed
    }
}

/// Token validation contract. Hosts implement one with their
/// own bootstrap channel — file / Keychain / Keystore /
/// postMessage / etc. The session module never reads the
/// token's bytes directly so an embedder can choose any
/// representation (hex, base64, opaque blob).
pub trait TokenValidator {
    /// Inspect the handshake's `token` field. Returns the
    /// permission tier the bearer earned, or a static error
    /// string the audit ring records on rejection.
    fn validate(&self, token: &str) -> Result<Permission, &'static str>;
}

/// Default validator that accepts a single in-memory secret +
/// permission pair. Useful for unit tests and the dev-tool agent
/// CLI that bootstraps the token via a file the host writes
/// before launch.
pub struct StaticTokenValidator {
    pub expected: String,
    pub grant: Permission,
}

impl StaticTokenValidator {
    pub fn new(expected: impl Into<String>, grant: Permission) -> Self {
        Self {
            expected: expected.into(),
            grant,
        }
    }
}

impl TokenValidator for StaticTokenValidator {
    fn validate(&self, token: &str) -> Result<Permission, &'static str> {
        // Constant-time compare to avoid leaking the secret one
        // byte at a time. With a tiny (≤ 64-byte) token a naive
        // `==` is fine in practice, but the constant-time form
        // pins the contract — a future host that wires a long
        // HMAC-style token gets the right behaviour by default.
        if constant_time_eq(token.as_bytes(), self.expected.as_bytes()) {
            Ok(self.grant)
        } else {
            Err("invalid handshake token")
        }
    }
}

/// Volatile session state. The server loop owns one of these
/// per connection. `audit` is bounded by `audit_capacity`; older
/// entries fall off the front when the cap is reached.
pub struct Session {
    pub permission: Permission,
    pub client: String,
    pub version: String,
    audit: VecDeque<AuditEntry>,
    audit_capacity: usize,
}

impl Session {
    /// Default audit ring size — Plan 18's spec calls for ~50
    /// tokens per entry, and an LLM agent's `audit` verb typically
    /// asks for the last few dozen entries. 1024 is a comfortable
    /// upper bound that an `audit last_n` request can sample from
    /// without rotating off useful context.
    pub const DEFAULT_AUDIT_CAPACITY: usize = 1024;

    /// Construct after a successful handshake. The validator's
    /// returned `Permission` becomes the session's tier; `client`
    /// + `version` come from the handshake payload.
    pub fn new(
        permission: Permission,
        client: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self::with_capacity(permission, client, version, Self::DEFAULT_AUDIT_CAPACITY)
    }

    /// Same as [`Self::new`] but with a non-default audit ring
    /// capacity. Hosts running long-lived sessions (an editor's
    /// scripting console) bump this; one-shot dev-tool agents
    /// can shrink it.
    pub fn with_capacity(
        permission: Permission,
        client: impl Into<String>,
        version: impl Into<String>,
        audit_capacity: usize,
    ) -> Self {
        Self {
            permission,
            client: client.into(),
            version: version.into(),
            audit: VecDeque::with_capacity(audit_capacity.min(64)),
            audit_capacity,
        }
    }

    /// Append one audit entry, dropping the oldest if the ring
    /// is at capacity. A capacity of 0 is treated as "no audit"
    /// — we drop the entry on the floor rather than violating
    /// the bound (the previous draft always pushed and only
    /// trimmed *next* time, so a zero-cap session retained the
    /// most recent entry for one cycle).
    pub fn record(&mut self, entry: AuditEntry) {
        if self.audit_capacity == 0 {
            return;
        }
        while self.audit.len() >= self.audit_capacity {
            self.audit.pop_front();
        }
        self.audit.push_back(entry);
    }

    /// Return up to `last_n` most recent audit entries, oldest
    /// first. `last_n == 0` returns an empty vec; `last_n` larger
    /// than the buffer returns the whole buffer.
    pub fn audit_tail(&self, last_n: usize) -> Vec<AuditEntry> {
        if last_n == 0 {
            return Vec::new();
        }
        let total = self.audit.len();
        let take = last_n.min(total);
        self.audit.iter().skip(total - take).cloned().collect()
    }

    /// Convenience: derive an `AuditEntry` from the `OutcomePayload`
    /// the verb produced and append it. Centralises the
    /// payload-to-entry mapping so every verb-impl gets the same
    /// audit shape automatically.
    pub fn record_outcome(&mut self, at_ms: u64, outcome: &OutcomePayload) {
        self.record(AuditEntry {
            at_ms,
            verb: outcome.verb.to_owned(),
            target: outcome.target.clone(),
            ok: outcome.ok,
            // Audit entries are tiny — keep the narrative as the
            // summary so the `audit` verb's payload is
            // grep-friendly without a separate format. ≤ a few
            // hundred chars per typical verb.
            summary: outcome.narrative.clone(),
        });
    }
}

/// Constant-time byte compare. Returns `true` only if `a` and `b`
/// are byte-identical and the same length. The wall-clock cost
/// is `O(max(a.len, b.len))` so a timing attacker can't recover
/// the secret one byte at a time NOR the expected length.
///
/// Earlier draft short-circuited on length mismatch, which leaked
/// `len(expected)` to a timing attacker that probes with various
/// candidate lengths. The current form always loops to the
/// longer of the two slices, treating missing bytes as `0` and
/// folding the length difference into the same accumulator.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let len = a.len().max(b.len());
    let mut acc: u8 = 0;
    for i in 0..len {
        let ai = a.get(i).copied().unwrap_or(0);
        let bi = b.get(i).copied().unwrap_or(0);
        acc |= ai ^ bi;
    }
    // Fold the length difference into the result so two equal-
    // prefixed slices of different lengths still compare unequal.
    // Widen `acc` to usize so a 256-byte length diff (which
    // truncates to 0 in a u8 cast) doesn't masquerade as
    // matching.
    ((acc as usize) | (a.len() ^ b.len())) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at: u64, verb: &str) -> AuditEntry {
        AuditEntry {
            at_ms: at,
            verb: verb.to_owned(),
            target: None,
            ok: true,
            summary: String::new(),
        }
    }

    #[test]
    fn permission_ordering_is_monotonic() {
        assert!(Permission::Full.covers(Permission::Act));
        assert!(Permission::Full.covers(Permission::Observe));
        assert!(Permission::Act.covers(Permission::Observe));
        assert!(!Permission::Observe.covers(Permission::Act));
        assert!(!Permission::Act.covers(Permission::Full));
    }

    #[test]
    fn static_validator_accepts_matching_token() {
        let v = StaticTokenValidator::new("secret", Permission::Act);
        assert_eq!(v.validate("secret").unwrap(), Permission::Act);
    }

    #[test]
    fn static_validator_rejects_wrong_token() {
        let v = StaticTokenValidator::new("secret", Permission::Act);
        assert!(v.validate("nope").is_err());
        assert!(v.validate("").is_err());
    }

    #[test]
    fn constant_time_eq_handles_length_mismatch() {
        assert!(!constant_time_eq(b"a", b"ab"));
        assert!(!constant_time_eq(b"ab", b"a"));
        assert!(constant_time_eq(b"ab", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn audit_ring_drops_oldest_at_capacity() {
        let mut s = Session::with_capacity(Permission::Observe, "c", "0.1", 3);
        s.record(entry(1, "tap"));
        s.record(entry(2, "tap"));
        s.record(entry(3, "tap"));
        s.record(entry(4, "tap"));
        let tail = s.audit_tail(10);
        // Capacity 3 → only the last three entries survive; oldest
        // first.
        let times: Vec<u64> = tail.iter().map(|e| e.at_ms).collect();
        assert_eq!(times, vec![2, 3, 4]);
    }

    #[test]
    fn audit_tail_clips_to_buffer_length() {
        let mut s = Session::new(Permission::Observe, "c", "0.1");
        s.record(entry(1, "tap"));
        s.record(entry(2, "tap"));
        // last_n larger than buffer returns whole buffer.
        let tail = s.audit_tail(100);
        assert_eq!(tail.len(), 2);
        // Zero last_n returns empty.
        assert!(s.audit_tail(0).is_empty());
    }

    #[test]
    fn audit_tail_returns_oldest_first() {
        let mut s = Session::new(Permission::Observe, "c", "0.1");
        s.record(entry(10, "find"));
        s.record(entry(20, "tap"));
        s.record(entry(30, "wait_for"));
        let tail = s.audit_tail(2);
        // Two most recent, oldest of those two first.
        let times: Vec<u64> = tail.iter().map(|e| e.at_ms).collect();
        assert_eq!(times, vec![20, 30]);
    }

    #[test]
    fn audit_zero_capacity_drops_every_entry() {
        // Pre-fix: `record` would push first then trim on the
        // *next* call, so a zero-cap session retained one entry
        // for one cycle. Now zero-cap drops on entry.
        let mut s = Session::with_capacity(Permission::Observe, "c", "0.1", 0);
        s.record(entry(1, "tap"));
        s.record(entry(2, "tap"));
        assert!(s.audit_tail(100).is_empty());
    }

    #[test]
    fn constant_time_eq_does_not_short_circuit_on_length() {
        // The new constant-time form folds the length difference
        // into the same accumulator, so two slices that share a
        // prefix but differ in length still compare unequal —
        // and (more importantly) iterate to the same `max(len)`
        // so a timing attacker can't recover the expected length.
        assert!(!constant_time_eq(b"sec", b"secret"));
        assert!(!constant_time_eq(b"secret", b"sec"));
        assert!(constant_time_eq(b"secret", b"secret"));
        // 256-byte length difference: a previous draft that
        // truncated `usize ^ usize` to u8 would have masked this
        // and returned `true`.
        let long = vec![0u8; 256];
        let empty: &[u8] = &[];
        assert!(!constant_time_eq(&long, empty));
    }

    #[test]
    fn record_outcome_preserves_verb_target_and_narrative() {
        let mut s = Session::new(Permission::Act, "c", "0.1");
        let outcome = OutcomePayload::ok("tap", Some("btn".into()), "tapped Submit");
        s.record_outcome(123, &outcome);
        let tail = s.audit_tail(1);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].verb, "tap");
        assert_eq!(tail[0].target.as_deref(), Some("btn"));
        assert_eq!(tail[0].summary, "tapped Submit");
        assert_eq!(tail[0].at_ms, 123);
        assert!(tail[0].ok);
    }
}
