//! `jian-action-surface` — production AI access (Phase 2).
//!
//! Wraps a `jian_core::Runtime` with the protocol surface from
//! `2026-04-24-ai-action-surface.md`. Exposes:
//!
//! - `ActionSurface::list(opts)` — render the MCP-shaped response for
//!   `list_available_actions`. Doesn't consume rate-limit tokens.
//! - `ActionSurface::execute(name, params)` — full gating chain
//!   (rate limit → unknown_action → static/confirm gate → params
//!   validation → busy check). Returns either `Ok(serde_json::Value)`
//!   in the spec's `{ ok: true }` shape or a structured `ExecuteError`
//!   in the four-tier taxonomy.
//!
//! The actual side-effect (synthesised PointerEvent / SetValue /
//! navigation) is not yet wired — that's a small follow-on; the
//! surface returns `Ok({ok:true})` once gating + validation pass and
//! invokes a host-supplied dispatch closure.
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

pub use audit::{
    hash_params, ActionAuditLog, ActionSurfaceAuditEntry, AuditVerdict, ReasonCode, SessionId,
};
pub use error::{
    BusyReason, ExecuteError, ExecutionReason, NotAvailableReason, ValidationReason,
};
pub use list::{list_actions, ListOptions, ListResponse, ListedAction, PageScope};

use crate::concurrency::ConcurrencyTracker;
use crate::execute::{decide, Decision};
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

/// Per-session state — rate-limit bucket + in-flight action set.
struct Session {
    bucket: TokenBucket,
    concurrency: ConcurrencyTracker,
}

impl Session {
    fn new() -> Self {
        Self {
            bucket: TokenBucket::new(),
            concurrency: ConcurrencyTracker::new(),
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
        let decision = decide(&self.actions, name, params);
        let (action, params_map) = match decision {
            Decision::Dispatch { action, params } => (action, params),
            Decision::Reject(e) => {
                self.audit_for(name, None, params, AuditVerdict::Denied, reason_for_err(&e));
                return ExecuteOutcome::Err(e);
            }
        };
        let full_name = action.name.full();
        let source_id = action.source_node_id.clone();
        let is_alias = full_name != name;

        if !self.session.bucket.take() {
            let e = ExecuteError::rate_limited();
            self.audit_for(
                &full_name,
                Some(&source_id),
                params,
                AuditVerdict::Denied,
                ReasonCode::RateLimited,
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
            );
            return ExecuteOutcome::Err(e);
        }
        // Record alias-used *before* the dispatch so the AuditLog
        // shows migration progress even if the action then errors.
        if is_alias {
            self.audit_for(
                &full_name,
                Some(&source_id),
                params,
                AuditVerdict::Allowed,
                ReasonCode::AliasUsed,
            );
        }
        let result = dispatcher.dispatch(action, &params_map);
        self.session.concurrency.release(&full_name);
        match result {
            Ok(()) => {
                self.audit_for(
                    &full_name,
                    Some(&source_id),
                    params,
                    AuditVerdict::Allowed,
                    ReasonCode::Ok,
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
    fn audit_records_alias_used_then_ok() {
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
        let reasons: Vec<_> = snap.iter().map(|e| e.reason_code).collect();
        assert!(reasons.contains(&ReasonCode::AliasUsed));
        assert!(reasons.contains(&ReasonCode::Ok));
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
