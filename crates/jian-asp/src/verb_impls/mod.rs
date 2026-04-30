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

use crate::protocol::{
    DetailKind, InspectKind, NodeSummary, OutcomePayload, Verb,
};
use crate::selector::Selector;
use crate::session::{Permission, Session};
use jian_core::Runtime;

#[cfg(feature = "dev-asp")]
pub mod expr_verb;
#[cfg(feature = "dev-asp")]
pub mod find_verb;
#[cfg(feature = "dev-asp")]
pub mod state_verb;
#[cfg(feature = "dev-asp")]
pub mod tap_verb;

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
        let (out, _) = dispatch(
            &Verb::Audit { last_n: Some(5) },
            &mut rt,
            &mut session,
        );
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
    fn dispatch_unimplemented_verb_returns_error() {
        // `snapshot` is still in the not-yet-impl arm — needs a
        // PNG encoder + text-tree formatter that the runtime
        // doesn't expose yet. Pin the placeholder so the agent
        // sees a clear error rather than a silent ok.
        let mut rt = make_runtime_with_doc(fixture_doc());
        let mut session = Session::new(Permission::Full, "test", "0.1");
        let (out, _) = dispatch(
            &Verb::Snapshot { format: None },
            &mut rt,
            &mut session,
        );
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("RuntimeError"));
        assert!(out.narrative.contains("not yet implemented"));
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
        Verb::Find { selector, limit } => {
            (run_find(runtime, selector, *limit), DispatchControl::Continue)
        }
        Verb::Tap { selector } => (run_tap(runtime, selector), DispatchControl::Continue),
        Verb::Navigate { path, mode } => {
            (run_navigate(runtime, path, *mode), DispatchControl::Continue)
        }
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
        Verb::Inspect { selector, what } => {
            (run_inspect(runtime, selector.as_ref(), *what), DispatchControl::Continue)
        }
        Verb::Audit { last_n } => {
            let n = last_n.unwrap_or(32) as usize;
            let entries = session.audit_tail(n);
            let outcome = OutcomePayload::ok("audit", None, format!("{} entries", entries.len()))
                .with_detail(DetailKind::Audit { entries });
            (outcome, DispatchControl::Continue)
        }
        Verb::Exit => (
            OutcomePayload::ok("exit", None, "session ended"),
            DispatchControl::Exit,
        ),
        // The "not yet implemented" verbs. Phase 3 fills each
        // with the runtime integration work that PR-sized
        // commits track separately.
        _ => (
            OutcomePayload::error(
                verb_name(verb),
                "not yet implemented in this Phase 2 build",
            ),
            DispatchControl::Continue,
        ),
    }
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

fn run_inspect(
    runtime: &Runtime,
    sel: Option<&Selector>,
    what: InspectKind,
) -> OutcomePayload {
    let Some(doc) = runtime.document.as_ref() else {
        return OutcomePayload::error("inspect", "no document loaded");
    };
    match what {
        InspectKind::NodeProps => {
            let Some(sel) = sel else {
                return OutcomePayload::invalid(
                    "inspect",
                    "what=node_props requires a selector",
                );
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
            let scope = sel
                .and_then(|s| s.id.as_deref())
                .unwrap_or("$app");
            run_inspect_state(runtime, scope)
        }
        // `ax_tree` needs an a11y-tree generator that doesn't
        // exist yet. Defer to Phase 4.
        _ => OutcomePayload::error(
            "inspect",
            "inspect what=ax_tree not yet implemented",
        ),
    }
}
