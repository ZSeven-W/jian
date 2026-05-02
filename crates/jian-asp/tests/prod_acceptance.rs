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
/// the public dispatch contract.
#[test]
fn prod_dispatch_rejects_each_structural_verb() {
    let mut rt = rt_with(COUNTER_DOC);
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
                what: jian_asp::protocol::InspectKind::AxTree,
            },
            "inspect",
        ),
        (Verb::Snapshot { format: None }, "snapshot"),
        (Verb::Audit { last_n: None }, "audit"),
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
    }
}
