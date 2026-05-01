//! ASP verb dispatch table + per-verb handler bodies (Plan 18
//! Tasks 3+4).
//!
//! Each verb maps to one handler that returns an
//! [`OutcomePayload`]. The dispatch entry point ([`dispatch`])
//! checks the session's permission tier first, then routes to
//! the right handler. Handlers borrow `&mut Runtime` so they
//! can read the document tree / state / spatial index, mutate
//! state when their verb is a writer, and synthesise pointer
//! events for `tap` / `type` / `scroll` / `swipe`.
//!
//! ## What this commit ships
//!
//! - [`Permission`]-based gating wired through [`min_permission`].
//! - Real implementations:
//!   - `find` (resolver-driven; emits the matched ids)
//!   - `inspect what=node_props` (compact node summary)
//!   - `inspect what=route` (current path + stack depth via
//!     `Runtime`'s router)
//!   - `audit` (last_n entries from the session ring)
//!   - `exit` (cooperative shutdown signal)
//! - Everything else returns
//!   `OutcomePayload::error("not yet implemented")` so the wire
//!   surface stays uniform; Phase 3 fills these in:
//!   `tap` / `type` / `scroll` / `swipe` (need pointer synth +
//!   gesture-arena coupling), `wait_for` / `assert` (need the
//!   expression evaluator borrows resolved against an
//!   `&Runtime`), `navigate` / `set_state` / `snapshot` /
//!   `inspect ax_tree | state`.

use crate::protocol::{DetailKind, InspectKind, NodeSummary, OutcomePayload, Verb};
use crate::selector::Selector;
use crate::session::{Permission, Session};
use jian_core::Runtime;

#[cfg(feature = "dev-asp")]
pub mod ax_verb;
#[cfg(feature = "dev-asp")]
pub mod expr_verb;
#[cfg(feature = "dev-asp")]
pub mod find_verb;
#[cfg(feature = "dev-asp")]
pub mod scroll_verb;
#[cfg(feature = "dev-asp")]
pub mod snapshot_verb;
#[cfg(feature = "dev-asp")]
pub mod state_verb;
#[cfg(feature = "dev-asp")]
pub mod swipe_verb;
#[cfg(feature = "dev-asp")]
pub mod tap_verb;
#[cfg(feature = "dev-asp")]
pub mod type_verb;

#[cfg(feature = "dev-asp")]
pub use expr_verb::{run_assert, run_wait_for};
#[cfg(feature = "dev-asp")]
pub use find_verb::collect_node_summaries;
#[cfg(feature = "dev-asp")]
pub use state_verb::{run_inspect_state, run_navigate, run_set_state};
#[cfg(feature = "dev-asp")]
pub use tap_verb::run_tap;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{InspectKind, Verb};
    use crate::session::{Permission, Session};
    use jian_ops_schema::document::PenDocument;

    fn make_runtime_with_doc(doc_json: &str) -> Runtime {
        let schema: PenDocument = jian_ops_schema::load_str(doc_json).unwrap().value;
        let mut rt = Runtime::new_from_document(schema).unwrap();
        rt.build_layout((480.0, 320.0)).unwrap();
        rt.rebuild_spatial();
        rt
    }

    fn fixture_doc() -> &'static str {
        r##"{
          "formatVersion": "1.0", "version": "1.0.0", "id": "fx",
          "app": { "name": "fx", "version": "1", "id": "fx" },
          "children": [
            {
              "type": "frame", "id": "root", "width": 480, "height": 320, "x": 0, "y": 0,
              "children": [
                { "type": "rectangle", "id": "save-btn", "x": 100, "y": 200, "width": 100, "height": 40,
                  "children": [ { "type": "text", "id": "save-label", "content": "Save" } ]
                }
              ]
            }
          ]
        }"##
    }

    #[test]
    fn min_permission_routes_correctly() {
        assert_eq!(
            min_permission(&Verb::Find {
                selector: Selector::default(),
                limit: None
            }),
            Permission::Observe
        );
        assert_eq!(
            min_permission(&Verb::Tap {
                selector: Selector::default()
            }),
            Permission::Act
        );
        assert_eq!(
            min_permission(&Verb::SetState {
                scope: "$app".into(),
                key: "x".into(),
                value_json: "1".into(),
            }),
            Permission::Full
        );
    }

    #[test]
    fn dispatch_denied_when_permission_too_low() {
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out, ctl) = dispatch(
            &Verb::Tap {
                selector: Selector::default(),
            },
            &mut rt,
            &mut session,
        );
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Denied"));
        assert_eq!(ctl, DispatchControl::Continue);
    }

    #[test]
    fn dispatch_find_returns_match_summary() {
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let sel = Selector {
            id: Some("save-btn".into()),
            ..Default::default()
        };
        let (out, ctl) = dispatch(
            &Verb::Find {
                selector: sel,
                limit: None,
            },
            &mut rt,
            &mut session,
        );
        assert!(out.ok, "expected ok, got {:?}", out);
        assert_eq!(out.target.as_deref(), Some("save-btn"));
        assert_eq!(ctl, DispatchControl::Continue);
        match out.detail {
            Some(DetailKind::Node { node }) => assert_eq!(node.id, "save-btn"),
            other => panic!("expected Node detail, got {:?}", other),
        }
    }

    #[test]
    fn dispatch_find_no_match_returns_not_found() {
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out, _) = dispatch(
            &Verb::Find {
                selector: Selector {
                    id: Some("nope".into()),
                    ..Default::default()
                },
                limit: None,
            },
            &mut rt,
            &mut session,
        );
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("NotFound"));
    }

    #[test]
    fn dispatch_find_with_zero_limit_returns_invalid() {
        // Pre-fix: `limit: 0` truncated all summaries but the
        // success branch fired anyway, returning `ok: true` with
        // an empty payload. Now we surface `invalid` so the
        // agent gets a clear "tighten the limit" signal.
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out, _) = dispatch(
            &Verb::Find {
                selector: Selector {
                    id: Some("save-btn".into()),
                    ..Default::default()
                },
                limit: Some(0),
            },
            &mut rt,
            &mut session,
        );
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
    }

    #[test]
    fn dispatch_inspect_node_props_returns_node_detail() {
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out, _) = dispatch(
            &Verb::Inspect {
                selector: Some(Selector {
                    id: Some("save-label".into()),
                    ..Default::default()
                }),
                what: InspectKind::NodeProps,
            },
            &mut rt,
            &mut session,
        );
        assert!(out.ok);
        match out.detail {
            Some(DetailKind::Node { node }) => {
                assert_eq!(node.id, "save-label");
                assert_eq!(node.role.as_deref(), Some("text"));
                assert_eq!(node.text.as_deref(), Some("Save"));
            }
            other => panic!("expected Node detail, got {:?}", other),
        }
    }

    #[test]
    fn dispatch_audit_returns_session_tail() {
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        // Pre-populate by dispatching a find first.
        let _ = dispatch(
            &Verb::Find {
                selector: Selector {
                    id: Some("save-btn".into()),
                    ..Default::default()
                },
                limit: None,
            },
            &mut rt,
            &mut session,
        );
        // The dispatcher itself doesn't touch the audit ring —
        // that's the server loop's job. Manually record one entry
        // so the audit verb has something to return.
        session.record_outcome(
            42,
            &OutcomePayload::ok("find", Some("save-btn".into()), "1 matches"),
        );
        let (out, _) = dispatch(&Verb::Audit { last_n: Some(5) }, &mut rt, &mut session);
        assert!(out.ok);
        match out.detail {
            Some(DetailKind::Audit { entries }) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].verb, "find");
            }
            other => panic!("expected Audit detail, got {:?}", other),
        }
    }

    #[test]
    fn dispatch_exit_returns_exit_control() {
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out, ctl) = dispatch(&Verb::Exit, &mut rt, &mut session);
        assert!(out.ok);
        assert_eq!(ctl, DispatchControl::Exit);
    }

    #[test]
    fn dispatch_inspect_ax_tree_returns_detail() {
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Full, "test", "0.1");
        let (out, _) = dispatch(
            &Verb::Inspect {
                selector: None,
                what: InspectKind::AxTree,
            },
            &mut rt,
            &mut session,
        );
        assert!(out.ok, "expected ax_tree to succeed, got {:?}", out);
        match out.detail {
            Some(DetailKind::AxTree { .. }) => {}
            other => panic!("expected AxTree detail, got {:?}", other),
        }
    }

    // ──────────────────────────────────────────────────────────────
    // Plan 18 ASP prod mode (C0) — list_actions + Mode dispatch
    // ──────────────────────────────────────────────────────────────

    #[test]
    fn list_actions_round_trips_on_the_wire() {
        // The prod-mode discovery verb must round-trip cleanly with
        // optional cursor + limit, including the "no fields" form.
        let bare: Verb = serde_json::from_str(r#"{"verb":"list_actions"}"#).unwrap();
        assert!(matches!(
            bare,
            Verb::ListActions {
                cursor: None,
                limit: None
            }
        ));
        let paged: Verb =
            serde_json::from_str(r#"{"verb":"list_actions","cursor":"opaque","limit":50}"#)
                .unwrap();
        match paged {
            Verb::ListActions { cursor, limit } => {
                assert_eq!(cursor.as_deref(), Some("opaque"));
                assert_eq!(limit, Some(50));
            }
            other => panic!("expected ListActions, got {:?}", other),
        }
    }

    #[test]
    fn dispatch_list_actions_returns_empty_action_list_today() {
        // C0 stub: the projection is intentionally empty until C1
        // wires the jian-action-surface derivation. The wire shape
        // must already match the spec — `ActionList { actions:[],
        // next_cursor: None }` so client integration can land
        // without waiting on the full pipeline.
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out, ctl) = dispatch(
            &Verb::ListActions {
                cursor: None,
                limit: None,
            },
            &mut rt,
            &mut session,
        );
        assert!(out.ok);
        assert_eq!(ctl, DispatchControl::Continue);
        match out.detail {
            Some(DetailKind::ActionList {
                actions,
                next_cursor,
            }) => {
                assert!(actions.is_empty(), "C0 stub returns no rows yet");
                assert!(next_cursor.is_none());
            }
            other => panic!("expected ActionList detail, got {:?}", other),
        }
    }

    #[test]
    fn prod_mode_rejects_structural_verbs_without_executing_handlers() {
        // Plan 18 prod-mode rejection: every structural verb must
        // surface as `OutcomePayload::unsupported_verb_in_prod`
        // BEFORE its handler would have run. The session stays open
        // (`DispatchControl::Continue`) so a misbehaving client can
        // self-correct without having to re-handshake.
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Full, "test", "0.1");
        let cases: &[(Verb, &str)] = &[
            (
                Verb::Find {
                    selector: Selector::default(),
                    limit: None,
                },
                "find",
            ),
            (
                Verb::Inspect {
                    selector: None,
                    what: InspectKind::AxTree,
                },
                "inspect",
            ),
            (Verb::Snapshot { format: None }, "snapshot"),
            (Verb::Audit { last_n: None }, "audit"),
            (
                Verb::WaitFor {
                    expr: "true".into(),
                    timeout_ms: None,
                },
                "wait_for",
            ),
            (
                Verb::Assert {
                    expr: "true".into(),
                },
                "assert",
            ),
            (
                Verb::Navigate {
                    path: "/".into(),
                    mode: None,
                },
                "navigate",
            ),
            (
                Verb::SetState {
                    scope: "$app".into(),
                    key: "x".into(),
                    value_json: "1".into(),
                },
                "set_state",
            ),
        ];
        for (verb, expected_name) in cases {
            let (out, ctl) = dispatch_with_mode(verb, &mut rt, &mut session, Mode::Prod);
            assert!(
                !out.ok,
                "prod mode should reject {expected_name}, got ok={:?}",
                out
            );
            assert_eq!(out.verb, *expected_name);
            assert!(
                out.narrative.contains("not available in production mode"),
                "expected prod-mode rejection message for {expected_name}, got: {}",
                out.narrative
            );
            assert_eq!(
                ctl,
                DispatchControl::Continue,
                "prod-mode rejection should leave the session open"
            );
        }
    }

    #[test]
    fn prod_allowed_verbs_const_matches_predicate() {
        // Single source of truth: every variant for which
        // `is_prod_allowed(...)` returns true must have its name in
        // PROD_ALLOWED_VERBS, and vice versa. Pins the const +
        // predicate together so C1+ can't drift them silently.
        // Codex round 2 NIT: this test now actually exercises
        // is_prod_allowed instead of just checking a hard-coded
        // list against the const.
        let prod_verb_samples: Vec<(Verb, &str)> = vec![
            (
                Verb::Handshake {
                    token: "t".into(),
                    client: "c".into(),
                    version: "0.1".into(),
                },
                "handshake",
            ),
            (
                Verb::ListActions {
                    cursor: None,
                    limit: None,
                },
                "list_actions",
            ),
            (
                Verb::Tap {
                    selector: Selector::default(),
                },
                "tap",
            ),
            (
                Verb::Type {
                    selector: Selector::default(),
                    text: "x".into(),
                    clear: None,
                },
                "type",
            ),
            (
                Verb::Scroll {
                    selector: Selector::default(),
                    direction: crate::protocol::ScrollDir::Up,
                    distance: None,
                },
                "scroll",
            ),
            (
                Verb::Swipe {
                    selector: Selector::default(),
                    direction: crate::protocol::ScrollDir::Down,
                    distance: None,
                },
                "swipe",
            ),
            (Verb::Exit, "exit"),
        ];
        // Predicate side: every prod sample must be allowed AND
        // every dev-only verb must be denied.
        for (verb, name) in &prod_verb_samples {
            assert!(
                is_prod_allowed(verb),
                "is_prod_allowed should accept {name}"
            );
        }
        let dev_only_samples: Vec<(Verb, &str)> = vec![
            (
                Verb::Find {
                    selector: Selector::default(),
                    limit: None,
                },
                "find",
            ),
            (
                Verb::Inspect {
                    selector: None,
                    what: InspectKind::AxTree,
                },
                "inspect",
            ),
            (Verb::Snapshot { format: None }, "snapshot"),
            (Verb::Audit { last_n: None }, "audit"),
        ];
        for (verb, name) in &dev_only_samples {
            assert!(
                !is_prod_allowed(verb),
                "is_prod_allowed should reject {name}"
            );
        }
        // Const side: name list matches the predicate's accepted set.
        let names_from_samples: Vec<&str> = prod_verb_samples.iter().map(|(_, n)| *n).collect();
        assert_eq!(names_from_samples.as_slice(), PROD_ALLOWED_VERBS);
    }

    #[test]
    fn prod_mode_allows_full_prod_verb_set() {
        // Mirror of the spec's prod verb set — every entry in
        // PROD_ALLOWED_VERBS must reach its handler under
        // dispatch_with_mode(Mode::Prod). Codex round 1 MEDIUM:
        // round-1 only covered 4 of 7; this case-set is the full 7.
        // We don't validate handler content here (handlers may
        // legitimately return `not_found` against the fixture) —
        // the contract under test is "prod dispatch reaches the
        // handler" vs "prod dispatch short-circuits with
        // UnsupportedVerbInProd".
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Full, "test", "0.1");
        let cases: &[Verb] = &[
            // Handshake takes the special path — `dispatch` errors
            // with "consumed before dispatch" and `Exit` control,
            // but the prod-mode gate must NOT fire on it.
            Verb::Handshake {
                token: "t".into(),
                client: "c".into(),
                version: "0.1".into(),
            },
            Verb::ListActions {
                cursor: None,
                limit: None,
            },
            Verb::Tap {
                selector: Selector {
                    id: Some("save-btn".into()),
                    ..Default::default()
                },
            },
            Verb::Type {
                selector: Selector {
                    id: Some("save-btn".into()),
                    ..Default::default()
                },
                text: "hi".into(),
                clear: None,
            },
            Verb::Scroll {
                selector: Selector {
                    id: Some("save-btn".into()),
                    ..Default::default()
                },
                direction: crate::protocol::ScrollDir::Up,
                distance: None,
            },
            Verb::Swipe {
                selector: Selector {
                    id: Some("save-btn".into()),
                    ..Default::default()
                },
                direction: crate::protocol::ScrollDir::Down,
                distance: None,
            },
            Verb::Exit,
        ];
        for verb in cases {
            let (out, _) = dispatch_with_mode(verb, &mut rt, &mut session, Mode::Prod);
            assert_ne!(
                out.error.as_deref(),
                Some("UnsupportedVerbInProd"),
                "verb {verb:?} should pass prod gating; got error tag: {:?}",
                out.error
            );
        }
    }

    #[test]
    fn prod_rejection_carries_unsupported_verb_in_prod_error_tag() {
        // Codex round 1 NIT: the prod rejection narrative used to
        // be the only client-visible signal. Now `error` carries
        // the stable `UnsupportedVerbInProd` tag so a client can
        // branch on it without parsing prose.
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out, _) = dispatch_with_mode(
            &Verb::Snapshot { format: None },
            &mut rt,
            &mut session,
            Mode::Prod,
        );
        assert_eq!(out.error.as_deref(), Some("UnsupportedVerbInProd"));
        // Narrative still includes the allowed-verb list so the
        // human-readable form helps when a client logs the
        // OutcomePayload directly.
        for verb in PROD_ALLOWED_VERBS {
            assert!(
                out.narrative.contains(verb),
                "narrative should list {verb}, got: {}",
                out.narrative
            );
        }
    }

    #[test]
    fn list_actions_rejects_invalid_pagination_inputs() {
        // Codex round 1 MEDIUM: pagination validation lands now so
        // C1's real projection inherits the safety net.
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        for (verb, expected_msg_substring) in [
            (
                Verb::ListActions {
                    cursor: None,
                    limit: Some(0),
                },
                "limit must be > 0",
            ),
            (
                Verb::ListActions {
                    cursor: None,
                    limit: Some(LIST_ACTIONS_MAX_LIMIT + 1),
                },
                "exceeds max",
            ),
            (
                Verb::ListActions {
                    cursor: Some("stale-cursor".into()),
                    limit: None,
                },
                "invalid cursor",
            ),
        ] {
            let (out, _) = dispatch(&verb, &mut rt, &mut session);
            assert!(!out.ok, "expected invalid for {verb:?}, got: {:?}", out);
            assert_eq!(out.error.as_deref(), Some("Invalid"));
            assert!(
                out.narrative.contains(expected_msg_substring),
                "narrative should contain {expected_msg_substring:?}, got: {}",
                out.narrative
            );
        }
    }

    #[test]
    fn list_actions_accepts_empty_cursor_string() {
        // The pagination guard rejects non-empty cursor strings, but
        // an empty string is the deserialized form of `Some("")` and
        // should be treated as "no cursor" — same as `None`. Pins
        // the boundary so a future tightening doesn't accidentally
        // flag the empty-cursor case.
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out, _) = dispatch(
            &Verb::ListActions {
                cursor: Some(String::new()),
                limit: None,
            },
            &mut rt,
            &mut session,
        );
        assert!(out.ok);
    }

    #[test]
    fn action_list_detail_serializes_next_cursor_as_explicit_null() {
        // Codex round 1 HIGH: a missing `next_cursor` on the wire
        // would let an old server response and a "no more pages"
        // response look identical. Pin that the field is always
        // serialised, including as JSON null.
        let detail = DetailKind::ActionList {
            actions: vec![],
            next_cursor: None,
        };
        let json = serde_json::to_value(&detail).unwrap();
        assert!(
            json.get("next_cursor").is_some(),
            "next_cursor must be serialised even when None: {json}"
        );
        assert!(json["next_cursor"].is_null());
    }

    #[test]
    fn dev_mode_dispatch_with_mode_is_pass_through() {
        // `dispatch_with_mode(_, _, _, Mode::Dev)` must behave
        // identically to the historical `dispatch` — every existing
        // caller's invariant survives the mode addition.
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let verb = Verb::Find {
            selector: Selector {
                id: Some("save-btn".into()),
                ..Default::default()
            },
            limit: None,
        };
        let (a, _) = dispatch_with_mode(&verb, &mut rt, &mut session, Mode::Dev);
        let (b, _) = dispatch(&verb, &mut rt, &mut session);
        assert_eq!(a.verb, b.verb);
        assert_eq!(a.ok, b.ok);
    }
}

/// Outcome of [`dispatch`] beyond the payload itself — lets the
/// server main loop know whether to keep accepting requests or
/// tear down the session. `Continue` is the steady state;
/// `Exit` flips on `Verb::Exit` and on irrecoverable handler
/// failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchControl {
    Continue,
    Exit,
}

/// Operating mode for [`dispatch_with_mode`] (Plan 18 ASP prod
/// mode / C0). `Dev` is the historical full-surface dispatch with
/// `find` / `inspect` / `snapshot` / `audit` / structural verbs;
/// `Prod` exposes only `handshake` / `list_actions` / `tap` / `type`
/// / `scroll` / `swipe` / `exit` and rejects every structural verb
/// with [`OutcomePayload::unsupported_verb_in_prod`] (stable error
/// tag `UnsupportedVerbInProd`) and `DispatchControl::Continue`.
///
/// Runtime gating is the **first** layer: the structural verb
/// handlers may still be linked into the binary (build-time
/// elision lands in C3 via the `prod-asp` cargo feature). A prod-
/// mode session that never calls a structural verb runs the same
/// code path either way; the rejection happens at the dispatch-
/// table layer before any handler body executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Full verb surface — used by `jian dev --asp` and by the
    /// CI integration tests for ASP itself. Default for back-
    /// compat with existing `dispatch` callers.
    #[default]
    Dev,
    /// Production-safe surface. Structural verbs reject loudly so a
    /// production agent can't accidentally request data the prod
    /// channel deliberately doesn't expose.
    Prod,
}

/// Verb names allowed in [`Mode::Prod`]. Single source of truth —
/// the predicate `is_prod_allowed` and the rejection narrative in
/// [`dispatch_with_mode`] both read from this slice (codex round 1
/// NIT). The order is the spec's: handshake → discovery → ops →
/// teardown.
pub const PROD_ALLOWED_VERBS: &[&str] = &[
    "handshake",
    "list_actions",
    "tap",
    "type",
    "scroll",
    "swipe",
    "exit",
];

/// True when `verb` is allowed in [`Mode::Prod`]. Mirror of the
/// spec's prod verb set; the `match` enumerates exactly the
/// variants whose names appear in [`PROD_ALLOWED_VERBS`].
fn is_prod_allowed(verb: &Verb) -> bool {
    matches!(
        verb,
        Verb::Handshake { .. }
            | Verb::ListActions { .. }
            | Verb::Tap { .. }
            | Verb::Type { .. }
            | Verb::Scroll { .. }
            | Verb::Swipe { .. }
            | Verb::Exit
    )
}

/// Minimum permission tier each verb requires. The server's
/// dispatch entry point checks this before routing — saves the
/// handler bodies from repeating the gate.
pub fn min_permission(verb: &Verb) -> Permission {
    match verb {
        // Handshake is special: the session doesn't exist yet, so
        // it never reaches this function. Default to `Observe` so
        // a misuse downstream fails closed.
        Verb::Handshake { .. } => Permission::Observe,
        // Read-only verbs.
        Verb::Find { .. }
        | Verb::Inspect { .. }
        | Verb::WaitFor { .. }
        | Verb::Assert { .. }
        | Verb::Audit { .. }
        | Verb::Snapshot { .. }
        | Verb::ListActions { .. }
        | Verb::Exit => Permission::Observe,
        // Pointer-synth + navigation verbs touch user-facing UI
        // state; they need `Act`.
        Verb::Tap { .. }
        | Verb::Type { .. }
        | Verb::Scroll { .. }
        | Verb::Swipe { .. }
        | Verb::Navigate { .. } => Permission::Act,
        // Direct state writes get the widest tier.
        Verb::SetState { .. } => Permission::Full,
    }
}

/// Route a verb to its handler. Permission gate fires first;
/// short-circuits to `OutcomePayload::denied` on insufficient
/// tier. Returns `(payload, control)` so the server loop can
/// surface the response and decide whether to drop the session.
///
/// **Production callers**: use [`dispatch_with_mode`] with
/// [`Mode::Prod`] instead — `dispatch` accepts every verb the
/// protocol declares, including `find` / `inspect` / `snapshot` /
/// `audit` that prod mode rejects (Plan 18 ASP prod mode / C0).
pub fn dispatch(
    verb: &Verb,
    runtime: &mut Runtime,
    session: &mut Session,
) -> (OutcomePayload, DispatchControl) {
    let needed = min_permission(verb);
    if !session.permission.covers(needed) {
        let payload = OutcomePayload::denied(
            verb_name(verb),
            "session permission tier insufficient for this verb",
            Some("re-handshake with a token granting the required tier"),
        );
        return (payload, DispatchControl::Continue);
    }
    match verb {
        Verb::Handshake { .. } => (
            OutcomePayload::invalid(
                "handshake",
                "handshake should be consumed before dispatch — server bug",
            ),
            DispatchControl::Exit,
        ),
        Verb::Find { selector, limit } => (
            run_find(runtime, selector, *limit),
            DispatchControl::Continue,
        ),
        Verb::Tap { selector } => (run_tap(runtime, selector), DispatchControl::Continue),
        Verb::Type {
            selector,
            text,
            clear,
        } => (
            type_verb::run_type(runtime, selector, text, *clear),
            DispatchControl::Continue,
        ),
        Verb::Scroll {
            selector,
            direction,
            distance,
        } => (
            scroll_verb::run_scroll(runtime, selector, *direction, *distance),
            DispatchControl::Continue,
        ),
        Verb::Swipe {
            selector,
            direction,
            distance,
        } => (
            swipe_verb::run_swipe(runtime, selector, *direction, *distance),
            DispatchControl::Continue,
        ),
        Verb::Snapshot { format } => (
            snapshot_verb::run_snapshot(runtime, *format),
            DispatchControl::Continue,
        ),
        Verb::Navigate { path, mode } => (
            run_navigate(runtime, path, *mode),
            DispatchControl::Continue,
        ),
        Verb::SetState {
            scope,
            key,
            value_json,
        } => (
            run_set_state(runtime, scope, key, value_json),
            DispatchControl::Continue,
        ),
        Verb::Assert { expr } => (run_assert(runtime, expr), DispatchControl::Continue),
        Verb::WaitFor { expr, timeout_ms } => (
            run_wait_for(runtime, expr, *timeout_ms),
            DispatchControl::Continue,
        ),
        Verb::Inspect { selector, what } => (
            run_inspect(runtime, selector.as_ref(), *what),
            DispatchControl::Continue,
        ),
        Verb::Audit { last_n } => {
            let n = last_n.unwrap_or(32) as usize;
            let entries = session.audit_tail(n);
            let outcome = OutcomePayload::ok("audit", None, format!("{} entries", entries.len()))
                .with_detail(DetailKind::Audit { entries });
            (outcome, DispatchControl::Continue)
        }
        Verb::ListActions { cursor, limit } => (
            run_list_actions(runtime, cursor.as_deref(), *limit),
            DispatchControl::Continue,
        ),
        Verb::Exit => (
            OutcomePayload::ok("exit", None, "session ended"),
            DispatchControl::Exit,
        ),
    }
}

/// Mode-aware dispatch (Plan 18 ASP prod mode / C0). Wraps
/// [`dispatch`] with a per-mode prelude:
///
/// - `Mode::Dev` is a pass-through to the historical dispatch.
/// - `Mode::Prod` rejects every verb outside the prod allow-list
///   ([`PROD_ALLOWED_VERBS`]) with `OutcomePayload::unsupported_verb_in_prod`
///   and `DispatchControl::Continue` (so the session stays open —
///   a misbehaving client can keep trying allowed verbs without
///   being kicked off the transport).
pub fn dispatch_with_mode(
    verb: &Verb,
    runtime: &mut Runtime,
    session: &mut Session,
    mode: Mode,
) -> (OutcomePayload, DispatchControl) {
    if mode == Mode::Prod && !is_prod_allowed(verb) {
        let allowed = PROD_ALLOWED_VERBS.join(", ");
        let payload = OutcomePayload::unsupported_verb_in_prod(
            verb_name(verb),
            &format!("verb not available in production mode (allowed: {allowed})"),
        );
        return (payload, DispatchControl::Continue);
    }
    dispatch(verb, runtime, session)
}

/// Maximum `limit` accepted by `list_actions`. Spec §12 says 1000;
/// pinned here so C1's projection inherits the cap without
/// re-reading the doc.
pub const LIST_ACTIONS_MAX_LIMIT: u32 = 1000;

/// `list_actions` handler (Plan 18 ASP prod mode / C0).
///
/// **Today (C0):** returns an empty action list after validating
/// pagination input. The real projection from the runtime's
/// interactive nodes lands in C1 alongside `jian-action-surface`
/// integration. The empty projection is intentional, not a stub:
/// it lets the wire surface stabilize + lets `dispatch_with_mode`'s
/// prod-mode rejection be tested end-to-end before the action-
/// derivation logic introduces its own surface area.
///
/// Validation enforced today (codex round 1 MEDIUM):
/// - `limit == 0` → `Invalid` ("limit must be > 0").
/// - `limit > LIST_ACTIONS_MAX_LIMIT` → `Invalid` ("limit exceeds
///   max"). Surface the cap now so C1 inherits it.
/// - Any non-empty `cursor` → `Invalid` ("invalid cursor"). C0
///   never issues cursors, so any value the client supplies is by
///   definition stale or fabricated.
fn run_list_actions(
    _runtime: &Runtime,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> OutcomePayload {
    if let Some(0) = limit {
        return OutcomePayload::invalid("list_actions", "limit must be > 0");
    }
    if let Some(n) = limit {
        if n > LIST_ACTIONS_MAX_LIMIT {
            return OutcomePayload::invalid(
                "list_actions",
                &format!("limit {n} exceeds max ({LIST_ACTIONS_MAX_LIMIT})"),
            );
        }
    }
    if cursor.is_some_and(|c| !c.is_empty()) {
        return OutcomePayload::invalid("list_actions", "invalid cursor");
    }
    OutcomePayload::ok("list_actions", None, "0 actions").with_detail(DetailKind::ActionList {
        actions: Vec::new(),
        next_cursor: None,
    })
}

/// Stable name used in `OutcomePayload.verb` / audit ring entries.
pub fn verb_name(verb: &Verb) -> &'static str {
    match verb {
        Verb::Handshake { .. } => "handshake",
        Verb::Find { .. } => "find",
        Verb::Tap { .. } => "tap",
        Verb::Type { .. } => "type",
        Verb::Scroll { .. } => "scroll",
        Verb::Swipe { .. } => "swipe",
        Verb::Navigate { .. } => "navigate",
        Verb::WaitFor { .. } => "wait_for",
        Verb::Assert { .. } => "assert",
        Verb::Inspect { .. } => "inspect",
        Verb::Snapshot { .. } => "snapshot",
        Verb::SetState { .. } => "set_state",
        Verb::Audit { .. } => "audit",
        Verb::ListActions { .. } => "list_actions",
        Verb::Exit => "exit",
    }
}

fn run_find(runtime: &Runtime, sel: &Selector, limit: Option<u32>) -> OutcomePayload {
    let Some(doc) = runtime.document.as_ref() else {
        return OutcomePayload::error("find", "no document loaded");
    };
    let hits = match sel.resolve(&doc.tree) {
        Ok(h) => h,
        Err(e) => return OutcomePayload::invalid("find", &format!("{}", e)),
    };
    let cap = limit.map(|n| n as usize).unwrap_or(usize::MAX);
    let summaries = collect_node_summaries(doc, &hits, runtime, cap);
    let n = summaries.len();
    if hits.is_empty() {
        return OutcomePayload::not_found("find", "selector matched zero nodes");
    }
    // `limit: 0` (or any cap that strands the resolver's matches)
    // post-filters every summary out — surface that as `invalid`
    // rather than a misleading `ok` with an empty payload.
    if summaries.is_empty() {
        return OutcomePayload::invalid(
            "find",
            "limit truncated all matches; use limit > 0 or omit the field",
        );
    }
    // For now `find` reports the first match's summary as the
    // structured detail — the common case is "find a button, then
    // tap it" where the agent only needs the first id. Multi-match
    // callers read the `narrative` count and re-issue `inspect`
    // with the next index. Phase 2.5 may switch this to a
    // `DetailKind::NodeList` so the agent gets every summary in
    // one round-trip.
    let first = summaries.into_iter().next().unwrap_or(NodeSummary {
        id: String::new(),
        role: None,
        text: None,
        visible: true,
        rect: [0.0; 4],
    });
    OutcomePayload::ok("find", Some(first.id.clone()), format!("{} matches", n))
        .with_detail(DetailKind::Node { node: first })
}

fn run_inspect(runtime: &Runtime, sel: Option<&Selector>, what: InspectKind) -> OutcomePayload {
    let Some(doc) = runtime.document.as_ref() else {
        return OutcomePayload::error("inspect", "no document loaded");
    };
    match what {
        InspectKind::NodeProps => {
            let Some(sel) = sel else {
                return OutcomePayload::invalid("inspect", "what=node_props requires a selector");
            };
            let hits = match sel.resolve(&doc.tree) {
                Ok(h) => h,
                Err(e) => return OutcomePayload::invalid("inspect", &format!("{}", e)),
            };
            let summaries = collect_node_summaries(doc, &hits, runtime, 1);
            let Some(first) = summaries.into_iter().next() else {
                return OutcomePayload::not_found("inspect", "selector matched zero nodes");
            };
            OutcomePayload::ok(
                "inspect",
                Some(first.id.clone()),
                format!("inspected node `{}`", first.id),
            )
            .with_detail(DetailKind::Node { node: first })
        }
        InspectKind::Route => {
            // The runtime's `nav` service exposes the current
            // route + stack via the `Router` trait; project the
            // bits an LLM agent typically reasons over (path,
            // stack depth, params) into the `State`-shaped detail
            // so the wire format stays one canonical
            // `{"kind":"state","entries":{...}}` shape across
            // inspect kinds.
            let route = runtime.nav.current();
            let mut entries = std::collections::BTreeMap::new();
            entries.insert("path".into(), serde_json::Value::String(route.path));
            entries.insert(
                "stack_depth".into(),
                serde_json::Value::Number((route.stack.len() as u64).into()),
            );
            if !route.params.is_empty() {
                entries.insert(
                    "params".into(),
                    serde_json::to_value(&route.params).unwrap_or(serde_json::Value::Null),
                );
            }
            if !route.query.is_empty() {
                entries.insert(
                    "query".into(),
                    serde_json::to_value(&route.query).unwrap_or(serde_json::Value::Null),
                );
            }
            OutcomePayload::ok("inspect", None, "route inspected")
                .with_detail(DetailKind::State { entries })
        }
        InspectKind::State => {
            // The agent's selector becomes a scope discriminator
            // here: `selector.id` is interpreted as the scope
            // name (`$app` / `$vars`). Phase 3.5 may give
            // `inspect what=state` a richer parameter shape;
            // this scope-via-id pattern keeps the wire surface
            // backward-compatible until then.
            let scope = sel.and_then(|s| s.id.as_deref()).unwrap_or("$app");
            run_inspect_state(runtime, scope)
        }
        InspectKind::AxTree => ax_verb::run_inspect_ax_tree(runtime),
    }
}
