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

use crate::protocol::{DetailKind, OutcomePayload, Verb};
#[cfg(feature = "dev-asp")]
use crate::protocol::{InspectKind, NodeSummary};
#[cfg(feature = "dev-asp")]
use crate::selector::Selector;
use crate::session::{Permission, Session};
use jian_core::Runtime;

// Plan 18 ASP prod mode / C3 — module-level feature gates.
//
// Always-on under either ASP feature:
// - `node_helpers`: shared node-summary helpers (`role_for`,
//   `visible_text`, `node_is_statically_visible`,
//   `collect_node_summaries`). Used by both prod operation verbs
//   and dev structural verbs.
// - `list_actions`: prod's only discovery verb — must be reachable
//   in both modes per spec §7's portable-client policy.
// - Operation verbs: `tap_verb` / `type_verb` / `scroll_verb` /
//   `swipe_verb`. Available in prod (target by id, C5 will
//   tighten selectors) and in dev.
//
// `dev-asp`-only:
// - `ax_verb`, `snapshot_verb` — structural readers prod drops.
// - `state_verb`, `expr_verb` — direct state writes / expression
//   evaluation; not part of the prod surface.
//
// Note: `find_verb` has no module-level body of its own — the
// dispatch handler `run_find` lives in this `mod.rs` and reaches
// into `node_helpers`. The previous `find_verb.rs` was renamed to
// `node_helpers.rs` in C3 (it was always helpers-only).
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod list_actions;
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod node_helpers;
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod prod_op_guard;
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod scroll_verb;
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod swipe_verb;
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod tap_verb;
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod type_verb;

#[cfg(feature = "dev-asp")]
pub mod ax_verb;
#[cfg(feature = "dev-asp")]
pub mod expr_verb;
#[cfg(feature = "dev-asp")]
pub mod snapshot_verb;
#[cfg(feature = "dev-asp")]
pub mod state_verb;

#[cfg(feature = "dev-asp")]
pub use expr_verb::{run_assert, run_wait_for};
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub use node_helpers::collect_node_summaries;
#[cfg(feature = "dev-asp")]
pub use state_verb::{run_inspect_state, run_navigate, run_set_state};
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub use tap_verb::run_tap;

// Most tests reach into dev-asp-only handlers (find/inspect/etc.)
// — gating the test mod to `dev-asp` keeps the prod-only build's
// `cargo test -p jian-asp --features prod-asp` tractable. Prod-
// specific tests (list_actions projection / Mode::Prod dispatch
// rejection) already pass under dev-asp because dev-asp is a
// superset; a dedicated prod-only test mod can land in C6 if we
// want CI coverage for the lean build.
#[cfg(all(test, feature = "dev-asp"))]
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
    fn dispatch_list_actions_returns_empty_action_list_for_doc_with_no_handlers() {
        // The fixture doc has a rectangle + text label but no
        // `events.onTap` / `bind:value` / `route:` etc., so the
        // derived action set is empty and `list_actions` returns a
        // well-formed empty page (C1 keeps C0's stub-shape contract
        // for the no-actions case).
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
                assert!(
                    actions.is_empty(),
                    "fixture has no event handlers → empty projection"
                );
                assert!(next_cursor.is_none());
            }
            other => panic!("expected ActionList detail, got {:?}", other),
        }
    }

    fn fixture_doc_with_actions() -> &'static str {
        // Shape: a frame containing a tap-able button, a text-input
        // with `bind:value`, and a "delete" rectangle with a
        // confirm-gated handler (must drop out of the projection).
        r##"{
          "formatVersion": "1.0", "version": "1.0.0", "id": "fa",
          "app": { "name": "fa", "version": "1", "id": "fa" },
          "children": [
            {
              "type": "frame", "id": "root", "width": 480, "height": 320, "x": 0, "y": 0,
              "children": [
                { "type": "rectangle", "id": "save-btn", "x": 100, "y": 200, "width": 100, "height": 40,
                  "events": { "onTap": [{ "set": { "$app.saved": "true" } }] } },
                { "type": "text_input", "id": "email", "x": 0, "y": 0, "width": 200, "height": 30,
                  "bindings": { "bind:value": "$state.email" } }
              ]
            }
          ],
          "state": { "saved": { "type": "bool", "default": false }, "email": { "type": "string", "default": "" } }
        }"##
    }

    #[test]
    fn dispatch_list_actions_projects_jian_action_surface_ids() {
        // Plan 18 §C1: `list_actions` returns the same
        // `scope.verb_slug_hash4` ids `jian-action-surface` emits.
        // Two actions in the fixture: tap on save-btn + set on email.
        let mut rt = make_runtime_with_doc(fixture_doc_with_actions());
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out, _) = dispatch(
            &Verb::ListActions {
                cursor: None,
                limit: None,
            },
            &mut rt,
            &mut session,
        );
        assert!(out.ok, "expected ok, got: {:?}", out);
        match out.detail {
            Some(DetailKind::ActionList { actions, .. }) => {
                assert!(!actions.is_empty(), "expected non-empty projection");
                // Every id must follow the scope.slug_hash4 shape.
                for row in &actions {
                    assert!(
                        row.id.contains('.'),
                        "id {:?} should follow scope.slug shape",
                        row.id
                    );
                    assert!(!row.events.is_empty());
                }
                // Hex hash4 suffix → ids end with `_<4 hex chars>`.
                // We assert at least one tap event and at least one set event.
                assert!(
                    actions.iter().any(|r| r.events.iter().any(|e| e == "tap")),
                    "expected at least one tap event in {actions:?}"
                );
                assert!(
                    actions.iter().any(|r| r.events.iter().any(|e| e == "set")),
                    "expected at least one set event in {actions:?}"
                );
            }
            other => panic!("expected ActionList, got {:?}", other),
        }
    }

    #[test]
    fn dispatch_list_actions_drops_actions_under_aihidden_ancestor() {
        // Plan 18 §3 / C2 end-to-end: an action whose source node
        // sits inside an aiHidden subtree must not appear in
        // list_actions, even when the source node itself isn't
        // flagged aiHidden directly. `derive_actions` already drops
        // node-level aiHidden as StaticHidden; this test pins the
        // ancestor-walking gap project_actions_with_doc closes.
        let doc = r##"{
          "formatVersion": "1.0", "version": "1.0.0", "id": "h",
          "app": { "name": "h", "version": "1", "id": "h" },
          "children": [
            { "type": "frame", "id": "private", "x": 0, "y": 0, "width": 200, "height": 100,
              "semantics": { "aiHidden": true },
              "children": [
                { "type": "rectangle", "id": "private-btn", "x": 0, "y": 0, "width": 100, "height": 40,
                  "events": { "onTap": [{ "set": { "$state.x": "1" } }] } }
              ]
            },
            { "type": "rectangle", "id": "public-btn", "x": 0, "y": 120, "width": 100, "height": 40,
              "events": { "onTap": [{ "set": { "$state.y": "1" } }] } }
          ],
          "state": { "x": { "type": "int", "default": 0 }, "y": { "type": "int", "default": 0 } }
        }"##;
        let mut rt = make_runtime_with_doc(doc);
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out, _) = dispatch(
            &Verb::ListActions {
                cursor: None,
                limit: None,
            },
            &mut rt,
            &mut session,
        );
        match out.detail {
            Some(DetailKind::ActionList { actions, .. }) => {
                let ids: Vec<_> = actions.iter().map(|a| a.id.clone()).collect();
                assert!(
                    ids.iter().any(|id| id.contains("public_btn")),
                    "expected public-btn in {ids:?}"
                );
                assert!(
                    !ids.iter().any(|id| id.contains("private_btn")),
                    "private-btn under aiHidden ancestor should NOT appear: {ids:?}"
                );
            }
            other => panic!("expected ActionList, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_list_actions_respects_pagination() {
        // Build a 250-action fixture by replicating buttons; pin
        // that `limit: 100` returns 100 rows + a non-None cursor,
        // and a follow-up call with that cursor returns the next
        // page deterministically.
        let mut buttons = String::new();
        for i in 0..250 {
            if i > 0 {
                buttons.push(',');
            }
            buttons.push_str(&format!(
                r#"{{ "type":"rectangle","id":"btn{i}","x":0,"y":{},"width":50,"height":20,
                "events": {{ "onTap": [{{ "set": {{ "$app.n": "$app.n + 1" }} }}] }} }}"#,
                i * 25
            ));
        }
        let doc = format!(
            r##"{{
              "formatVersion":"1.0","version":"1.0.0","id":"big",
              "app":{{"name":"big","version":"1","id":"big"}},
              "state":{{"n":{{"type":"int","default":0}}}},
              "children":[
                {{"type":"frame","id":"root","width":480,"height":6500,"x":0,"y":0,
                "children":[{buttons}]}}
              ]
            }}"##
        );
        let mut rt = make_runtime_with_doc(&doc);
        let mut session = Session::new(Permission::Observe, "test", "0.1");
        let (out1, _) = dispatch(
            &Verb::ListActions {
                cursor: None,
                limit: Some(100),
            },
            &mut rt,
            &mut session,
        );
        let (page1, next1) = match out1.detail {
            Some(DetailKind::ActionList {
                actions,
                next_cursor,
            }) => (actions, next_cursor),
            other => panic!("expected ActionList, got {other:?}"),
        };
        assert_eq!(page1.len(), 100);
        let next1 = next1.expect("250 > 100 → cursor must be Some");
        let (out2, _) = dispatch(
            &Verb::ListActions {
                cursor: Some(next1),
                limit: Some(100),
            },
            &mut rt,
            &mut session,
        );
        let (page2, next2) = match out2.detail {
            Some(DetailKind::ActionList {
                actions,
                next_cursor,
            }) => (actions, next_cursor),
            other => panic!("expected ActionList, got {other:?}"),
        };
        assert_eq!(page2.len(), 100);
        // Page 2 is disjoint from page 1.
        let p1: std::collections::HashSet<_> = page1.iter().map(|r| r.id.clone()).collect();
        let p2: std::collections::HashSet<_> = page2.iter().map(|r| r.id.clone()).collect();
        assert!(p1.is_disjoint(&p2));
        assert!(next2.is_some(), "still 50 more rows after page 2");
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
        Verb::Find {
            selector: _selector,
            limit: _limit,
        } => {
            #[cfg(feature = "dev-asp")]
            let payload = run_find(runtime, _selector, *_limit);
            #[cfg(not(feature = "dev-asp"))]
            let payload = unsupported_in_prod_build("find");
            (payload, DispatchControl::Continue)
        }
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
        Verb::Snapshot { format: _format } => {
            #[cfg(feature = "dev-asp")]
            let payload = snapshot_verb::run_snapshot(runtime, *_format);
            #[cfg(not(feature = "dev-asp"))]
            let payload = unsupported_in_prod_build("snapshot");
            (payload, DispatchControl::Continue)
        }
        Verb::Navigate {
            path: _path,
            mode: _mode,
        } => {
            #[cfg(feature = "dev-asp")]
            let payload = run_navigate(runtime, _path, *_mode);
            #[cfg(not(feature = "dev-asp"))]
            let payload = unsupported_in_prod_build("navigate");
            (payload, DispatchControl::Continue)
        }
        Verb::SetState {
            scope: _scope,
            key: _key,
            value_json: _value,
        } => {
            #[cfg(feature = "dev-asp")]
            let payload = run_set_state(runtime, _scope, _key, _value);
            #[cfg(not(feature = "dev-asp"))]
            let payload = unsupported_in_prod_build("set_state");
            (payload, DispatchControl::Continue)
        }
        Verb::Assert { expr: _expr } => {
            #[cfg(feature = "dev-asp")]
            let payload = run_assert(runtime, _expr);
            #[cfg(not(feature = "dev-asp"))]
            let payload = unsupported_in_prod_build("assert");
            (payload, DispatchControl::Continue)
        }
        Verb::WaitFor {
            expr: _expr,
            timeout_ms: _timeout_ms,
        } => {
            #[cfg(feature = "dev-asp")]
            let payload = run_wait_for(runtime, _expr, *_timeout_ms);
            #[cfg(not(feature = "dev-asp"))]
            let payload = unsupported_in_prod_build("wait_for");
            (payload, DispatchControl::Continue)
        }
        Verb::Inspect {
            selector: _selector,
            what: _what,
        } => {
            #[cfg(feature = "dev-asp")]
            let payload = run_inspect(runtime, _selector.as_ref(), *_what);
            #[cfg(not(feature = "dev-asp"))]
            let payload = unsupported_in_prod_build("inspect");
            (payload, DispatchControl::Continue)
        }
        Verb::Audit { last_n: _last_n } => {
            #[cfg(feature = "dev-asp")]
            let payload = {
                let n = _last_n.unwrap_or(32) as usize;
                let entries = session.audit_tail(n);
                OutcomePayload::ok("audit", None, format!("{} entries", entries.len()))
                    .with_detail(DetailKind::Audit { entries })
            };
            #[cfg(not(feature = "dev-asp"))]
            let payload = unsupported_in_prod_build("audit");
            (payload, DispatchControl::Continue)
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
    if mode == Mode::Prod {
        // Plan 18 ASP prod mode / C5: narrow op-verb selectors to
        // `list_actions` ids and rewrite to source-node-id selectors
        // before dispatch. Pass-through for non-op verbs.
        match prod_op_guard::rewrite_op_verb_for_prod(verb, runtime) {
            Ok(Some(rewritten)) => return dispatch(&rewritten, runtime, session),
            Ok(None) => { /* not an op verb — fall through to plain dispatch */ }
            Err(payload) => return (payload, DispatchControl::Continue),
        }
    }
    dispatch(verb, runtime, session)
}

/// Maximum `limit` accepted by `list_actions`. Spec §12 says 1000;
/// pinned here so the projection in [`list_actions`] inherits the
/// cap without re-reading the doc.
pub const LIST_ACTIONS_MAX_LIMIT: u32 = 1000;

/// Build-time prod-only guard — used by [`dispatch`] arms when the
/// crate is compiled WITHOUT the `dev-asp` feature, i.e. as a
/// `prod-asp` only build (Plan 18 ASP prod mode / C3). The
/// `Mode::Prod` runtime check in [`dispatch_with_mode`] already
/// returns the same payload before any handler runs, so reaching
/// this branch via the bare [`dispatch`] entry point on a prod-asp
/// build means the host called the wrong API. The error tag is the
/// same `UnsupportedVerbInProd` so a client branch on `error` is
/// stable across build configurations.
#[cfg(not(feature = "dev-asp"))]
fn unsupported_in_prod_build(verb: &'static str) -> OutcomePayload {
    OutcomePayload::unsupported_verb_in_prod(
        verb,
        "verb not compiled in this build (build with `dev-asp` feature \
         to enable structural verbs)",
    )
}

/// `list_actions` handler (Plan 18 ASP prod mode / C0 + C1).
///
/// C0 stubbed this to an empty array; C1 wires the real projection
/// off [`jian_core::action_surface::derive_actions`], reusing the
/// same `<scope>.<verb-prefix-slug>_<hash4>` ids `jian-action-surface`
/// emits over MCP so a single agent client can switch transport
/// without re-learning ids.
///
/// Validation enforced (codex round 1 MEDIUM):
/// - `limit == 0` → `Invalid` ("limit must be > 0").
/// - `limit > LIST_ACTIONS_MAX_LIMIT` → `Invalid` ("limit exceeds
///   max"). Surface the cap explicitly so a client can adjust
///   before re-issuing.
/// - Malformed cursor → `Invalid` ("invalid cursor"). Empty
///   cursor (`Some("")`) is treated as `None` — same as the
///   pagination boundary docs.
///
/// `aiHidden` filtering: `derive_actions` already excludes nodes
/// whose source author flipped `semantics.aiHidden = true`
/// (`AvailabilityStatic::StaticHidden` never reaches the projector).
/// Dynamic state-gating against `bindings.visible` /
/// `bindings.disabled` is C2's job.
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
fn run_list_actions(runtime: &Runtime, cursor: Option<&str>, limit: Option<u32>) -> OutcomePayload {
    use jian_core::action_surface::{derive_actions, BUILD_SALT};
    if let Some(0) = limit {
        return OutcomePayload::invalid("list_actions", "limit must be > 0");
    }
    let limit = limit.unwrap_or(list_actions::LIST_ACTIONS_DEFAULT_LIMIT);
    if limit > LIST_ACTIONS_MAX_LIMIT {
        return OutcomePayload::invalid(
            "list_actions",
            &format!("limit {limit} exceeds max ({LIST_ACTIONS_MAX_LIMIT})"),
        );
    }
    let Some(doc) = runtime.document.as_ref() else {
        // No document loaded — well-formed empty response. The
        // session is still usable; the agent can re-issue once
        // the host hot-reloads or attaches a doc.
        return OutcomePayload::ok("list_actions", None, "0 actions").with_detail(
            DetailKind::ActionList {
                actions: Vec::new(),
                next_cursor: None,
            },
        );
    };
    let derived = derive_actions(&doc.schema, &BUILD_SALT);
    // C2: project_actions_with_doc filters out rows whose source
    // node sits inside an `aiHidden` subtree, not just nodes
    // flagged aiHidden directly (which `derive_actions` already
    // marks `StaticHidden` for the projector to drop).
    let rows = list_actions::project_actions_with_doc(&derived, &doc.schema);
    let total = rows.len();
    let (page, next_cursor) = match list_actions::paginate(rows, cursor, limit) {
        Ok(p) => p,
        Err(msg) => return OutcomePayload::invalid("list_actions", msg),
    };
    let n = page.len();
    OutcomePayload::ok("list_actions", None, format!("{n} of {total} action(s)")).with_detail(
        DetailKind::ActionList {
            actions: page,
            next_cursor,
        },
    )
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

#[cfg(feature = "dev-asp")]
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

#[cfg(feature = "dev-asp")]
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
