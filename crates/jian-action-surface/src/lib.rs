//! `jian-action-surface` — production AI access (Phase 2).
//!
//! Wraps a `jian_core::Runtime` with the protocol surface from
//! `2026-04-24-ai-action-surface.md`. Exposes:
//!
//! - `ActionSurface::list(opts)` — render the MCP-shaped response for
//!   `list_available_actions`. Doesn't consume rate-limit tokens.
//! - `ActionSurface::execute(name, params)` / `execute_with_gate(...)`
//!   — full gating chain in spec §4.2 order: lookup (`unknown_action`)
//!   → StaticHidden / ConfirmGated → StateGated (the supplied
//!   `StateGate`) → swipe throttle → rate limit → concurrency
//!   (`already_running`) → param validation → dispatch. Returns
//!   either `ExecuteOutcome::Ok` (serialises to `{ ok: true }`) or
//!   a structured `ExecuteError` in the four-tier taxonomy.
//!
//! Side effects ride on a host-supplied `ActionDispatcher`. The
//! built-in [`RuntimeDispatcher`] wraps `&mut Runtime` and
//! synthesises real runtime writes — Tap-class actions go through
//! `Runtime::dispatch_pointer`, SetValue writes the state graph,
//! OpenRoute drives the router. Less-trivial source kinds
//! (DoubleTap / LongPress / Swipe* / Scroll / LoadMore) currently
//! return `handler_error` until each gets its own synthesis path.
//! Tests that only care about gating use `SinkDispatcher`.
//!
//! No MCP transport here yet — that's gated behind the future `mcp`
//! cargo feature with rmcp + tokio. The Rust API is enough for
//! in-process use (jian-host-desktop dev panel, OpenPencil editor
//! preview).

pub mod audit;
pub mod concurrency;
pub mod error;
pub mod execute;
pub mod list;
pub mod rate_limit;
pub mod runtime_dispatch;
pub mod swipe_throttle;
pub mod transport;

pub use audit::{
    hash_params, ActionAuditLog, ActionSurfaceAuditEntry, AuditVerdict, ReasonCode, SessionId,
};
pub use error::{
    BusyReason, ExecuteError, ExecutionReason, NotAvailableReason, ValidationReason,
};
pub use list::{list_actions, ListOptions, ListResponse, ListedAction, PageScope};
pub use runtime_dispatch::RuntimeDispatcher;

use crate::concurrency::ConcurrencyTracker;
use crate::rate_limit::TokenBucket;
use jian_core::action_surface::{derive_actions, ActionDefinition};
use jian_ops_schema::document::PenDocument;
use serde::Serialize;
use serde_json::{json, Value};
use std::rc::Rc;
use std::time::Instant;

/// Author-stable build seed. Hosts derive this from `package.version +
/// git.rev` (or any equivalent monotonic identifier) so action hashes
/// stay constant within a build but rotate across releases — same
/// semantics as `2026-04-24-ai-action-surface.md` §3.4.
pub type BuildSalt = [u8; 16];

/// Typed `execute_action` outcome. JSON serialisation matches §5.3:
/// success is `{ "ok": true }`; failure is
/// `{ "ok": false, "error": { kind, reason } }`.
#[derive(Debug, Clone)]
pub enum ExecuteOutcome {
    Ok,
    Err(ExecuteError),
}

impl Serialize for ExecuteOutcome {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Ok => json!({ "ok": true }).serialize(s),
            Self::Err(e) => json!({ "ok": false, "error": e }).serialize(s),
        }
    }
}

/// Trait host adapters implement to actually carry out an `execute_action`
/// once the surface has cleared all the gates. The default no-op impl
/// (`SinkDispatcher`) is useful for unit tests that only care about the
/// gating + validation path.
pub trait ActionDispatcher {
    fn dispatch(
        &mut self,
        action: &ActionDefinition,
        params: &serde_json::Map<String, Value>,
    ) -> Result<(), ExecuteError>;
}

/// No-op dispatcher used by tests + the in-process API when the host
/// hasn't wired a real synthesiser yet. Always returns `Ok(())`.
#[derive(Debug, Default)]
pub struct SinkDispatcher;

impl ActionDispatcher for SinkDispatcher {
    fn dispatch(
        &mut self,
        _action: &ActionDefinition,
        _params: &serde_json::Map<String, Value>,
    ) -> Result<(), ExecuteError> {
        Ok(())
    }
}

/// Spec §4.2 #4 dynamic check. Hosts implement this with access to
/// the live StateGraph + node tree: walk the source node and its
/// ancestors, evaluate any `bindings.visible` / `bindings.disabled`
/// expressions, and return `false` if any ancestor is currently
/// hidden or disabled. The runtime consults the gate **after** the
/// static gates (StaticHidden / ConfirmGated) and **before** rate
/// limit, matching spec §4.2's gate order.
pub trait StateGate {
    fn allows(&self, source_node_id: &str) -> bool;
}

/// No-op gate — every action passes. Default for `ActionSurface`
/// when the host hasn't wired a real state-gate (matches the
/// pre-StateGated behaviour, so existing tests stay green).
#[derive(Debug, Default)]
pub struct AlwaysAllow;

impl StateGate for AlwaysAllow {
    fn allows(&self, _: &str) -> bool {
        true
    }
}

/// Closure adapter — hosts that already have access to the live
/// runtime can wrap a one-shot lookup as a StateGate without
/// declaring a new struct.
pub struct ClosureGate<F>(pub F);

impl<F> StateGate for ClosureGate<F>
where
    F: Fn(&str) -> bool,
{
    fn allows(&self, source_node_id: &str) -> bool {
        (self.0)(source_node_id)
    }
}

/// Per-session state — rate-limit bucket + in-flight action set +
/// swipe throttle.
struct Session {
    bucket: TokenBucket,
    concurrency: ConcurrencyTracker,
    swipe: crate::swipe_throttle::SwipeThrottle,
}

impl Session {
    fn new() -> Self {
        Self {
            bucket: TokenBucket::new(),
            concurrency: ConcurrencyTracker::new(),
            swipe: crate::swipe_throttle::SwipeThrottle::new(),
        }
    }
}

/// Phase 1 wraps a single session — host adapters multiplex over
/// multiple `ActionSurface` instances if they need per-client isolation.
pub struct ActionSurface {
    actions: Vec<ActionDefinition>,
    session: Session,
    audit: Option<Rc<ActionAuditLog>>,
    session_id: String,
}

impl ActionSurface {
    /// Build the surface from a parsed document. Re-derives the action
    /// list under the supplied `build_salt`.
    pub fn from_document(doc: &PenDocument, salt: &BuildSalt) -> Self {
        Self {
            actions: derive_actions(doc, salt),
            session: Session::new(),
            audit: None,
            session_id: "default".to_owned(),
        }
    }

    /// Attach an audit log — every `execute` call writes one
    /// `ActionSurfaceAuditEntry` (success or failure) per spec §8.1.
    pub fn with_audit(mut self, log: Rc<ActionAuditLog>) -> Self {
        self.audit = Some(log);
        self
    }

    /// Override the session id stamped on each audit entry. Default
    /// `"default"` works for single-client setups; multiplex hosts
    /// generate one per AI client connection.
    pub fn with_session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = id.into();
        self
    }

    /// Re-derive after a hot-reload — spec's "AI = 人对称" property
    /// requires the surface to track the current document tree.
    pub fn refresh(&mut self, doc: &PenDocument, salt: &BuildSalt) {
        self.actions = derive_actions(doc, salt);
    }

    /// Read-only view of the underlying action list (test + debug).
    pub fn actions(&self) -> &[ActionDefinition] {
        &self.actions
    }

    /// `list_available_actions` — never rate-limited (§7).
    pub fn list(&self, opts: ListOptions) -> ListResponse {
        list_actions(&self.actions, opts)
    }

    /// `execute_action` with a host-supplied dispatcher. Runs the
    /// gating chain in spec §4.2 order:
    ///
    ///   1. lookup (`unknown_action`)
    ///   2. StaticHidden / ConfirmGated short-circuit
    ///   3. param validation (`missing_required` / `type_mismatch` / …)
    ///   4. rate limit (`rate_limited`)
    ///   5. concurrency (`already_running`)
    ///   6. dispatch
    ///
    /// Putting the rate-limit step before the static/lookup gates
    /// would let a misbehaving client burn the bucket on
    /// unknown/hidden actions and eventually mask their actual error
    /// with `rate_limited` — that would confuse legitimate clients
    /// debugging an aiName change. The bucket is meant to throttle
    /// real dispatch, not to police lookup typos.
    pub fn execute<D: ActionDispatcher>(
        &mut self,
        name: &str,
        params: Option<&Value>,
        dispatcher: &mut D,
    ) -> ExecuteOutcome {
        self.execute_with_gate(name, params, dispatcher, &AlwaysAllow)
    }

    /// Same as [`execute`] but consults a `StateGate` for the
    /// dynamic visibility / disabled check (spec §4.2 #4). Hosts
    /// that hold a live `Runtime` reference pass a `ClosureGate` or
    /// custom `StateGate` impl that walks the node tree + state
    /// graph; clients without that infrastructure use [`execute`]
    /// (which uses `AlwaysAllow`) and rely on the dispatcher itself
    /// to refuse stale calls.
    pub fn execute_with_gate<D: ActionDispatcher, G: StateGate>(
        &mut self,
        name: &str,
        params: Option<&Value>,
        dispatcher: &mut D,
        state_gate: &G,
    ) -> ExecuteOutcome {
        // Spec §4.2 order: 1) lookup → 2) static gate → 3) state gate
        // → 5) rate limit → 6) concurrency → 7) param validation →
        // 8) dispatch. Param validation MUST run after the state
        // gate so a hidden action doesn't get a `validation_failed`
        // verdict (and an attacker with bad params gets `state_gated`
        // instead of leaking the schema shape via the error reason).
        let action = match crate::execute::lookup_static_gate(&self.actions, name) {
            Ok(a) => a,
            Err(e) => {
                self.audit_for(
                    name,
                    None,
                    params,
                    AuditVerdict::Denied,
                    reason_for_err(&e),
                    false,
                );
                return ExecuteOutcome::Err(e);
            }
        };
        let full_name = action.name.full();
        let source_id = action.source_node_id.clone();
        let is_alias = full_name != name;

        // Step 4 — dynamic state-gate. Runs **before** rate limit so
        // a stale-state rejection doesn't burn a bucket token (an
        // AI client repeatedly trying a briefly-hidden button
        // shouldn't get throttled out).
        if !state_gate.allows(&source_id) {
            let e = ExecuteError::state_gated();
            self.audit_for(
                &full_name,
                Some(&source_id),
                params,
                AuditVerdict::Denied,
                ReasonCode::StateGated,
                is_alias,
            );
            return ExecuteOutcome::Err(e);
        }

        // Spec §6.3 — swipe throttle runs *before* rate limit so
        // spammed same-direction swipes report `Busy(already_running)`
        // (the precise failure reason), not `rate_limited` (the
        // generic global throttle). Same audit + same outcome shape.
        if !self
            .session
            .swipe
            .try_acquire(action.source_kind, &full_name)
        {
            let e = ExecuteError::already_running();
            self.audit_for(
                &full_name,
                Some(&source_id),
                params,
                AuditVerdict::Denied,
                ReasonCode::AlreadyRunning,
                is_alias,
            );
            return ExecuteOutcome::Err(e);
        }

        if !self.session.bucket.take() {
            let e = ExecuteError::rate_limited();
            self.audit_for(
                &full_name,
                Some(&source_id),
                params,
                AuditVerdict::Denied,
                ReasonCode::RateLimited,
                is_alias,
            );
            return ExecuteOutcome::Err(e);
        }
        if !self.session.concurrency.try_acquire(&full_name) {
            let e = ExecuteError::already_running();
            self.audit_for(
                &full_name,
                Some(&source_id),
                params,
                AuditVerdict::Denied,
                ReasonCode::AlreadyRunning,
                is_alias,
            );
            return ExecuteOutcome::Err(e);
        }

        // Step 7 — param validation runs *after* gates + rate-limit.
        // Spec §4.2 ordering plus a security argument: an attacker
        // sending bad params to a hidden action should learn
        // `state_gated`, not the param schema shape.
        let params_map = match crate::execute::validate(&action.params, params) {
            Ok(m) => m,
            Err(e) => {
                self.session.concurrency.release(&full_name);
                self.audit_for(
                    &full_name,
                    Some(&source_id),
                    params,
                    AuditVerdict::Denied,
                    reason_for_err(&e),
                    is_alias,
                );
                return ExecuteOutcome::Err(e);
            }
        };

        let result = dispatcher.dispatch(action, &params_map);
        self.session.concurrency.release(&full_name);
        match result {
            Ok(()) => {
                // Single audit row per `execute_action` (§8.1). When the
                // call resolved through an alias, that fact rides on
                // the same row via the dedicated `alias_used` boolean
                // — no extra entry, no double-counted rate-limit math.
                self.audit_for(
                    &full_name,
                    Some(&source_id),
                    params,
                    AuditVerdict::Allowed,
                    ReasonCode::Ok,
                    is_alias,
                );
                ExecuteOutcome::Ok
            }
            Err(e) => {
                self.audit_for(
                    &full_name,
                    Some(&source_id),
                    params,
                    AuditVerdict::Error,
                    reason_for_err(&e),
                    is_alias,
                );
                ExecuteOutcome::Err(e)
            }
        }
    }

    fn audit_for(
        &self,
        action_name: &str,
        source_node_id: Option<&str>,
        params: Option<&Value>,
        outcome: AuditVerdict,
        reason: ReasonCode,
        alias_used: bool,
    ) {
        let Some(log) = self.audit.as_ref() else {
            return;
        };
        let payload = params.cloned().unwrap_or(Value::Null);
        log.record(ActionSurfaceAuditEntry {
            at: Some(Instant::now()),
            action_name: action_name.to_owned(),
            params_hash: crate::audit::hash_params(&payload),
            source_node_id: source_node_id.map(str::to_owned),
            reason_code: reason,
            outcome,
            alias_used,
            session_id: self.session_id.clone(),
        });
    }
}

fn reason_for_err(e: &ExecuteError) -> ReasonCode {
    use crate::error::{
        BusyReason as B, ExecutionReason as Ex, NotAvailableReason as N, ValidationReason as V,
    };
    match e {
        ExecuteError::NotAvailable { reason } => match reason {
            N::UnknownAction => ReasonCode::UnknownAction,
            N::StaticHidden => ReasonCode::StaticHidden,
            N::StateGated => ReasonCode::StateGated,
            N::ConfirmGated => ReasonCode::ConfirmGated,
            N::RateLimited => ReasonCode::RateLimited,
        },
        ExecuteError::Busy {
            reason: B::AlreadyRunning,
        } => ReasonCode::AlreadyRunning,
        ExecuteError::ValidationFailed { reason } => match reason {
            V::MissingRequired => ReasonCode::MissingRequired,
            V::TypeMismatch => ReasonCode::TypeMismatch,
            V::OutOfRange => ReasonCode::SchemaViolation,
            V::SchemaViolation => ReasonCode::SchemaViolation,
        },
        ExecuteError::ExecutionFailed { reason } => match reason {
            Ex::CapabilityDenied => ReasonCode::CapabilityDenied,
            Ex::HandlerError => ReasonCode::HandlerError,
            Ex::Timeout => ReasonCode::Timeout,
            Ex::Unknown => ReasonCode::UnknownError,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> PenDocument {
        serde_json::from_str(
            r#"{
              "version":"0.8.0",
              "state":{ "count":{ "type":"int", "default":0 } },
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"plus", "semantics":{ "aiName":"plus" },
                  "events":{ "onTap": [ { "set": { "$state.count": "$state.count + 1" } } ] }
                },
                { "type":"frame","id":"set-input", "semantics":{ "aiName":"counter" },
                  "bindings": { "bind:value": "$state.count" }
                },
                { "type":"frame","id":"hidden", "semantics":{ "aiName":"hidden_btn", "aiHidden": true },
                  "events":{ "onTap": [ { "set": { "$state.x": "1" } } ] }
                }
              ]}],
              "children":[]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn list_returns_only_available() {
        let doc = fixture();
        let surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let r = surface.list(ListOptions::default());
        let names: Vec<_> = r.actions.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"home.plus"));
        assert!(names.contains(&"home.set_counter"));
        assert!(!names.contains(&"home.hidden_btn"));
    }

    #[test]
    fn execute_unknown_action_is_not_available() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let out = surface.execute("home.does_not_exist", None, &mut sink);
        assert!(matches!(
            out,
            ExecuteOutcome::Err(ExecuteError::NotAvailable {
                reason: NotAvailableReason::UnknownAction
            })
        ));
    }

    #[test]
    fn execute_static_hidden_blocked() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let out = surface.execute("home.hidden_btn", None, &mut sink);
        assert!(matches!(
            out,
            ExecuteOutcome::Err(ExecuteError::NotAvailable {
                reason: NotAvailableReason::StaticHidden
            })
        ));
    }

    #[test]
    fn execute_happy_path_serialises_to_ok_true() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let out = surface.execute("home.plus", None, &mut sink);
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn execute_validation_fails_serialises_per_spec() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let out = surface.execute("home.set_counter", Some(&json!({})), &mut sink);
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["kind"], "ValidationFailed");
        assert_eq!(v["error"]["reason"], "missing_required");
    }

    #[test]
    fn rate_limit_kicks_in_after_ten_calls() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        for _ in 0..10 {
            let r = surface.execute("home.plus", None, &mut sink);
            assert!(matches!(r, ExecuteOutcome::Ok));
        }
        let blocked = surface.execute("home.plus", None, &mut sink);
        assert!(matches!(
            blocked,
            ExecuteOutcome::Err(ExecuteError::NotAvailable {
                reason: NotAvailableReason::RateLimited
            })
        ));
    }

    #[test]
    fn audit_records_success_with_ok_reason() {
        let doc = fixture();
        let log = Rc::new(ActionAuditLog::new(100));
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16])
            .with_audit(log.clone())
            .with_session_id("s-42");
        let mut sink = SinkDispatcher;
        surface.execute("home.plus", None, &mut sink);
        let snap = log.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].outcome, AuditVerdict::Allowed);
        assert_eq!(snap[0].reason_code, ReasonCode::Ok);
        assert_eq!(snap[0].action_name, "home.plus");
        assert_eq!(snap[0].session_id, "s-42");
    }

    #[test]
    fn audit_records_unknown_action() {
        let doc = fixture();
        let log = Rc::new(ActionAuditLog::new(10));
        let mut surface =
            ActionSurface::from_document(&doc, &[0u8; 16]).with_audit(log.clone());
        let mut sink = SinkDispatcher;
        surface.execute("home.does_not_exist", None, &mut sink);
        let snap = log.snapshot();
        assert_eq!(snap[0].reason_code, ReasonCode::UnknownAction);
        assert_eq!(snap[0].outcome, AuditVerdict::Denied);
    }

    #[test]
    fn state_gate_rejects_with_state_gated() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let gate = ClosureGate(|_node_id: &str| false); // every node hidden
        let out = surface.execute_with_gate("home.plus", None, &mut sink, &gate);
        assert!(matches!(
            out,
            ExecuteOutcome::Err(ExecuteError::NotAvailable {
                reason: NotAvailableReason::StateGated
            })
        ));
    }

    #[test]
    fn state_gate_pass_through_dispatches() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let gate = ClosureGate(|_: &str| true);
        let out = surface.execute_with_gate("home.plus", None, &mut sink, &gate);
        assert!(matches!(out, ExecuteOutcome::Ok));
    }

    #[test]
    fn state_gate_does_not_burn_rate_limit_token() {
        // A node that's currently hidden shouldn't drain the
        // 10 calls/sec bucket — otherwise an AI client polling a
        // briefly-hidden button gets throttled out for no reason.
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let blocking = ClosureGate(|_: &str| false);
        // 20 rejections — well past the 10-token cap — but no token
        // should have been consumed.
        for _ in 0..20 {
            surface.execute_with_gate("home.plus", None, &mut sink, &blocking);
        }
        // Now the same action with the gate open should still succeed.
        let pass = ClosureGate(|_: &str| true);
        let out = surface.execute_with_gate("home.plus", None, &mut sink, &pass);
        assert!(matches!(out, ExecuteOutcome::Ok), "bucket should still be full");
    }

    #[test]
    fn audit_records_alias_with_single_entry() {
        // Spec §8.1: one audit row per `execute_action`, full stop.
        // When the call resolved through an alias, that fact lives on
        // the ok row via `alias_used: true` — *not* a second entry.
        let doc: PenDocument = serde_json::from_str(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"btn",
                  "semantics":{ "aiName":"renamed", "aiAliases":["plus"] },
                  "events":{ "onTap": [ { "set": { "$state.x": "1" } } ] }
                }
              ]}],
              "children":[]
            }"#,
        )
        .unwrap();
        let log = Rc::new(ActionAuditLog::new(10));
        let mut surface =
            ActionSurface::from_document(&doc, &[0u8; 16]).with_audit(log.clone());
        let mut sink = SinkDispatcher;
        surface.execute("home.plus", None, &mut sink);
        let snap = log.snapshot();
        assert_eq!(snap.len(), 1, "spec §8.1 — one row per execute");
        assert_eq!(snap[0].reason_code, ReasonCode::Ok);
        assert!(snap[0].alias_used, "alias_used flag should be set");
        assert_eq!(snap[0].action_name, "home.renamed");
    }

    #[test]
    fn alias_resolves() {
        // Authoring scenario: rename via `aiName: "renamed"` while
        // keeping `aiAliases: ["plus"]` so existing AI clients still hit.
        let doc: PenDocument = serde_json::from_str(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"btn",
                  "semantics":{ "aiName":"renamed", "aiAliases":["plus"] },
                  "events":{ "onTap": [ { "set": { "$state.x": "1" } } ] }
                }
              ]}],
              "children":[]
            }"#,
        )
        .unwrap();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let r = surface.execute("home.plus", None, &mut sink);
        assert!(matches!(r, ExecuteOutcome::Ok));
    }
}
