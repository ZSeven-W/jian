//! Byte/token budget benchmark for ASP prod (Plan 18 ASP prod mode
//! / C6 / spec §10 acceptance gate).
//!
//! Quantifies the prod-mode token-savings claim from spec §1: ASP
//! prod is "~4-8× cheaper per session than MCP" because the wire
//! shape of `list_actions` is a flat `[{id, events}]` projection
//! and MCP's `tools/list` shape carries `{name, description,
//! params_schema, returns_schema, status}` per action.
//!
//! The test:
//! 1. Builds a synthetic 50-action document.
//! 2. Renders the MCP-shaped response body (exact field set the
//!    `ListResponse` struct in `jian-action-surface::list` emits —
//!    duplicated as a string-builder here because `jian-asp`
//!    doesn't link `jian-action-surface` and we want the test to
//!    run under `cargo test -p jian-asp --features prod-asp`).
//! 3. Renders the ASP `list_actions` response body via the live
//!    dispatcher.
//! 4. Compares byte counts and asserts ASP prod is at least 3×
//!    smaller (the spec's lower bound on the savings claim — a
//!    tighter assertion would be brittle against future doc-shape
//!    changes; this test is a regression guard, not a fixed
//!    measurement).
//!
//! Run with `--nocapture` to see the actual byte counts:
//! `cargo test -p jian-asp --features prod-asp byte_budget -- --nocapture`

#![cfg(feature = "prod-asp")]

use jian_asp::protocol::Verb;
use jian_asp::session::{Permission, Session};
use jian_asp::verb_impls::{dispatch_with_mode, Mode};
use jian_core::action_surface::{derive_actions, BUILD_SALT};
use jian_core::Runtime;
use jian_ops_schema::document::PenDocument;

const TARGET_ACTIONS: usize = 50;

fn rt_with(doc_json: &str) -> Runtime {
    let schema: PenDocument = jian_ops_schema::load_str(doc_json).unwrap().value;
    let mut rt = Runtime::new_from_document(schema).unwrap();
    rt.build_layout((480.0, 9999.0)).unwrap();
    rt.rebuild_spatial();
    rt
}

/// Build a 50-action `.op` schema. Each action is a tappable
/// rectangle whose id ends in a 4-digit sequence so the slug
/// derivation is unique.
fn fifty_action_doc() -> String {
    let mut children = String::new();
    for i in 0..TARGET_ACTIONS {
        if i > 0 {
            children.push(',');
        }
        children.push_str(&format!(
            r##"{{"type":"rectangle","id":"btn-{:04}","x":0,"y":{},"width":100,"height":40,"events":{{"onTap":[{{"set":{{"$app.count":"$app.count + 1"}}}}]}},"semantics":{{"role":"button","label":"Button {}","aiDescription":"button {} — increments the counter"}}}}"##,
            i,
            i * 50,
            i,
            i,
        ));
    }
    format!(
        r##"{{
          "formatVersion":"1.0","version":"1.0.0","id":"bench",
          "app":{{"name":"bench","version":"1","id":"bench","capabilities":["storage"]}},
          "state":{{"count":{{"type":"int","default":0}}}},
          "children":[
            {{"type":"frame","id":"root","width":480,"height":9999,"x":0,"y":0,
              "children":[{}]}}
          ]
        }}"##,
        children
    )
}

/// Render the MCP-shaped `list_available_actions` response body.
/// Field set is `{actions: [{name, description, params_schema,
/// returns_schema}], total}` per `jian-action-surface::list::ListResponse`
/// (we omit `page` when `None` to match `skip_serializing_if`). The
/// `tools/list` envelope MCP wraps this in is constant-overhead per
/// session; the per-page comparison here is the steady-state cost.
fn render_mcp_list(actions: &[jian_core::action_surface::ActionDefinition]) -> String {
    use serde_json::{json, Value};
    let rows: Vec<Value> = actions
        .iter()
        .filter(|a| {
            matches!(
                a.status,
                jian_core::action_surface::AvailabilityStatic::Available
            )
        })
        .map(|a| {
            json!({
                "name": a.full_name(),
                "description": a.description,
                "params_schema": empty_params_schema(),
                "returns_schema": empty_returns_schema(),
            })
        })
        .collect();
    let total = rows.len();
    json!({
        "actions": rows,
        "total": total,
    })
    .to_string()
}

fn empty_params_schema() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}, "required": []})
}

fn empty_returns_schema() -> serde_json::Value {
    serde_json::json!({"ok": "boolean"})
}

/// Render an ASP `list_actions` response body in the requested
/// mode. The full `OutcomePayload` JSON the wire emits, including
/// the flat `ActionList` detail. Includes the response envelope so
/// we're comparing apples-to-apples against MCP's response body.
///
/// `Mode::Dev` and `Mode::Prod` produce byte-identical output for
/// `list_actions` because the projection logic is the same in both
/// modes (spec §7's "portable client" migration policy: dev MUST
/// support `list_actions` additively). The two-tier ratio is a
/// property of the *protocol envelope shape*, not the dispatch
/// mode — the bench renders both rows so the §10-bullet-10
/// "three-tier comparison" is visible at the same time.
fn render_asp_body(rt: &mut Runtime, mode: Mode) -> String {
    let mut session = Session::new(Permission::Observe, "test", "0.1");
    let (out, _) = dispatch_with_mode(
        &Verb::ListActions {
            cursor: None,
            limit: Some(1000),
        },
        rt,
        &mut session,
        mode,
    );
    assert!(out.ok, "list_actions should succeed");
    serde_json::to_string(&out).unwrap()
}

#[test]
fn prod_asp_is_at_least_3x_smaller_than_mcp_on_50_action_screen() {
    let mut rt = rt_with(&fifty_action_doc());
    let doc_ref = rt.document.as_ref().unwrap();
    let actions = derive_actions(&doc_ref.schema, &BUILD_SALT);
    assert!(
        actions.len() >= TARGET_ACTIONS,
        "fixture should derive at least {TARGET_ACTIONS} actions, got {}",
        actions.len()
    );

    let mcp_body = render_mcp_list(&actions);
    let asp_dev_body = render_asp_body(&mut rt, Mode::Dev);
    let asp_prod_body = render_asp_body(&mut rt, Mode::Prod);

    let mcp_bytes = mcp_body.len();
    let asp_dev_bytes = asp_dev_body.len();
    let asp_prod_bytes = asp_prod_body.len();
    let ratio = mcp_bytes as f64 / asp_prod_bytes as f64;

    // Visible to `cargo test -- --nocapture`. Surfaces the actual
    // numbers as a record artifact for the spec §10 acceptance
    // gate ("benchmarks show the three-tier token comparison").
    println!("--- list_actions byte budget on a {TARGET_ACTIONS}-action screen ---");
    println!("MCP     tools/list_available_actions response body : {mcp_bytes:>6} bytes");
    println!("ASP dev list_actions response body                 : {asp_dev_bytes:>6} bytes");
    println!("ASP prd list_actions response body                 : {asp_prod_bytes:>6} bytes");
    println!("ratio (mcp / asp prod)                             : {ratio:>6.2}×");
    if asp_dev_bytes == asp_prod_bytes {
        println!(
            "(dev == prod for list_actions: portable-client guarantee per spec §7)"
        );
    }

    // Spec §1 claims "~4-8×". Assert the looser 3× lower-bound so
    // routine field churn (e.g. an additional event tag) doesn't
    // break the regression guard.
    assert!(
        ratio >= 3.0,
        "expected ASP prod to be at least 3× smaller than MCP for {} actions; \
         got ratio {:.2}× (mcp={}, asp_prod={})",
        TARGET_ACTIONS,
        ratio,
        mcp_bytes,
        asp_prod_bytes
    );
    // Spec §7's "portable-client" promise: dev list_actions returns
    // the same projection prod does. If they ever diverge, an agent
    // that worked in dev would silently break in prod (or vice
    // versa); pin the byte-equality here so a future dev-mode
    // sidecar ("list_actions_with_tree" or similar) doesn't slip
    // in unobserved.
    assert_eq!(
        asp_dev_bytes, asp_prod_bytes,
        "portable-client invariant: list_actions wire bytes must match between \
         Mode::Dev and Mode::Prod (got dev={} prod={})",
        asp_dev_bytes, asp_prod_bytes
    );
}
