//! `inspect what=ax_tree` implementation (Plan 18 Phase 3).
//!
//! Returns a compressed accessibility-tree text. Each line carries:
//!
//! - the **role** (author-declared `semantics.role` if present,
//!   otherwise the primitive variant — `button` if the author set
//!   it, else `rectangle` / `frame` / etc.);
//! - the visible **label** (text content, `text_input` value or
//!   placeholder, `aria-label`-style `semantics.label` if set);
//! - the layout **rect** when laid out;
//! - any **state** flags (`disabled`, `hidden`, `checked`,
//!   `expanded`) the schema or runtime expose.
//!
//! "Compressed" means decorative subtrees collapse: a frame whose
//! sub-tree contains no role / text / event handler is omitted
//! entirely (with its non-decorative descendants reparented to
//! the nearest ancestor that *does* survive). This keeps the
//! agent's view focused on the surface a screen-reader user
//! would interact with.
//!
//! Truncation: the output text is hard-capped at `MAX_AX_BYTES`
//! (16 KiB) bytes. When the cap fires the rest of the tree is
//! dropped and `truncated: true` is set on the detail.
//! Agents that hit the cap typically scope down by passing a
//! selector to `inspect what=ax_tree` (Phase 4 — for now the
//! verb walks every root).

use jian_core::document::tree::{node_schema_id, NodeKey, NodeTree};
use jian_core::Runtime;
use jian_ops_schema::node::PenNode;

use crate::protocol::{DetailKind, OutcomePayload};
use crate::verb_impls::find_verb::{node_is_statically_visible, role_for, visible_text};

/// Hard cap on the text payload. Tightly-bounded: the ax_tree is
/// a compressed view that an agent should be able to read in a
/// single round of context, not a full document dump (use
/// `snapshot` for that).
const MAX_AX_BYTES: usize = 16 * 1024;

pub fn run_inspect_ax_tree(runtime: &Runtime) -> OutcomePayload {
    let Some(doc) = runtime.document.as_ref() else {
        return OutcomePayload::error(
            "inspect",
            "no document loaded — ax_tree needs a runtime tree to walk",
        );
    };
    let mut buf = String::new();
    let mut truncated = false;
    let tree = &doc.tree;
    for &root in tree.roots.iter() {
        if !ax_relevant_subtree(tree, root) {
            continue;
        }
        write_ax_subtree(&mut buf, tree, runtime, root, 0, &mut truncated);
        if truncated {
            break;
        }
    }
    if truncated {
        buf.push_str("...truncated (ax_tree byte budget exceeded)\n");
    }
    let line_count = buf.lines().count();
    OutcomePayload::ok("inspect", None, format!("{} ax line(s)", line_count)).with_detail(
        DetailKind::AxTree {
            text: buf,
            truncated,
        },
    )
}

/// Recursive walk: print the node when ax-relevant, then recurse
/// into children. Decorative nodes drop themselves but still
/// recurse — their relevant descendants reparent visually under
/// the nearest surviving ancestor by reusing `depth` rather than
/// `depth + 1` for the dropped level.
fn write_ax_subtree(
    buf: &mut String,
    tree: &NodeTree,
    runtime: &Runtime,
    key: NodeKey,
    depth: usize,
    truncated: &mut bool,
) {
    if *truncated {
        return;
    }
    let Some(data) = tree.nodes.get(key) else {
        return;
    };
    let mine_relevant = is_ax_relevant_node(&data.schema);
    let next_depth = if mine_relevant {
        if !push_ax_line(buf, tree, runtime, key, depth) {
            *truncated = true;
            return;
        }
        depth + 1
    } else {
        depth
    };
    for &child in data.children.iter() {
        if *truncated {
            return;
        }
        if !ax_relevant_subtree(tree, child) {
            continue;
        }
        write_ax_subtree(buf, tree, runtime, child, next_depth, truncated);
    }
}

/// Append one ax line. Returns false when the byte cap fires so
/// the caller can stop walking — the partial line is dropped to
/// keep the tail clean.
fn push_ax_line(
    buf: &mut String,
    tree: &NodeTree,
    runtime: &Runtime,
    key: NodeKey,
    depth: usize,
) -> bool {
    let data = match tree.nodes.get(key) {
        Some(d) => d,
        None => return true,
    };
    let mut line = String::new();
    for _ in 0..depth {
        line.push_str("  ");
    }
    let role = author_role(&data.schema).unwrap_or_else(|| role_for(&data.schema).to_owned());
    line.push_str(&role);
    if let Some(label) = label_for(&data.schema) {
        line.push_str(" \"");
        line.push_str(&compress(&label, 96));
        line.push('"');
    }
    if let Some(r) = runtime.layout.node_rect(key) {
        line.push_str(&format!(
            " rect=({:.0},{:.0},{:.0},{:.0})",
            r.origin.x, r.origin.y, r.size.width, r.size.height
        ));
    }
    let id = node_schema_id(&data.schema);
    line.push_str(" #");
    line.push_str(id);
    for flag in state_flags(&data.schema) {
        line.push(' ');
        line.push_str(flag);
    }
    line.push('\n');
    if buf.len() + line.len() > MAX_AX_BYTES {
        return false;
    }
    buf.push_str(&line);
    true
}

/// True if this subtree contributes anything an assistive
/// technology would surface: a labelled / role-bearing node, or
/// any descendant that is one.
fn ax_relevant_subtree(tree: &NodeTree, key: NodeKey) -> bool {
    let Some(data) = tree.nodes.get(key) else {
        return false;
    };
    if is_ax_relevant_node(&data.schema) {
        return true;
    }
    data.children.iter().any(|&c| ax_relevant_subtree(tree, c))
}

/// True if this node has any property that warrants an ax line.
fn is_ax_relevant_node(node: &PenNode) -> bool {
    if author_role(node).is_some() {
        return true;
    }
    if visible_text(node).is_some() {
        return true;
    }
    match node {
        PenNode::TextInput(_) | PenNode::Text(_) | PenNode::IconFont(_) => return true,
        _ => {}
    }
    if has_any_event_handler(node) {
        return true;
    }
    if author_label(node).is_some() {
        return true;
    }
    false
}

/// Author-declared `semantics.role`.
fn author_role(node: &PenNode) -> Option<String> {
    let v = serde_json::to_value(node).ok()?;
    v.get("semantics")?.get("role")?.as_str().map(str::to_owned)
}

/// Author-declared `semantics.label` — the aria-label equivalent.
fn author_label(node: &PenNode) -> Option<String> {
    let v = serde_json::to_value(node).ok()?;
    v.get("semantics")?
        .get("label")?
        .as_str()
        .map(str::to_owned)
}

/// Combine `semantics.label` (when set) with the visible text;
/// `label` overrides on conflict (matches ARIA precedence).
fn label_for(node: &PenNode) -> Option<String> {
    if let Some(label) = author_label(node) {
        return Some(label);
    }
    visible_text(node)
}

/// True if the node carries any non-empty `events.*` handler.
fn has_any_event_handler(node: &PenNode) -> bool {
    let Ok(v) = serde_json::to_value(node) else {
        return false;
    };
    let Some(events) = v.get("events").and_then(|e| e.as_object()) else {
        return false;
    };
    events.values().any(|h| match h {
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Null => false,
        _ => true,
    })
}

/// Static state flags worth surfacing. Today only `hidden` is
/// truly static — `semantics.disabled` is an `Expression` (string)
/// the runtime evaluates per-frame, so a structural ax_tree can't
/// resolve it without committing to expression evaluation here.
/// A follow-up that walks `Runtime::eval` could add `disabled`
/// when the expression evaluates truthy.
fn state_flags(node: &PenNode) -> Vec<&'static str> {
    let mut out = Vec::new();
    if !node_is_statically_visible(node) {
        out.push("hidden");
    }
    out
}

/// Compress whitespace + cap label text. Inputs to ax_tree often
/// come from author-supplied prose (text nodes), so we collapse
/// runs and clip at `cap` chars to keep one line per node.
fn compress(s: &str, cap: usize) -> String {
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

    #[test]
    fn ax_tree_includes_roles_and_text() {
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"a",
          "app":{"name":"a","version":"1","id":"a"},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"rectangle","id":"submit","x":0,"y":0,"width":80,"height":24,
                  "semantics": { "role": "button", "label": "Submit" },
                  "events": { "onTap": [ { "set": { "$state.x": 1 } } ] } },
                { "type":"text","id":"hint","content":"Press to send" }
              ]
            }
          ]
        }"##;
        let rt = rt_with(doc);
        let out = run_inspect_ax_tree(&rt);
        assert!(out.ok, "{:?}", out);
        let detail = out.detail.expect("detail");
        match detail {
            DetailKind::AxTree { text, truncated } => {
                assert!(!truncated);
                assert!(text.contains("button \"Submit\""), "got:\n{}", text);
                assert!(text.contains("text \"Press to send\""), "got:\n{}", text);
                // The root frame has no role / handler / label and
                // no visible text — it's decorative and should be
                // collapsed out.
                assert!(
                    !text.contains("frame #root"),
                    "decorative frame leaked into ax tree:\n{}",
                    text
                );
            }
            other => panic!("expected AxTree, got {:?}", other),
        }
    }

    #[test]
    fn ax_tree_collapses_decorative_containers() {
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"a",
          "app":{"name":"a","version":"1","id":"a"},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"frame","id":"wrap","width":480,"height":40,"x":0,"y":0,
                  "children":[
                    { "type":"frame","id":"inner","width":80,"height":40,"x":0,"y":0,
                      "children":[
                        { "type":"text","id":"label","content":"Hello" }
                      ]
                    }
                  ]
                }
              ]
            }
          ]
        }"##;
        let rt = rt_with(doc);
        let out = run_inspect_ax_tree(&rt);
        let DetailKind::AxTree { text, .. } = out.detail.unwrap() else {
            panic!("expected AxTree");
        };
        // Only the `text` line survives; root/wrap/inner are all
        // decorative frames. The surviving line is at depth 0.
        assert_eq!(text.lines().count(), 1, "got:\n{}", text);
        let line = text.lines().next().unwrap();
        assert!(
            !line.starts_with("  "),
            "expected no indent, got: {:?}",
            line
        );
        assert!(line.contains("\"Hello\""));
    }

    #[test]
    fn ax_tree_emits_hidden_flag() {
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"a",
          "app":{"name":"a","version":"1","id":"a"},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"rectangle","id":"hidden_btn","x":0,"y":0,"width":80,"height":24,
                  "visible": false,
                  "semantics": { "role": "button", "label": "Cancel" } },
                { "type":"rectangle","id":"shown_btn","x":0,"y":40,"width":80,"height":24,
                  "semantics": { "role": "button", "label": "Confirm" } }
              ]
            }
          ]
        }"##;
        let rt = rt_with(doc);
        let out = run_inspect_ax_tree(&rt);
        let DetailKind::AxTree { text, .. } = out.detail.unwrap() else {
            panic!();
        };
        let hidden_line = text
            .lines()
            .find(|l| l.contains("Cancel"))
            .expect("hidden button line present");
        assert!(
            hidden_line.contains("hidden"),
            "expected hidden flag, got: {:?}",
            hidden_line
        );
        let shown_line = text
            .lines()
            .find(|l| l.contains("Confirm"))
            .expect("shown button line present");
        assert!(
            !shown_line.contains("hidden"),
            "shown button should not carry hidden, got: {:?}",
            shown_line
        );
    }

    #[test]
    fn ax_tree_no_document() {
        let rt = Runtime::new();
        let out = run_inspect_ax_tree(&rt);
        assert!(!out.ok);
        assert!(out.narrative.contains("no document loaded"));
    }

    #[test]
    fn ax_tree_truncates_at_byte_cap() {
        // Build a doc with many labelled buttons until we exceed
        // the cap. Each button line is ~60 bytes, so 16KB ≈ 270.
        // Use 2000 to be sure we cross the line.
        let mut children = String::new();
        for i in 0..2000 {
            children.push_str(&format!(
                r#"{{"type":"rectangle","id":"b{}","x":0,"y":0,"width":40,"height":24,"semantics":{{"role":"button","label":"Button {}"}}}},"#,
                i, i
            ));
        }
        let children = children.trim_end_matches(',');
        let doc = format!(
            r##"{{"formatVersion":"1.0","version":"1.0.0","id":"a","app":{{"name":"a","version":"1","id":"a"}},"children":[{{"type":"frame","id":"root","width":480,"height":80000,"x":0,"y":0,"children":[{}]}}]}}"##,
            children
        );
        let rt = rt_with(&doc);
        let out = run_inspect_ax_tree(&rt);
        let DetailKind::AxTree { text, truncated } = out.detail.unwrap() else {
            panic!();
        };
        assert!(truncated, "expected truncated=true on 2000-button doc");
        assert!(text.ends_with("...truncated (ax_tree byte budget exceeded)\n"));
        assert!(text.len() < MAX_AX_BYTES + 100);
    }
}
