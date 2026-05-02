//! Plan 18 ASP prod-mode acceptance tests (C6).
//!
//! Pins the spec §10 acceptance gate from outside the crate so a
//! refactor can't silently slip past:
//!
//! 1. **Structural-leak gate.** `list_actions` rows must contain
//!    exactly `id` + `events` and nothing else. No labels, rects,
//!    roles, hierarchy, schema ids — the threat-model T1 boundary.
//! 2. **Structural-verb refusal.** Prod mode rejects `find` /
//!    `inspect` / `snapshot` / `audit` (and the other dev-only
//!    verbs) with the stable `UnsupportedVerbInProd` error tag,
//!    keeps the session open, and never reaches the verb's
//!    handler.
//!
//! These two also live as in-crate `#[cfg(test)]` cases inside
//! `verb_impls/mod.rs`. The duplication here is deliberate — an
//! external test pins the public wire surface and would catch a
//! visibility regression that an in-crate test wouldn't.

#![cfg(feature = "prod-asp")]

use jian_asp::protocol::{ActionRow, DetailKind, Verb};
use jian_asp::selector::Selector;
use jian_asp::session::{Permission, Session};
use jian_asp::verb_impls::{dispatch_with_mode, Mode};
use jian_core::action_surface::{derive_actions, AvailabilityStatic, BUILD_SALT};
use jian_core::Runtime;
use jian_ops_schema::document::PenDocument;

fn rt_with(doc_json: &str) -> Runtime {
    let schema: PenDocument = jian_ops_schema::load_str(doc_json).unwrap().value;
    let mut rt = Runtime::new_from_document(schema).unwrap();
    rt.build_layout((480.0, 320.0)).unwrap();
    rt.rebuild_spatial();
    rt
}

const COUNTER_DOC: &str = r##"{
  "formatVersion":"1.0","version":"1.0.0","id":"acc",
  "app":{"name":"acc","version":"1","id":"acc","capabilities":["storage"]},
  "state":{"count":{"type":"int","default":0}},
  "children":[
    { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
      "children":[
        { "type":"rectangle","id":"btn","x":100,"y":100,"width":100,"height":40,
          "events":{"onTap":[{"set":{"$app.count":"$app.count + 1"}}]},
          "semantics":{"role":"button","label":"Increment","aiDescription":"increments the counter"}
        }
      ]
    }
  ]
}"##;

/// **Acceptance §10**: prod `list_actions` returns only `id`,
/// `events`, and pagination metadata.
#[test]
fn prod_list_actions_row_has_only_id_and_events() {
    let mut rt = rt_with(COUNTER_DOC);
    let mut session = Session::new(Permission::Observe, "test", "0.1");
    let (out, _) = dispatch_with_mode(
        &Verb::ListActions {
            cursor: None,
            limit: None,
        },
        &mut rt,
        &mut session,
        Mode::Prod,
    );
    assert!(out.ok, "list_actions should succeed; got {:?}", out);
    let actions = match out.detail {
        Some(DetailKind::ActionList { actions, .. }) => actions,
        other => panic!("expected ActionList detail, got {other:?}"),
    };
    assert!(!actions.is_empty(), "fixture has at least one onTap action");

    // Round-trip every row through `serde_json::to_value` so we
    // can inspect the actual wire shape (top-level keys). The
    // gate: each row's serialised object has *only* `id` +
    // `events` keys — no `label`, `role`, `description`,
    // `aiDescription`, `rect`, `node_id`, etc. leaked from the
    // fixture's `semantics` block.
    for (i, row) in actions.iter().enumerate() {
        let v = serde_json::to_value(row).expect("ActionRow serialises");
        let obj = v.as_object().expect("ActionRow is JSON object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["events", "id"],
            "row #{i}: ActionRow leaked structural keys: {keys:?}",
        );
        // Type-level second guard: `ActionRow`'s typed fields are
        // exactly `id` + `events`. If a future field is added to
        // the struct, this destructure forces a compile-error
        // until the test is updated to assert the new field is
        // also non-structural.
        let ActionRow { id, events } = row;
        assert!(!id.is_empty(), "row #{i} id should be non-empty");
        assert!(!events.is_empty(), "row #{i} events should be non-empty");
    }
}

/// **Acceptance §10**: prod rejects `find`, `inspect`, `snapshot`,
/// `audit`. Pinned externally because the in-crate test version is
/// a private-to-`verb_impls` regression check; this one verifies
/// the public dispatch contract AND that no handler side-effect ran
/// before the rejection (codex C6 round 1, MEDIUM 2b).
#[test]
fn prod_dispatch_rejects_each_structural_verb() {
    let mut rt = rt_with(COUNTER_DOC);
    let mut session = Session::new(Permission::Full, "test", "0.1");

    // Snapshot the runtime's mutable state BEFORE running rejected
    // verbs. After every rejection the state must be byte-identical —
    // a buggy prod gate that ran handler side-effects before returning
    // the rejection would mutate state here (e.g. set_state writing,
    // navigate pushing a route).
    let initial_count = rt
        .state
        .app_get("count")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);

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
                what: jian_asp::protocol::InspectKind::AxTree,
            },
            "inspect",
        ),
        (Verb::Snapshot { format: None }, "snapshot"),
        (Verb::Audit { last_n: None }, "audit"),
        // `set_state` is the canary side-effect verb: if a prod
        // gate-bypass let it run, the runtime's `count` would change.
        (
            Verb::SetState {
                scope: "$app".into(),
                key: "count".into(),
                value_json: "999".into(),
            },
            "set_state",
        ),
    ];
    for (verb, expected_name) in cases {
        let (out, _) = dispatch_with_mode(verb, &mut rt, &mut session, Mode::Prod);
        assert!(
            !out.ok,
            "prod must reject `{expected_name}`, got ok response: {out:?}"
        );
        assert_eq!(
            out.error.as_deref(),
            Some("UnsupportedVerbInProd"),
            "verb `{expected_name}` should carry the stable error tag, got {:?}",
            out.error
        );
        assert_eq!(out.verb, *expected_name);
        // Side-effect canary: state must still equal the initial
        // value, proving the handler never ran.
        let now = rt
            .state
            .app_get("count")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        assert_eq!(
            now, initial_count,
            "verb `{expected_name}` rejection must not have mutated state \
             (count went {initial_count}→{now}, indicating the handler ran \
             before the rejection)",
        );
    }
}

/// **Acceptance §10 bullet 9**: dev mode supports `list_actions`
/// additively for portable clients. Codex C6 round 1, MEDIUM gap.
#[test]
fn dev_mode_dispatches_list_actions() {
    let mut rt = rt_with(COUNTER_DOC);
    let mut session = Session::new(Permission::Observe, "test", "0.1");
    let (out, _) = dispatch_with_mode(
        &Verb::ListActions {
            cursor: None,
            limit: None,
        },
        &mut rt,
        &mut session,
        Mode::Dev,
    );
    assert!(
        out.ok,
        "dev mode must support list_actions for portable clients, got {out:?}"
    );
    let actions = match out.detail {
        Some(DetailKind::ActionList { actions, .. }) => actions,
        other => panic!("expected ActionList detail in dev mode, got {other:?}"),
    };
    assert!(
        !actions.is_empty(),
        "dev list_actions should project the same fixture rows prod does"
    );
}

/// **Acceptance §10 bullet 2**: prod op response bodies must not
/// carry `.op` tree structure. Codex C6 round 1 surfaced two
/// concrete leaks — the dev op handlers populate `target` with the
/// schema node id and bake layout-rect coordinates into `narrative`.
/// `dispatch_with_mode(Mode::Prod)` now sanitizes both before
/// returning to the agent.
#[test]
fn prod_tap_response_does_not_leak_node_id_or_coords() {
    let mut rt = rt_with(COUNTER_DOC);
    let mut session = Session::new(Permission::Act, "test", "0.1");

    // Find the action id for the `btn` tap action.
    let doc_ref = rt.document.as_ref().unwrap();
    let actions = derive_actions(&doc_ref.schema, &BUILD_SALT);
    let action = actions
        .iter()
        .find(|a| matches!(a.status, AvailabilityStatic::Available))
        .expect("at least one action");
    let action_id = action.full_name();
    let source_node_id = action.source_node_id.clone();
    let _ = doc_ref;

    let (out, _) = dispatch_with_mode(
        &Verb::Tap {
            selector: Selector {
                id: Some(action_id.clone()),
                ..Default::default()
            },
        },
        &mut rt,
        &mut session,
        Mode::Prod,
    );
    assert!(out.ok, "tap should dispatch, got {out:?}");

    // `target` must be the action id, not the schema node id.
    assert_eq!(
        out.target.as_deref(),
        Some(action_id.as_str()),
        "prod target should be action id, not source_node_id `{}`",
        source_node_id
    );

    // `narrative` must NOT contain the source node id (the dev
    // handler's narrative was `tapped node `<id>` at (X.X, Y.Y)`).
    assert!(
        !out.narrative.contains(&source_node_id),
        "narrative leaked source node id `{}`: {}",
        source_node_id,
        out.narrative
    );
    // `narrative` must NOT contain digits-with-decimal (a coarse
    // proxy for layout-rect coords like `(110.0, 120.0)`). The
    // sanitized narrative is `action `<id>` dispatched`, no coords.
    let has_decimal = out.narrative.chars().enumerate().any(|(i, c)| {
        c == '.'
            && i > 0
            && i + 1 < out.narrative.len()
            && out.narrative.as_bytes()[i - 1].is_ascii_digit()
            && out.narrative.as_bytes()[i + 1].is_ascii_digit()
    });
    assert!(
        !has_decimal,
        "narrative may have leaked layout-rect coords: {}",
        out.narrative
    );
}
