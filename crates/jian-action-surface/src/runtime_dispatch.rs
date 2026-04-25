//! `ActionDispatcher` impl that converts a cleared `execute_action`
//! into a real runtime side effect — synthesised `PointerEvent`,
//! direct state write, or router navigation.
//!
//! The four flavours mirror spec §4.2 #6:
//! - **Pointer-style** (Tap / DoubleTap / LongPress / Submit / Confirm
//!   / Dismiss / Swipe* / Scroll / LoadMore): synthesise a Down + Up
//!   pair at the source node's layout centre and feed them through
//!   `Runtime::dispatch_pointer`. The gesture arena recognises the
//!   tap and the action handler runs through the regular `events.*`
//!   path, so any `set` / `push` / `call` actions the author wrote
//!   fire as if the user had clicked.
//! - **SetValue**: the source node carries a `bindings.bind:value`
//!   targeting `$state.<path>`. We skip the arena and write the
//!   value straight to the state graph — the action surface already
//!   validated the param type, and there is no host-driven gesture
//!   to recognise.
//! - **OpenRoute**: read `route.push` / `route.replace`, substitute
//!   any `:param` placeholders from the call's params, hand off to
//!   the runtime's `Router` service.
//!
//! Anything we can't statically resolve (missing source node, no
//! `bind:value` target, malformed route) surfaces as
//! `ExecuteError::HandlerError` — the surface returns the standard
//! `{ ok: false, error: { kind: "ExecutionFailed", reason: "handler_error" } }`
//! payload and the audit row records the same code.

use crate::error::ExecuteError;
use crate::ActionDispatcher;
use jian_core::action_surface::{ActionDefinition, SourceKind};
use jian_core::geometry::point;
use jian_core::gesture::pointer::{PointerEvent, PointerPhase};
use jian_core::Runtime;
use jian_ops_schema::node::PenNode;
use serde_json::{Map, Value};

/// Wraps `&mut Runtime` so a cleared `execute_action` actually fires
/// through the runtime instead of being swallowed by `SinkDispatcher`.
/// Constructed per `execute` call — the borrow is short-lived so the
/// caller keeps full ownership of the runtime between dispatches.
pub struct RuntimeDispatcher<'a> {
    runtime: &'a mut Runtime,
}

impl<'a> RuntimeDispatcher<'a> {
    pub fn new(runtime: &'a mut Runtime) -> Self {
        Self { runtime }
    }
}

impl<'a> ActionDispatcher for RuntimeDispatcher<'a> {
    fn dispatch(
        &mut self,
        action: &ActionDefinition,
        params: &Map<String, Value>,
    ) -> Result<(), ExecuteError> {
        match action.source_kind {
            SourceKind::SetValue => dispatch_set_value(self.runtime, action, params),
            SourceKind::OpenRoute => dispatch_open_route(self.runtime, action, params),
            _ => dispatch_pointer_tap(self.runtime, action),
        }
    }
}

fn dispatch_pointer_tap(runtime: &mut Runtime, action: &ActionDefinition) -> Result<(), ExecuteError> {
    let key = runtime
        .document
        .as_ref()
        .and_then(|doc| doc.tree.get(&action.source_node_id))
        .ok_or_else(ExecuteError::handler_error)?;
    let rect = runtime
        .layout
        .node_rect(key)
        .ok_or_else(ExecuteError::handler_error)?;
    let cx = rect.min_x() + rect.size.width / 2.0;
    let cy = rect.min_y() + rect.size.height / 2.0;
    let centre = point(cx, cy);
    runtime.dispatch_pointer(PointerEvent::simple(0, PointerPhase::Down, centre));
    runtime.dispatch_pointer(PointerEvent::simple(0, PointerPhase::Up, centre));
    Ok(())
}

fn dispatch_set_value(
    runtime: &mut Runtime,
    action: &ActionDefinition,
    params: &Map<String, Value>,
) -> Result<(), ExecuteError> {
    let path = source_node_state_path(runtime, &action.source_node_id)
        .ok_or_else(ExecuteError::handler_error)?;
    let value = params
        .get("value")
        .cloned()
        .ok_or_else(ExecuteError::handler_error)?;
    runtime.state.app_set(&path, value);
    Ok(())
}

fn dispatch_open_route(
    runtime: &mut Runtime,
    action: &ActionDefinition,
    params: &Map<String, Value>,
) -> Result<(), ExecuteError> {
    let route = source_node_route(runtime, &action.source_node_id)
        .ok_or_else(ExecuteError::handler_error)?;
    let filled = substitute_route_params(&route.template, params);
    match route.kind {
        RouteKind::Push => runtime.nav.push(&filled),
        RouteKind::Replace => runtime.nav.replace(&filled),
    }
    Ok(())
}

/// Strip the `$state.` prefix from the source node's `bindings.bind:value`
/// and return the remaining path. Anything else (route / app params /
/// missing binding) yields `None`, which surfaces as `handler_error`.
fn source_node_state_path(runtime: &Runtime, node_id: &str) -> Option<String> {
    let node = source_node(runtime, node_id)?;
    let json = serde_json::to_value(node).ok()?;
    let raw = json
        .get("bindings")?
        .get("bind:value")?
        .as_str()?
        .trim()
        .strip_prefix("$state.")?
        .to_owned();
    if raw.is_empty() {
        return None;
    }
    Some(raw)
}

struct ResolvedRoute {
    kind: RouteKind,
    template: String,
}

enum RouteKind {
    Push,
    Replace,
}

fn source_node_route(runtime: &Runtime, node_id: &str) -> Option<ResolvedRoute> {
    let node = source_node(runtime, node_id)?;
    let json = serde_json::to_value(node).ok()?;
    let route = json.get("route")?;
    if let Some(s) = route.get("push").and_then(|v| v.as_str()) {
        return Some(ResolvedRoute {
            kind: RouteKind::Push,
            template: s.to_owned(),
        });
    }
    if let Some(s) = route.get("replace").and_then(|v| v.as_str()) {
        return Some(ResolvedRoute {
            kind: RouteKind::Replace,
            template: s.to_owned(),
        });
    }
    None
}

fn source_node<'a>(runtime: &'a Runtime, node_id: &str) -> Option<&'a PenNode> {
    let doc = runtime.document.as_ref()?;
    let key = doc.tree.get(node_id)?;
    doc.tree.nodes.get(key).map(|d| &d.schema)
}

/// Replace `:name` segments in `template` with the corresponding
/// `params[name]` value (stringified). Unknown placeholders pass
/// through unchanged so the router can surface a 404 / not-found
/// rather than us silently dropping the segment.
fn substitute_route_params(template: &str, params: &Map<String, Value>) -> String {
    template
        .split('/')
        .map(|seg| match seg.strip_prefix(':') {
            Some(name) => params
                .get(name)
                .map(value_to_path_segment)
                .unwrap_or_else(|| seg.to_owned()),
            None => seg.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn value_to_path_segment(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn substitute_replaces_named_param() {
        let mut params = Map::new();
        params.insert("id".into(), Value::String("42".into()));
        assert_eq!(substitute_route_params("/detail/:id", &params), "/detail/42");
    }

    #[test]
    fn substitute_keeps_unmatched_param_segment() {
        let params = Map::new();
        assert_eq!(
            substitute_route_params("/detail/:id", &params),
            "/detail/:id"
        );
    }

    #[test]
    fn substitute_handles_no_leading_slash() {
        let mut params = Map::new();
        params.insert("slug".into(), Value::String("about".into()));
        assert_eq!(substitute_route_params(":slug", &params), "about");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ActionSurface;
    use crate::ExecuteOutcome;
    use jian_ops_schema::document::PenDocument;
    use serde_json::json;

    fn build_runtime(src: &str) -> (Runtime, PenDocument) {
        let schema: PenDocument = serde_json::from_str(src).unwrap();
        let mut rt = Runtime::new_from_document(schema.clone()).unwrap();
        rt.build_layout((400.0, 200.0)).unwrap();
        rt.rebuild_spatial();
        (rt, schema)
    }

    #[test]
    fn tap_action_synthesises_pointer_event() {
        let (mut rt, doc) = build_runtime(
            r##"{
              "formatVersion":"1.0","version":"1.0.0",
              "state":{ "count":{ "type":"int", "default":0 } },
              "children":[
                { "type":"rectangle","id":"plus","width":200,"height":50,
                  "fill":[{ "type":"solid","color":"#1e88e5" }],
                  "semantics":{ "aiName":"plus" },
                  "events":{ "onTap": [ { "set": { "$app.count": "$app.count + 1" } } ] }
                }
              ]
            }"##,
        );
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut dispatcher = RuntimeDispatcher::new(&mut rt);
        let out = surface.execute("global.plus", None, &mut dispatcher);
        assert!(matches!(out, ExecuteOutcome::Ok), "outcome={out:?}");
        assert_eq!(rt.state.app_get("count").and_then(|v| v.as_i64()), Some(1));
    }

    #[test]
    fn set_value_writes_state_directly() {
        let (mut rt, doc) = build_runtime(
            r#"{
              "version":"0.8.0",
              "state":{ "email":{ "type":"string", "default":"" } },
              "pages":[{ "id":"signup","name":"Sign up","children":[
                { "type":"text_input","id":"email-input",
                  "semantics":{ "aiName":"email" },
                  "bindings": { "bind:value": "$state.email" }
                }
              ]}],
              "children":[]
            }"#,
        );
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut dispatcher = RuntimeDispatcher::new(&mut rt);
        let out = surface.execute(
            "signup.set_email",
            Some(&json!({ "value": "fini@example.com" })),
            &mut dispatcher,
        );
        assert!(matches!(out, ExecuteOutcome::Ok), "outcome={out:?}");
        assert_eq!(
            rt.state.app_get("email").and_then(|v| v.as_str().map(str::to_owned)),
            Some("fini@example.com".to_owned()),
        );
    }

    #[test]
    fn open_route_substitutes_path_params() {
        use jian_core::action::services::router::{RouteState, Router};
        use std::cell::RefCell;
        use std::collections::BTreeMap;

        // Recording router stub — the only Router impls in tree are
        // null / runtime-internal, neither of which lets a test peek
        // at what got pushed.
        struct RecordingRouter {
            last: RefCell<Option<String>>,
        }
        impl Router for RecordingRouter {
            fn current(&self) -> RouteState {
                RouteState {
                    path: self.last.borrow().clone().unwrap_or_else(|| "/".into()),
                    params: BTreeMap::new(),
                    query: BTreeMap::new(),
                    stack: vec!["/".into()],
                }
            }
            fn push(&self, p: &str) {
                *self.last.borrow_mut() = Some(p.to_owned());
            }
            fn replace(&self, p: &str) {
                *self.last.borrow_mut() = Some(p.to_owned());
            }
            fn pop(&self) {}
            fn reset(&self, _: &str) {}
        }

        let (mut rt, doc) = build_runtime(
            r#"{
              "version":"0.8.0",
              "routes":{
                "entry":"/list",
                "routes":{
                  "/list": { "pageId":"list" },
                  "/detail/:id": { "pageId":"list", "params":{ "id":"string" } }
                }
              },
              "pages":[{ "id":"list","name":"List","children":[
                { "type":"frame","id":"card",
                  "semantics":{ "aiName":"open_detail" },
                  "route":{ "push": "/detail/:id" }
                }
              ]}],
              "children":[]
            }"#,
        );
        let router = std::rc::Rc::new(RecordingRouter {
            last: RefCell::new(None),
        });
        rt.nav = router.clone();

        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut dispatcher = RuntimeDispatcher::new(&mut rt);
        let out = surface.execute(
            "list.open_open_detail",
            Some(&json!({ "id": "42" })),
            &mut dispatcher,
        );
        assert!(matches!(out, ExecuteOutcome::Ok), "outcome={out:?}");
        assert_eq!(router.current().path, "/detail/42");
    }
}
