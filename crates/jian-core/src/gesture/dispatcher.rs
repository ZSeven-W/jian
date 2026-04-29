//! EventDispatcher — route SemanticEvent to the node's `events.*` ActionList
//! and run it via Plan 4's `execute_list`.
//!
//! **Event bubbling** (CSS-style): when the topmost hit node has no
//! handler for the event, the dispatcher walks up the parent chain
//! and runs the first matching ancestor's handler. Without this, a
//! Tap on the text *inside* a button gets silently dropped because
//! the text node has no `events.onTap` even though the button does.
//! Bubbling fires at most one handler per event.

use super::semantic::SemanticEvent;
use crate::action::{execute_list_shared, ActionContext, ExecOutcome, SharedRegistry};
use crate::document::RuntimeDocument;

/// Resolve the JSON `events.<handler_key>` ActionList for the event's
/// target node OR any ancestor up to the root, and execute the first
/// match. Returns the outcome (result + warnings). Empty Ok when no
/// node in the chain declares a handler.
pub fn dispatch_event(
    doc: &RuntimeDocument,
    event: &SemanticEvent,
    reg: &SharedRegistry,
    ctx: &ActionContext,
) -> ExecOutcome {
    // Cycle-bound the bubble walk at node count: legitimate
    // ancestor chains are shorter than that. NodeData.parent is
    // pub, so a buggy mutation could install a cycle and hang
    // every event dispatch — bail out instead.
    let max_steps = doc.tree.nodes.len();
    let mut node_key = Some(event.node());
    let mut steps = 0usize;
    while let Some(key) = node_key {
        if steps > max_steps {
            break;
        }
        let data = match doc.tree.nodes.get(key) {
            Some(d) => d,
            None => break,
        };
        if let Some(list) = extract_handler(&data.schema, event.handler_key()) {
            return execute_list_shared(reg, &list, ctx);
        }
        node_key = data.parent;
        steps += 1;
    }
    ExecOutcome {
        result: Ok(()),
        warnings: Vec::new(),
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
