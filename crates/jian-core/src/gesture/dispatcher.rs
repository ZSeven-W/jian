//! EventDispatcher — route SemanticEvent to the node's `events.*` ActionList
//! and run it via Plan 4's `execute_list`.
//!
//! **Event bubbling** (CSS-style): when the topmost hit node has no
//! handler for the event, the dispatcher walks up the parent chain
//! and runs the first matching ancestor's handler. Without this, a
//! Tap on the text *inside* a button gets silently dropped because
//! the text node has no `events.onTap` even though the button does.
//! Bubbling fires at most one handler per event.
//!
//! Handler configuration is honored here: a node whose
//! `gestures.disabledEvents` lists the event key, or whose
//! `gestures.disabled` expression evaluates truthy, is skipped and
//! bubbling continues to an eligible ancestor. `interactionOrder` is
//! authoring presentation only and is not consulted.

use super::config;
use super::semantic::SemanticEvent;
use crate::document::{NodeKey, RuntimeDocument};

/// Resolve the JSON `events.<handler_key>` ActionList for the event's
/// target node OR any ancestor up to the root, skipping nodes that
/// statically disable the handler (`disabledEvents`) or whose
/// `gestures.disabled` expression evaluates truthy (`node_disabled`).
/// Returns `(handler_owner, list)` — the owner node is the layout
/// target for node-local payload coordinates. `None` when no node in
/// the chain declares the handler.
pub(crate) fn resolve_handler(
    doc: &RuntimeDocument,
    event: &SemanticEvent,
    node_disabled: impl Fn(NodeKey) -> bool,
) -> Option<(NodeKey, serde_json::Value)> {
    let mut node_key = Some(event.node());
    for _ in 0..=doc.tree.nodes.len() {
        let key = node_key?;
        let data = doc.tree.nodes.get(key)?;
        let declares = config::node_declares_handler(doc, key, event.handler_key());
        if declares
            && !config::node_disables_handler(doc, key, event.handler_key())
            && !node_disabled(key)
        {
            if let Some(list) = extract_handler(&data.schema, event.handler_key()) {
                return Some((key, list));
            }
        }
        node_key = data.parent;
    }
    None
}

/// Owner-anchored `onSwipe` resolution for a claimed Swipe.
///
/// A Swipe's recognizer target is its CAPTURED handler owner (the node
/// that supplied the qualifying thresholds), not a generic hit target —
/// so the bubbling walk of [`resolve_handler`] must never apply to it:
/// after a same-batch `PressCancel` action dynamically disables the
/// captured owner, a generic bubble would re-bind the claimed Swipe to
/// the next *enabled* ancestor handler (the parent's), executing a
/// handler whose thresholds never qualified.
///
/// Resolution is anchored EXACTLY at `event.node()`: the owner must
/// still exist, still declare an enabled (nonempty, not
/// `disabledEvents`-slated, not dynamically disabled) `onSwipe`.
/// Otherwise the Swipe is dropped — never re-resolved to an ancestor.
/// Other semantics (Tap/Press/Pan/…) keep [`resolve_handler`] bubbling.
pub(crate) fn resolve_swipe_owner(
    doc: &RuntimeDocument,
    event: &SemanticEvent,
    node_disabled: impl Fn(NodeKey) -> bool,
) -> Option<(NodeKey, serde_json::Value)> {
    let key = event.node();
    let data = doc.tree.nodes.get(key)?;
    if !config::node_declares_handler(doc, key, event.handler_key())
        || config::node_disables_handler(doc, key, event.handler_key())
        || node_disabled(key)
    {
        return None;
    }
    extract_handler(&data.schema, event.handler_key()).map(|list| (key, list))
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
