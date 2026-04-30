//! `snapshot` verb implementation (Plan 18 Phase 3).
//!
//! Captures a structured view of the running app. Three formats:
//!
//! - **`text_tree`** (default): indented role + id tree with the
//!   matched text and computed rect on each line. Cheap, no
//!   renderer dependency, and the format an LLM agent without
//!   image input can reason over.
//! - **`none`**: empty payload — the agent only needed the verb
//!   to fire (synchronisation point inside a script). Returns
//!   `ok` with no detail.
//! - **`png_base64`**: deferred to a host-side renderer follow-up.
//!   Producing a base64 PNG requires a Skia raster surface
//!   (`jian-skia::SkiaSurface` + `softbuffer`), which the
//!   stand-alone `jian-asp` build doesn't link. Returns
//!   `error("snapshot", …)` with a hint pointing at `text_tree`
//!   so an agent can retry without round-tripping for help.
//!
//! Indentation is two spaces per depth level. Truncation: the
//! tree is hard-capped at [`MAX_NODES`] to keep agent context
//! windows from blowing up on a tree with thousands of nodes;
//! the trailing line includes `...truncated at N nodes...` when
//! the cap fires so the agent knows the view is partial.

use jian_core::document::tree::{node_schema_id, NodeKey, NodeTree};
use jian_core::Runtime;
use jian_ops_schema::node::PenNode;

use crate::protocol::{DetailKind, OutcomePayload, SnapshotFormat};
use crate::verb_impls::find_verb::{
    node_is_statically_visible, role_for, visible_text,
};

/// Hard cap on the snapshot's node count. Default budget — 500 is
/// well past any realistic single-screen `.op` document while
/// staying inside a few KB of agent context. Documents larger than
/// this are typically multi-page apps where the agent should
/// `inspect` a sub-tree instead.
const MAX_NODES: usize = 500;

pub fn run_snapshot(runtime: &Runtime, format: Option<SnapshotFormat>) -> OutcomePayload {
    let fmt = format.unwrap_or(SnapshotFormat::TextTree);
    match fmt {
        SnapshotFormat::None => snapshot_none(runtime),
        SnapshotFormat::TextTree => snapshot_text_tree(runtime),
        // PngBase64 is the deferred half of Plan 18 §Phase 3 (line
        // 131: "snapshot (PNG encoder)"). Returning an explicit
        // error tells the agent the format is unavailable; the hint
        // points at the implemented alternatives so the agent can
        // self-recover without round-tripping the user.
        SnapshotFormat::PngBase64 => OutcomePayload::error(
            "snapshot",
            "png_base64 format is deferred — jian-asp doesn't link a \
             raster renderer (Plan 18 Phase 3 follow-up); agents that \
             need pixels should request `text_tree` and reason on the \
             structured view instead",
        )
        .with_hint(
            "retry with `\"format\": \"text_tree\"` for a structured \
             tree, or `\"format\": null` for a synchronisation-only \
             snapshot"
                .to_owned(),
        ),
    }
}

/// `format: None` per Plan 18 §Snapshot ("verify render succeeds,
/// return bytes count only"). With no raster path linked, the most
/// honest "render succeeds" check we can make is: a document is
/// loaded *and* a layout has been built (so the frame the agent
/// might inspect actually has a rect to look at). The "bytes count"
/// the spec mentions is reported as the total laid-out node count
/// — there are no pixel bytes to count without a renderer.
fn snapshot_none(runtime: &Runtime) -> OutcomePayload {
    let Some(doc) = runtime.document.as_ref() else {
        return OutcomePayload::error(
            "snapshot",
            "no document loaded — nothing to snapshot",
        );
    };
    let total = doc.tree.len();
    let laid_out = doc
        .tree
        .nodes
        .keys()
        .filter(|&k| runtime.layout.node_rect(k).is_some())
        .count();
    if laid_out == 0 {
        return OutcomePayload::error(
            "snapshot",
            "document is loaded but no layout has been built — render \
             cannot be verified",
        )
        .with_hint(
            "the host must call `Runtime::build_layout(viewport)` before \
             a snapshot can confirm the frame is renderable"
                .to_owned(),
        );
    }
    OutcomePayload::ok(
        "snapshot",
        None,
        format!(
            "render verified: {}/{} node(s) laid out",
            laid_out, total
        ),
    )
}

fn snapshot_text_tree(runtime: &Runtime) -> OutcomePayload {
    let Some(doc) = runtime.document.as_ref() else {
        return OutcomePayload::error("snapshot", "no document loaded");
    };
    let mut buf = String::new();
    let mut count = 0usize;
    let mut truncated = false;
    for &root in doc.tree.roots.iter() {
        if count >= MAX_NODES {
            truncated = true;
            break;
        }
        write_subtree(&mut buf, &doc.tree, runtime, root, 0, &mut count, &mut truncated);
    }
    if truncated {
        buf.push_str(&format!(
            "...truncated at {} nodes (document has more — use `inspect` to drill into a sub-tree)\n",
            MAX_NODES
        ));
    }
    let narrative = format!(
        "captured {} node(s){}",
        count,
        if truncated { " (truncated)" } else { "" }
    );
    OutcomePayload::ok("snapshot", None, narrative).with_detail(DetailKind::Snapshot {
        format: "text_tree".into(),
        bytes_or_text: buf,
    })
}

fn write_subtree(
    buf: &mut String,
    tree: &NodeTree,
    runtime: &Runtime,
    key: NodeKey,
    depth: usize,
    count: &mut usize,
    truncated: &mut bool,
) {
    if *count >= MAX_NODES {
        *truncated = true;
        return;
    }
    let Some(data) = tree.nodes.get(key) else {
        return;
    };
    *count += 1;
    let role = role_for(&data.schema);
    let id = node_schema_id(&data.schema);
    let visible = node_is_statically_visible(&data.schema);
    let text = visible_text(&data.schema);
    let rect = runtime.layout.node_rect(key);

    for _ in 0..depth {
        buf.push_str("  ");
    }
    buf.push_str(role);
    buf.push_str(" #");
    buf.push_str(id);
    if !visible {
        buf.push_str(" hidden");
    }
    if let Some(r) = rect {
        buf.push_str(&format!(
            " rect=({:.1},{:.1},{:.1},{:.1})",
            r.origin.x, r.origin.y, r.size.width, r.size.height
        ));
    }
    if let Some(text) = text {
        let trimmed = compress_text(&text, 64);
        buf.push_str(" text=\"");
        buf.push_str(&trimmed);
        buf.push('"');
    }
    if let Some(role) = author_role(&data.schema) {
        buf.push_str(" role=");
        buf.push_str(&role);
    }
    buf.push('\n');

    for &child in data.children.iter() {
        if *count >= MAX_NODES {
            *truncated = true;
            break;
        }
        write_subtree(buf, tree, runtime, child, depth + 1, count, truncated);
    }
}

/// Author-declared `semantics.role` (`button`, `tab`, …). The verb
/// surfaces both the primitive role (`rectangle`) and the semantic
/// role when set — agents that pick a `tap` target by `role:button`
/// see the same value the resolver matches against.
fn author_role(node: &PenNode) -> Option<String> {
    let v = serde_json::to_value(node).ok()?;
    v.get("semantics")?
        .get("role")?
        .as_str()
        .map(str::to_owned)
}

/// Collapse whitespace runs into a single space and cap to `cap`
/// chars. Snapshot lines are one-per-node so very long text would
/// blow up the wire size for no reason — agents that need the full
/// content `inspect` the node directly.
fn compress_text(s: &str, cap: usize) -> String {
    let mut out = String::with_capacity(s.len().min(cap));
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    let trimmed = out.trim().to_owned();
    if trimmed.chars().count() > cap {
        let mut s: String = trimmed.chars().take(cap).collect();
        s.push('…');
        s
    } else {
        trimmed
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

    fn doc_two_levels() -> &'static str {
        r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"snap",
          "app":{"name":"snap","version":"1","id":"snap"},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"rectangle","id":"btn","x":0,"y":0,"width":80,"height":24,
                  "semantics": { "role": "button" } },
                { "type":"text","id":"label","content":"Save changes" }
              ]
            }
          ]
        }"##
    }

    #[test]
    fn snapshot_text_tree_default() {
        let rt = rt_with(doc_two_levels());
        let out = run_snapshot(&rt, None);
        assert!(out.ok, "expected ok, got {:?}", out);
        let detail = out.detail.expect("detail present");
        match detail {
            DetailKind::Snapshot { format, bytes_or_text } => {
                assert_eq!(format, "text_tree");
                assert!(bytes_or_text.contains("frame #root"));
                assert!(bytes_or_text.contains("rectangle #btn"));
                assert!(bytes_or_text.contains("role=button"));
                assert!(bytes_or_text.contains("text=\"Save changes\""));
            }
            other => panic!("expected Snapshot detail, got {:?}", other),
        }
    }

    #[test]
    fn snapshot_format_none_verifies_render_readiness() {
        let rt = rt_with(doc_two_levels());
        let out = run_snapshot(&rt, Some(SnapshotFormat::None));
        assert!(out.ok, "{:?}", out);
        assert!(out.detail.is_none());
        assert!(
            out.narrative.contains("render verified"),
            "expected render-ready narrative, got: {}",
            out.narrative
        );
    }

    #[test]
    fn snapshot_format_none_errors_when_layout_not_built() {
        let schema: PenDocument = jian_ops_schema::load_str(doc_two_levels()).unwrap().value;
        let rt = Runtime::new_from_document(schema).unwrap();
        // Note: NO build_layout / rebuild_spatial — this is the
        // "doc loaded, layout not built" intermediate state.
        let out = run_snapshot(&rt, Some(SnapshotFormat::None));
        assert!(!out.ok);
        assert!(out.narrative.contains("no layout"));
    }

    #[test]
    fn snapshot_png_format_returns_error_with_hint() {
        let rt = rt_with(doc_two_levels());
        let out = run_snapshot(&rt, Some(SnapshotFormat::PngBase64));
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("RuntimeError"));
        assert!(out.hints.iter().any(|h| h.contains("text_tree")));
    }

    #[test]
    fn snapshot_no_document() {
        let rt = Runtime::new();
        let out = run_snapshot(&rt, Some(SnapshotFormat::TextTree));
        assert!(!out.ok);
        assert!(out.narrative.contains("no document loaded"));
    }

    #[test]
    fn snapshot_compresses_whitespace() {
        let s = compress_text("  hello   world  \n  again  ", 64);
        assert_eq!(s, "hello world again");
    }

    #[test]
    fn snapshot_truncates_long_text() {
        let long = "a".repeat(200);
        let s = compress_text(&long, 32);
        assert!(s.ends_with('…'));
        assert_eq!(s.chars().count(), 33);
    }
}
