//! NDJSON transport — spec §5.1's "secondary regimen" of
//! line-delimited JSON-RPC over an arbitrary byte stream
//! (stdio / Unix socket / WebSocket frame).
//!
//! Phase 1 ships the in-process Rust core; this module gives hosts
//! a sync, single-threaded request handler they can wire into a
//! transport of their choice without pulling in tokio + rmcp.
//!
//! Wire format:
//!
//! ```text
//! { "jsonrpc": "2.0", "id": 1, "method": "list_available_actions",
//!   "params": { "page_scope": "current" } }
//! ```
//!
//! Response:
//!
//! ```text
//! { "jsonrpc": "2.0", "id": 1, "result": { "actions": [...] } }
//! ```
//!
//! On error (parse failure / unknown method / dispatch failure):
//!
//! ```text
//! { "jsonrpc": "2.0", "id": 1, "error": { "code": -32601,
//!   "message": "method not found" } }
//! ```
//!
//! The JSON-RPC 2.0 envelope is what MCP clients already speak —
//! once the rmcp transport lands, this module's handler shape
//! plugs straight in.

use crate::list::{ListOptions, PageScope};
use crate::{ActionDispatcher, ActionSurface, AlwaysAllow, ExecuteOutcome, StateGate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// JSON-RPC 2.0 request (subset — `id` may be omitted for
/// notifications, but Phase 1 only handles request/response).
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

/// JSON-RPC standard error codes — kept stable across crate
/// versions because clients hard-code them.
pub mod codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
}

/// Handle one JSON-RPC request line, returning the corresponding
/// response line. Single-threaded — caller serialises requests if
/// it wants ordered semantics.
///
/// `state_gate` defaults to `AlwaysAllow` (matches `ActionSurface::execute`);
/// pass a real gate via [`handle_request_with_gate`] when the host
/// has access to a `&Runtime`.
pub fn handle_request<D: ActionDispatcher>(
    surface: &mut ActionSurface,
    dispatcher: &mut D,
    raw: &str,
) -> String {
    handle_request_with_gate(surface, dispatcher, &AlwaysAllow, raw)
}

pub fn handle_request_with_gate<D: ActionDispatcher, G: StateGate>(
    surface: &mut ActionSurface,
    dispatcher: &mut D,
    state_gate: &G,
    raw: &str,
) -> String {
    // Two-step parse so we can distinguish "JSON syntax error"
    // (parse_error) from "valid JSON, wrong shape" (invalid_request).
    let raw_value: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            return reply_err(Value::Null, codes::PARSE_ERROR, &format!("parse: {}", e));
        }
    };
    let req: RpcRequest = match serde_json::from_value(raw_value.clone()) {
        Ok(r) => r,
        Err(e) => {
            // Recover the id if present so the client can correlate.
            let id = raw_value.get("id").cloned().unwrap_or(Value::Null);
            return reply_err(
                id,
                codes::INVALID_REQUEST,
                &format!("invalid request shape: {}", e),
            );
        }
    };
    if req.jsonrpc != "2.0" {
        return reply_err(req.id, codes::INVALID_REQUEST, "jsonrpc must be \"2.0\"");
    }
    // §5.1 invariant: when present, `params` must be an object so
    // each method can address fields by name. Reject non-object
    // shapes (e.g. arrays) up-front.
    if let Some(p) = req.params.as_ref() {
        if !p.is_object() && !p.is_null() {
            return reply_err(req.id, codes::INVALID_PARAMS, "params must be an object");
        }
    }
    let result = match req.method.as_str() {
        "list_available_actions" => list(surface, state_gate, req.params.as_ref()),
        "execute_action" => execute(surface, dispatcher, state_gate, req.params.as_ref()),
        _ => {
            return reply_err(
                req.id,
                codes::METHOD_NOT_FOUND,
                "method not found (supported: list_available_actions, execute_action)",
            );
        }
    };
    match result {
        Ok(value) => serialise(&RpcResponse {
            jsonrpc: "2.0",
            id: req.id,
            result: Some(value),
            error: None,
        }),
        Err((code, msg)) => reply_err(req.id, code, &msg),
    }
}

fn list<G: StateGate>(
    surface: &ActionSurface,
    state_gate: &G,
    params: Option<&Value>,
) -> Result<Value, (i32, String)> {
    let mut opts = ListOptions::default();
    if let Some(p) = params {
        if let Some(scope) = p.get("page_scope").and_then(|v| v.as_str()) {
            opts.page_scope = match scope {
                "current" => PageScope::Current,
                "all" => PageScope::All,
                _ => {
                    return Err((
                        codes::INVALID_PARAMS,
                        format!("page_scope must be \"current\" or \"all\" (got {})", scope),
                    ))
                }
            };
        }
        if let Some(b) = p.get("include_confirm_gated").and_then(|v| v.as_bool()) {
            opts.include_confirm_gated = b;
        }
        if let Some(s) = p.get("current_page").and_then(|v| v.as_str()) {
            opts.current_page = Some(s.to_owned());
        }
    }
    // Spec consistency: an action that's `state_gated` would
    // immediately reject `execute_action`, so it shouldn't appear
    // in the listed set. `list_with_gate` is the single-source-of-
    // truth filter used by the in-process surface, MCP host, and
    // this JSON-RPC transport — adding a second per-action lookup
    // here would re-introduce the alias-bypass / duplicated-logic
    // class of bugs Codex round 25 flagged.
    let resp = surface.list_with_gate(opts, state_gate);
    serde_json::to_value(resp).map_err(|e| (codes::INVALID_PARAMS, format!("serialise: {}", e)))
}

fn execute<D: ActionDispatcher, G: StateGate>(
    surface: &mut ActionSurface,
    dispatcher: &mut D,
    state_gate: &G,
    params: Option<&Value>,
) -> Result<Value, (i32, String)> {
    let p = params.ok_or((
        codes::INVALID_PARAMS,
        "execute_action requires `name` + optional `params`".into(),
    ))?;
    let name = p
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((codes::INVALID_PARAMS, "missing `name`".into()))?;
    let exec_params = p.get("params");
    let outcome = surface.execute_with_gate(name, exec_params, dispatcher, state_gate);
    let value = match outcome {
        ExecuteOutcome::Ok => json!({ "ok": true }),
        ExecuteOutcome::Err(e) => json!({ "ok": false, "error": e }),
    };
    Ok(value)
}

fn reply_err(id: Value, code: i32, message: &str) -> String {
    serialise(&RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcError {
            code,
            message: message.to_owned(),
        }),
    })
}

fn serialise(resp: &RpcResponse) -> String {
    serde_json::to_string(resp).unwrap_or_else(|e| {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{{\"code\":-32603,\"message\":\"serialise: {}\"}}}}",
            e
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SinkDispatcher;
    use jian_ops_schema::document::PenDocument;

    fn fixture() -> PenDocument {
        serde_json::from_str(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"plus","semantics":{ "aiName":"plus" },
                  "events":{ "onTap": [ { "set": { "$app.x": "1" } } ] }
                }
              ]}],
              "children":[]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn list_request_round_trip() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"list_available_actions","params":{}}"#;
        let resp = handle_request(&mut surface, &mut sink, req);
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert!(parsed["result"]["actions"].is_array());
    }

    #[test]
    fn execute_happy_path() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let req =
            r#"{"jsonrpc":"2.0","id":2,"method":"execute_action","params":{"name":"home.plus"}}"#;
        let resp = handle_request(&mut surface, &mut sink, req);
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(parsed["result"]["ok"], true);
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let req = r#"{"jsonrpc":"2.0","id":3,"method":"banana"}"#;
        let resp = handle_request(&mut surface, &mut sink, req);
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(parsed["error"]["code"], codes::METHOD_NOT_FOUND);
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let resp = handle_request(&mut surface, &mut sink, "not json");
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(parsed["error"]["code"], codes::PARSE_ERROR);
    }

    #[test]
    fn missing_jsonrpc_version() {
        let doc = fixture();
        let mut surface = ActionSurface::from_document(&doc, &[0u8; 16]);
        let mut sink = SinkDispatcher;
        let req = r#"{"jsonrpc":"1.0","id":4,"method":"list_available_actions"}"#;
        let resp = handle_request(&mut surface, &mut sink, req);
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(parsed["error"]["code"], codes::INVALID_REQUEST);
    }
}
