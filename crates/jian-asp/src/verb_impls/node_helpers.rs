//! Helpers shared by every verb that needs to summarise a node —
//! `find` / `inspect` / operation verbs (`tap` / `type` / `scroll`
//! / `swipe`). Lives in a module that's available under both the
//! `dev-asp` and `prod-asp` feature flags (Plan 18 ASP prod mode /
//! C3) so prod builds — which drop the structural verbs but keep
//! the operation verbs — can still resolve a `Selector` to a
//! `NodeSummary` for logging without pulling in `find_verb` /
//! `ax_verb` / `snapshot_verb` etc.
//!
//! Originally lived under `find_verb.rs`; renamed in C3 because
//! the module never owned a dispatch-handler body — `run_find`
//! lives in `verb_impls/mod.rs` and reaches into these helpers.
//! Keeping the helpers here avoids cyclic imports and lets the
//! prod build elide `find_verb`-shaped code without pulling the
//! operation-verb logging path with it.

use crate::protocol::NodeSummary;
use jian_core::document::tree::node_schema_id;
use jian_core::document::{NodeKey, RuntimeDocument};
use jian_core::Runtime;
use jian_ops_schema::node::{PenNode, TextContent};

/// Project the matched node keys into compact summaries the agent
/// can reason over without pulling the full schema graph. `cap`
/// truncates the result so an agent's context window doesn't blow
/// up when a wildcard selector matches every node.
pub fn collect_node_summaries(
    doc: &RuntimeDocument,
    hits: &[NodeKey],
    runtime: &Runtime,
    cap: usize,
) -> Vec<NodeSummary> {
    hits.iter()
        .take(cap)
        .filter_map(|&key| node_summary(doc, runtime, key))
        .collect()
}

fn node_summary(doc: &RuntimeDocument, runtime: &Runtime, key: NodeKey) -> Option<NodeSummary> {
    let data = doc.tree.nodes.get(key)?;
    let id = node_schema_id(&data.schema).to_owned();
    // Prefer the author-supplied `semantics.role` when present
    // (matches the resolver's two-tier role match): a rectangle
    // declared as `semantics.role: "button"` reports "button"
    // here so the agent's `find` / `inspect` payload agrees with
    // the selector that matched it. Falls back to the primitive
    // variant name when no semantic role is set.
    let role = node_semantic_role(&data.schema).or_else(|| Some(role_for(&data.schema).to_owned()));
    let text = visible_text(&data.schema);
    let rect = runtime
        .layout
        .node_rect(key)
        .map(|r| [r.origin.x, r.origin.y, r.size.width, r.size.height]);
    let visible = node_is_statically_visible(&data.schema);
    Some(NodeSummary {
        id,
        role,
        text,
        visible,
        rect: rect.unwrap_or([0.0; 4]),
    })
}

/// Reads the author-declared `semantics.role` for any node
/// variant via a serde-json round-trip. Mirrors the helper in
/// `selector::resolve` — kept duplicated here to avoid
/// publishing the resolver's private helpers as crate-level
/// API just for one read site.
fn node_semantic_role(node: &PenNode) -> Option<String> {
    let v = serde_json::to_value(node).ok()?;
    v.get("semantics")?.get("role")?.as_str().map(str::to_owned)
}

pub(crate) fn role_for(node: &PenNode) -> &'static str {
    match node {
        PenNode::Frame(_) => "frame",
        PenNode::Group(_) => "group",
        PenNode::Rectangle(_) => "rectangle",
        PenNode::Ellipse(_) => "ellipse",
        PenNode::Line(_) => "line",
        PenNode::Polygon(_) => "polygon",
        PenNode::Path(_) => "path",
        PenNode::Text(_) => "text",
        PenNode::TextInput(_) => "text_input",
        PenNode::Image(_) => "image",
        PenNode::IconFont(_) => "icon_font",
        PenNode::Ref(_) => "ref",
    }
}

pub(crate) fn visible_text(node: &PenNode) -> Option<String> {
    match node {
        PenNode::Text(t) => match &t.content {
            TextContent::Plain(s) => Some(s.clone()),
            TextContent::Styled(segments) => {
                let mut out = String::new();
                for seg in segments {
                    out.push_str(&seg.text);
                }
                Some(out)
            }
        },
        PenNode::TextInput(t) => t
            .value
            .as_ref()
            .filter(|s| !s.is_empty())
            .or(t.placeholder.as_ref())
            .cloned(),
        _ => None,
    }
}

/// Static visibility — the schema's own `visible` field if set,
/// otherwise true. Phase 3 layers a binding-aware check on top
/// once the verb-impls have access to the live `StateGraph` /
/// `LayoutEngine` borrows; for the agent's `find` / `inspect`
/// path the static value is the right level of detail.
pub(crate) fn node_is_statically_visible(node: &PenNode) -> bool {
    let Ok(json) = serde_json::to_value(node) else {
        return true;
    };
    json.get("visible")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}
