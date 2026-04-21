//! EventDispatcher — route SemanticEvent to the node's `events.*` ActionList
//! and run it via Plan 4's `execute_list`.

use super::semantic::SemanticEvent;
use crate::action::{execute_list_shared, ActionContext, ExecOutcome, SharedRegistry};
use crate::document::RuntimeDocument;

/// Resolve the JSON `events.<handler_key>` ActionList on the node corresponding
/// to `event.node()` and execute it. Returns the outcome (result + warnings).
/// If the node has no handler, returns Ok with empty warnings.
pub fn dispatch_event(
    doc: &RuntimeDocument,
    event: &SemanticEvent,
    reg: &SharedRegistry,
    ctx: &ActionContext,
) -> ExecOutcome {
    let node_key = event.node();
    let schema = &doc.tree.nodes[node_key].schema;
    if let Some(list) = extract_handler(schema, event.handler_key()) {
        execute_list_shared(reg, &list, ctx)
    } else {
        ExecOutcome {
            result: Ok(()),
            warnings: Vec::new(),
        }
    }
}

/// Pull `events.<handler>` off a PenNode. Because the schema types are
/// per-variant, we round-trip through JSON.
fn extract_handler(n: &jian_ops_schema::node::PenNode, handler: &str) -> Option<serde_json::Value> {
    let v = serde_json::to_value(n).ok()?;
    v.as_object()?
        .get("events")?
        .as_object()?
        .get(handler)
        .cloned()
}
