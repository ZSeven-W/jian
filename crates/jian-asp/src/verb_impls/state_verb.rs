//! `set_state`, `navigate`, `inspect what=state` runtime-coupled
//! verb implementations (Plan 18 Phase 3).

use jian_core::Runtime;
use std::collections::BTreeMap;

use crate::protocol::{DetailKind, NavMode, OutcomePayload};

/// `set_state` verb. Restricted to the `Full` permission tier
/// upstream (the dispatcher gate enforces this); `value_json` is
/// the agent's serialised JSON for the new value, parsed and
/// applied to the named scope's key. Returns one
/// `DeltaEntry` so the agent can confirm the path that changed.
pub fn run_set_state(
    runtime: &mut Runtime,
    scope: &str,
    key: &str,
    value_json: &str,
) -> OutcomePayload {
    let value: serde_json::Value = match serde_json::from_str(value_json) {
        Ok(v) => v,
        Err(e) => {
            return OutcomePayload::invalid(
                "set_state",
                &format!("value_json is not valid JSON: {}", e),
            )
        }
    };
    // Read the previous value so the response includes a
    // before/after delta.
    let path = format!("{}.{}", scope, key);
    let before = read_scope_key(runtime, scope, key)
        .unwrap_or(serde_json::Value::Null);
    match scope {
        "$app" => runtime.state.app_set(key, value.clone()),
        "$vars" => runtime.state.vars_set(key, value.clone()),
        other => {
            return OutcomePayload::invalid(
                "set_state",
                &format!(
                    "scope `{}` is not writable from ASP (only $app / $vars)",
                    other
                ),
            )
        }
    }
    OutcomePayload::ok(
        "set_state",
        Some(path.clone()),
        format!("set {} to JSON value", path),
    )
    .with_delta(path, before, value, Some("asp.set_state".into()))
}

/// `navigate` verb. Routes through the runtime's `nav` service
/// (`Router` trait) — push / replace / pop / reset. The detail
/// includes the new path so the agent can confirm the
/// transition without a round-trip.
pub fn run_navigate(runtime: &mut Runtime, path: &str, mode: Option<NavMode>) -> OutcomePayload {
    let mode = mode.unwrap_or(NavMode::Push);
    let before = runtime.nav.current().path;
    match mode {
        NavMode::Push => runtime.nav.push(path),
        NavMode::Replace => runtime.nav.replace(path),
        NavMode::Pop => runtime.nav.pop(),
        NavMode::Reset => runtime.nav.reset(path),
    }
    let after = runtime.nav.current().path;
    OutcomePayload::ok(
        "navigate",
        Some(after.clone()),
        format!("navigated {:?} → {}", mode, after),
    )
    .with_delta(
        "$route.path",
        serde_json::Value::String(before),
        serde_json::Value::String(after),
        Some(format!("navigate {:?}", mode)),
    )
}

/// `inspect what=state` — return the named scope's key-value
/// snapshot. Phase 3 supports `$app` + `$vars` (the writable
/// scopes); `$route` / `$page` projections live in
/// `inspect what=route` / a future `what=page`.
pub fn run_inspect_state(runtime: &Runtime, scope: &str) -> OutcomePayload {
    let mut entries: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    if let Some(doc) = runtime.document.as_ref() {
        match scope {
            "$app" => {
                if let Some(state_schema) = doc.schema.state.as_ref() {
                    for key in state_schema.keys() {
                        let json = runtime
                            .state
                            .app_get(key)
                            .map(|v| v.0)
                            .unwrap_or(serde_json::Value::Null);
                        entries.insert(key.clone(), json);
                    }
                }
            }
            "$vars" => {
                if let Some(vars_schema) = doc.schema.variables.as_ref() {
                    for key in vars_schema.keys() {
                        let json = runtime
                            .state
                            .vars_get(key)
                            .map(|v| v.0)
                            .unwrap_or(serde_json::Value::Null);
                        entries.insert(key.clone(), json);
                    }
                }
            }
            other => {
                return OutcomePayload::invalid(
                    "inspect",
                    &format!(
                        "inspect what=state with scope `{}` not supported (use $app / $vars)",
                        other
                    ),
                );
            }
        }
    }
    let n = entries.len();
    OutcomePayload::ok("inspect", Some(scope.to_owned()), format!("{} key(s)", n))
        .with_detail(DetailKind::State { entries })
}

fn read_scope_key(runtime: &Runtime, scope: &str, key: &str) -> Option<serde_json::Value> {
    match scope {
        "$app" => runtime.state.app_get(key).map(|v| v.0),
        "$vars" => runtime.state.vars_get(key).map(|v| v.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_ops_schema::document::PenDocument;

    fn rt_with(doc_json: &str) -> Runtime {
        let schema: PenDocument = jian_ops_schema::load_str(doc_json).unwrap().value;
        let mut rt = Runtime::new_from_document(schema).unwrap();
        rt.build_layout((480.0, 320.0)).unwrap();
        rt.rebuild_spatial();
        rt
    }

    fn fixture() -> &'static str {
        r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{"count":{"type":"int","default":0}},
          "vars":{"flag":{"type":"bool","default":false}},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,"children":[] }
          ]
        }"##
    }

    #[test]
    fn set_state_writes_app_scope() {
        let mut rt = rt_with(fixture());
        let out = run_set_state(&mut rt, "$app", "count", "42");
        assert!(out.ok, "got {:?}", out);
        assert_eq!(out.deltas.len(), 1);
        assert_eq!(out.deltas[0].path, "$app.count");
        let count = rt.state.app_get("count").and_then(|v| v.as_i64());
        assert_eq!(count, Some(42));
    }

    #[test]
    fn set_state_writes_vars_scope() {
        let mut rt = rt_with(fixture());
        let out = run_set_state(&mut rt, "$vars", "flag", "true");
        assert!(out.ok);
        let flag = rt
            .state
            .vars_get("flag")
            .and_then(|v| v.0.as_bool());
        assert_eq!(flag, Some(true));
    }

    #[test]
    fn set_state_rejects_invalid_json() {
        let mut rt = rt_with(fixture());
        let out = run_set_state(&mut rt, "$app", "count", "not-json");
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
    }

    #[test]
    fn set_state_rejects_unknown_scope() {
        let mut rt = rt_with(fixture());
        let out = run_set_state(&mut rt, "$nope", "x", "1");
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
    }

    #[test]
    fn navigate_push_returns_ok_with_delta_entry() {
        // The default `Runtime::new_from_document` uses a
        // `NullRouter` placeholder that ignores the call —
        // production hosts install `HistoryRouter`. The verb's
        // contract is "dispatched the call against whatever
        // router the runtime carries"; we can only verify the
        // shape of the response here, not the state mutation.
        // A host integration test (jian-host-desktop) covers
        // the path-changes-after-push case.
        let mut rt = rt_with(fixture());
        let out = run_navigate(&mut rt, "/profile", Some(NavMode::Push));
        assert!(out.ok);
        assert_eq!(out.deltas.len(), 1);
        assert_eq!(out.deltas[0].path, "$route.path");
    }

    #[test]
    fn navigate_default_mode_is_push() {
        // `mode: None` falls through to `NavMode::Push`. The
        // narrative includes the resolved mode for the agent's
        // log.
        let mut rt = rt_with(fixture());
        let out = run_navigate(&mut rt, "/x", None);
        assert!(out.ok);
        assert!(out.narrative.contains("Push"));
    }

    #[test]
    fn inspect_state_returns_app_scope_keys() {
        let mut rt = rt_with(fixture());
        rt.state.app_set("count", serde_json::json!(7));
        let out = run_inspect_state(&rt, "$app");
        assert!(out.ok);
        match out.detail {
            Some(DetailKind::State { entries }) => {
                assert!(entries.contains_key("count"));
                assert_eq!(entries.get("count"), Some(&serde_json::json!(7)));
            }
            other => panic!("expected State detail, got {:?}", other),
        }
    }

    #[test]
    fn inspect_state_rejects_route_scope() {
        let rt = rt_with(fixture());
        let out = run_inspect_state(&rt, "$route");
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
    }
}
