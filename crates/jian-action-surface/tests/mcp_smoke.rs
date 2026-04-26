//! End-to-end MCP protocol smoke test.
//!
//! Spec: Plan `2026-04-25-jian-action-surface-mcp.md` Task 5.
//!
//! Drives the full `tools/list` + `tools/call` round-trip across an
//! in-process `tokio::io::duplex` pipe — no real stdin/stdout. The
//! transport is the same `ServiceExt::serve(...)` path the
//! production stdio listener uses, so this exercises the rmcp
//! framing + JSON-RPC envelope + tool-router dispatch + bridge +
//! `ActionSurface` chain end-to-end.
//!
//! Why a single test rather than per-error-shape #[test]s: each
//! test would need its own duplex + server task + drain loop set
//! up, which is ~30 lines of plumbing per case. Bundling the
//! assertions into one client session keeps the scaffolding cost
//! amortised, the rmcp handshake runs exactly once, and the test
//! reads top-to-bottom as the spec scenario the AI client follows.

#![cfg(feature = "mcp")]

use jian_action_surface::mcp::{Bridge, JianToolServer, Request};
use jian_action_surface::{
    ActionDispatcher, ActionSurface, AlwaysAllow, ExecuteError, SinkDispatcher,
};
use jian_ops_schema::PenDocument;
use rmcp::model::CallToolRequestParam;
use rmcp::ServiceExt;
use serde_json::{json, Map, Value};
use std::time::Duration;
use tokio::task::LocalSet;

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
    .expect("fixture parses")
}

/// Dispatcher that never fails — for the happy-path + gate tests.
fn dispatch_via<D: ActionDispatcher>(
    surface: &mut ActionSurface,
    req: Request,
    dispatcher: &mut D,
) {
    match req {
        Request::List { opts, reply } => {
            let _ = reply.send(surface.list(opts));
        }
        Request::Execute {
            name,
            params,
            reply,
        } => {
            let outcome =
                surface.execute_with_gate(&name, params.as_ref(), dispatcher, &AlwaysAllow);
            let _ = reply.send(outcome);
        }
    }
}

#[test]
fn full_protocol_round_trip_against_in_process_duplex() {
    // Current-thread runtime + LocalSet because `ActionSurface` holds
    // an `Rc<ActionAuditLog>` — `!Send`, so the drain task can't run
    // on a multi-threaded executor.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio rt");
    let local = LocalSet::new();

    local.block_on(&rt, async move {
        let (server_io, client_io) = tokio::io::duplex(8 * 1024);

        // Server side: bridge + tool server, served on the duplex.
        let (bridge, mut drain) = Bridge::new();
        let server_task = tokio::spawn(async move {
            JianToolServer::new(bridge)
                .serve(server_io)
                .await
                .expect("server initialise")
                .waiting()
                .await
        });

        // Drain loop (`!Send` because of the Rc-bearing surface).
        // Single-threaded `spawn_local` lets us hold the surface and
        // service both tools/list + tools/call requests as the rmcp
        // server forwards them.
        let drain_task = tokio::task::spawn_local(async move {
            let doc = fixture();
            let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
            let mut sink = SinkDispatcher;
            loop {
                if let Some(req) = drain.try_recv() {
                    dispatch_via(&mut surface, req, &mut sink);
                    continue;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });

        // Client side: rmcp does the initialize handshake when
        // `().serve(...)` resolves.
        let client = ().serve(client_io).await.expect("client initialise");

        // tools/list — should advertise both list_available_actions
        // and execute_action.
        let tools = client.list_all_tools().await.expect("list tools");
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            tool_names.contains(&"list_available_actions"),
            "list_available_actions missing from tools/list: {tool_names:?}"
        );
        assert!(
            tool_names.contains(&"execute_action"),
            "execute_action missing from tools/list: {tool_names:?}"
        );

        // tools/call list_available_actions — body is a Json<ListResponse>.
        let listed = client
            .call_tool(CallToolRequestParam {
                name: "list_available_actions".into(),
                arguments: Some(empty_args()),
            })
            .await
            .expect("call list_available_actions");
        let list_payload = first_json(&listed);
        let actions = list_payload["actions"].as_array().expect("actions array");
        let names: Vec<&str> = actions.iter().filter_map(|a| a["name"].as_str()).collect();
        assert!(names.contains(&"home.plus"), "plus missing: {names:?}");
        assert!(
            names.contains(&"home.set_counter"),
            "set_counter missing: {names:?}"
        );
        assert!(
            !names.contains(&"home.hidden_btn"),
            "aiHidden leaked: {names:?}"
        );

        // tools/call execute_action — happy path.
        let ok = call_execute(&client, "home.plus", None).await;
        assert_eq!(ok, json!({ "ok": true }), "happy path: {ok}");

        // Error shape #1: unknown_action.
        let unknown = call_execute(&client, "home.does_not_exist", None).await;
        assert_eq!(
            unknown,
            json!({
                "ok": false,
                "error": { "kind": "NotAvailable", "reason": "unknown_action" }
            }),
            "unknown_action: {unknown}"
        );

        // Error shape #2: validation_failed (set_counter requires `value`).
        let bad_params = call_execute(&client, "home.set_counter", Some(json!({}))).await;
        assert_eq!(
            bad_params,
            json!({
                "ok": false,
                "error": { "kind": "ValidationFailed", "reason": "missing_required" }
            }),
            "validation_failed: {bad_params}"
        );

        // Error shape #3: rate_limited. Bucket is 10/sec; the first
        // ten plus calls already burned one (the happy-path above),
        // so another nine + one trigger lands the 11th.
        for _ in 0..9 {
            let _ = call_execute(&client, "home.plus", None).await;
        }
        let throttled = call_execute(&client, "home.plus", None).await;
        assert_eq!(
            throttled,
            json!({
                "ok": false,
                "error": { "kind": "NotAvailable", "reason": "rate_limited" }
            }),
            "rate_limited: {throttled}"
        );

        // Sanity: outcome objects expose only `ok` (and `error` on
        // failure). Spec §10 data-hiding — already pinned by the
        // unit test in `mcp/tools.rs`, but re-asserted here on the
        // wire payload to guarantee rmcp's framing didn't add a
        // diagnostic field.
        for body in [&ok, &unknown, &bad_params, &throttled] {
            let obj = body.as_object().expect("object");
            for k in obj.keys() {
                assert!(
                    k == "ok" || k == "error",
                    "wire body leaked field {k:?}: {body}"
                );
            }
            if let Some(err) = obj.get("error") {
                let eo = err.as_object().expect("error object");
                for k in eo.keys() {
                    assert!(
                        k == "kind" || k == "reason",
                        "wire error leaked field {k:?}: {err}"
                    );
                }
            }
        }

        // Tear down: cancel the client (drops its end of the duplex,
        // server `waiting()` resolves), then abort the drain.
        client.cancel().await.expect("client cancel");
        let _ = server_task.await;
        drain_task.abort();
    });
}

#[test]
fn execute_action_handler_error_round_trips_per_spec_5_3() {
    // Separate test because it needs a *failing* dispatcher in the
    // drain loop — wiring it as a per-test branch on the happy-path
    // scaffolding above would obscure the assertion.
    struct Failing;
    impl ActionDispatcher for Failing {
        fn dispatch(
            &mut self,
            _action: &jian_core::action_surface::ActionDefinition,
            _params: &Map<String, Value>,
        ) -> Result<(), ExecuteError> {
            Err(ExecuteError::handler_error())
        }
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio rt");
    let local = LocalSet::new();

    local.block_on(&rt, async move {
        let (server_io, client_io) = tokio::io::duplex(8 * 1024);
        let (bridge, mut drain) = Bridge::new();
        let server_task = tokio::spawn(async move {
            JianToolServer::new(bridge)
                .serve(server_io)
                .await
                .expect("server init")
                .waiting()
                .await
        });
        let drain_task = tokio::task::spawn_local(async move {
            let doc = fixture();
            let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
            let mut failing = Failing;
            loop {
                if let Some(req) = drain.try_recv() {
                    dispatch_via(&mut surface, req, &mut failing);
                    continue;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });

        let client = ().serve(client_io).await.expect("client init");

        let body = call_execute(&client, "home.plus", None).await;
        assert_eq!(
            body,
            json!({
                "ok": false,
                "error": { "kind": "ExecutionFailed", "reason": "handler_error" }
            }),
            "handler_error: {body}"
        );

        client.cancel().await.expect("client cancel");
        let _ = server_task.await;
        drain_task.abort();
    });
}

fn empty_args() -> Map<String, Value> {
    Map::new()
}

async fn call_execute(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    params: Option<Value>,
) -> Value {
    let mut args = Map::new();
    args.insert("name".to_owned(), Value::String(name.to_owned()));
    if let Some(p) = params {
        args.insert("params".to_owned(), p);
    }
    let res = client
        .call_tool(CallToolRequestParam {
            name: "execute_action".into(),
            arguments: Some(args),
        })
        .await
        .expect("call execute_action");
    first_json(&res)
}

fn first_json(res: &rmcp::model::CallToolResult) -> Value {
    let raw = res
        .content
        .first()
        .expect("call_tool returned no content")
        .raw
        .clone();
    // Each tool returns one Content::json(...) which serialises as a
    // text part containing the JSON-encoded value.
    if let Some(text) = raw.as_text() {
        serde_json::from_str::<Value>(&text.text).expect("payload is JSON")
    } else {
        panic!("expected text/json content, got {raw:?}");
    }
}
