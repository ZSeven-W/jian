//! `tools/list` + `tools/call` rmcp tool definitions for the jian
//! Action Surface.
//!
//! `JianToolServer` holds the worker-side [`Bridge`] handle. Each
//! tool method builds the typed [`Request`] envelope, awaits the
//! main thread's reply, and projects that into rmcp's
//! `Json<ListResponse>` / `Json<ExecuteOutcome>` shapes — which the
//! rmcp transport then JSON-encodes to MCP's `tools/call` reply.
//!
//! The wire-level request structs ([`ListRequest`], [`ExecuteRequest`])
//! mirror in-process [`ListOptions`] but with `JsonSchema` /
//! `Deserialize` so rmcp can advertise the input schema and parse
//! incoming params. They're MCP-only — the in-process API stays on
//! the existing types.
//!
//! Spec §5.1 / §5.2 / §10:
//! - `list_available_actions` is rate-limit-exempt.
//! - `execute_action` writes one audit row per call, and the wire
//!   payload is exactly `{ ok: true } | { ok: false, error: { kind,
//!   reason } }` — no internal state path leaks.

use crate::mcp::bridge::Bridge;
use crate::{ListOptions, ListResponse, PageScope};
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, ErrorData as McpError, Implementation, ProtocolVersion,
    ServerCapabilities, ServerInfo,
};
use rmcp::{tool, tool_handler, tool_router, Json, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wire-level `tools/list_available_actions` request body.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct ListRequest {
    /// Defaults to "current" — only show actions on the active
    /// page. Pass `"all"` to see every page's actions.
    #[serde(default)]
    pub page_scope: WirePageScope,
    /// The page id the AI client thinks is active. Echoed back as
    /// `page` in the response when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_page: Option<String>,
    /// Per spec §4.1: when true, ConfirmGated actions appear with
    /// `status: "confirm_gated"`. Off by default — the agent only
    /// sees `Available` actions.
    #[serde(default)]
    pub include_confirm_gated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WirePageScope {
    #[default]
    Current,
    All,
}

impl From<WirePageScope> for PageScope {
    fn from(w: WirePageScope) -> Self {
        match w {
            WirePageScope::Current => PageScope::Current,
            WirePageScope::All => PageScope::All,
        }
    }
}

impl From<ListRequest> for ListOptions {
    fn from(req: ListRequest) -> Self {
        Self {
            page_scope: req.page_scope.into(),
            current_page: req.current_page,
            include_confirm_gated: req.include_confirm_gated,
        }
    }
}

/// Wire-level `tools/execute_action` request body.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ExecuteRequest {
    /// Fully-qualified action name (e.g. `home.sign_in`).
    pub name: String,
    /// Action params per the surface's `params_schema`. Optional —
    /// verb-style actions (Tap / Submit / …) accept no params.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// rmcp service that bridges `tools/list` + `tools/call` into the
/// in-process surface via the worker-thread [`Bridge`].
#[derive(Clone)]
pub struct JianToolServer {
    bridge: Bridge,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl JianToolServer {
    pub fn new(bridge: Bridge) -> Self {
        Self {
            bridge,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "list_available_actions",
        description = "List the actions the runtime exposes for the active page (or every page)."
    )]
    async fn list_available_actions(
        &self,
        params: Parameters<ListRequest>,
    ) -> Result<Json<ListResponse>, McpError> {
        let resp = self
            .bridge
            .list(params.0.into())
            .await
            .ok_or_else(|| McpError::internal_error("runtime drain unavailable", None))?;
        Ok(Json(resp))
    }

    #[tool(
        name = "execute_action",
        description = "Execute a derived action by name. Returns { ok: true } on success or a structured ExecuteFailed payload."
    )]
    async fn execute_action(
        &self,
        params: Parameters<ExecuteRequest>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = self
            .bridge
            .execute(params.0.name, params.0.params)
            .await
            .ok_or_else(|| McpError::internal_error("runtime drain unavailable", None))?;
        let body = serde_json::to_value(&outcome)
            .map_err(|e| McpError::internal_error(format!("serialize outcome: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::json(body)?]))
    }
}

#[tool_handler]
impl ServerHandler for JianToolServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "AI Action Surface for a Jian runtime. Call `list_available_actions` \
                 to discover derived tools, then `execute_action` to run one. Errors \
                 follow the four-tier taxonomy { NotAvailable | ValidationFailed | \
                 Busy | ExecutionFailed }."
                    .to_owned(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_request_defaults_match_in_process_options() {
        let req = ListRequest::default();
        let opts: ListOptions = req.into();
        assert!(matches!(opts.page_scope, PageScope::Current));
        assert!(opts.current_page.is_none());
        assert!(!opts.include_confirm_gated);
    }

    #[test]
    fn list_request_all_scope_maps_through() {
        let json = serde_json::json!({ "page_scope": "all" });
        let req: ListRequest = serde_json::from_value(json).unwrap();
        let opts: ListOptions = req.into();
        assert!(matches!(opts.page_scope, PageScope::All));
    }

    #[test]
    fn execute_request_round_trips() {
        let json = serde_json::json!({
            "name": "home.plus",
            "params": { "value": 42 }
        });
        let req: ExecuteRequest = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(req.name, "home.plus");
        assert_eq!(
            req.params
                .as_ref()
                .unwrap()
                .get("value")
                .and_then(|v| v.as_i64()),
            Some(42)
        );
        let back = serde_json::to_value(&req).unwrap();
        assert_eq!(back, json);
    }
}
