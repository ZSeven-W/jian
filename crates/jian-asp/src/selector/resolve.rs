//! Resolve a [`Selector`] to a list of matching [`NodeKey`]s
//! (Plan 18 Task 2 — Phase 2 resolver).
//!
//! Walks the runtime [`NodeTree`] in document order and applies the
//! selector's predicates + combinators + final-pick rules.
//!
//! ## What's covered in this Phase 2 commit
//!
//! - `id` / `alias`: exact match against the schema's stable id (we
//!   don't have a separate `alias` field in the runtime today, so
//!   `alias` aliases to `id` until Plan 22 lands the action-surface
//!   alias map).
//! - `role`: exact match against the `PenNode` variant's wire name
//!   (`"rectangle"` / `"text"` / `"frame"` / etc — same case as the
//!   schema's `"type"` discriminator).
//! - `text`: exact match against the visible text content of a
//!   `PenNode::Text` node (concatenated `Plain` segments only —
//!   styled segments resolve to their `text` field too).
//! - `text_contains`: substring of the same content, case-sensitive.
//! - `not` / `all_of` / `any_of`: combinator semantics — recurse on
//!   each sub-selector and intersect / union the resulting
//!   `NodeKey` sets.
//! - `child_of`: a candidate matches when *some* ancestor in its
//!   parent chain matches the inner selector.
//! - `parent_of`: a candidate matches when *some* descendant in its
//!   subtree matches the inner selector.
//! - `first`: pick the document-order first match.
//! - `index`: zero-indexed pick; out-of-range → empty result.
//!
//! ## What's deferred (Phase 2.5+)
//!
//! - `visible` / `focused` / `enabled`: need `StateGraph` /
//!   `LayoutEngine` access for live binding evaluation. The
//!   resolver in this commit treats those fields as "not
//!   constrained" (filter is skipped); Phase 2.5 wires them.
//! - `near`: depends on `SpatialIndex` access. Same pattern —
//!   skipped today, Phase 2.5 follow-up.
//! - Mutually-exclusive validation (`first` + `index`, `text` +
//!   `text_contains`): the resolver emits an [`Err`] for `first +
//!   index` (verbs reject up-front), but the two text variants are
//!   AND-combined today (a node must satisfy *both* — which is
//!   typically empty if the agent supplied conflicting values).
//!   Phase 2.5 may surface a clearer error.

use crate::selector::types::Selector;
use jian_core::document::tree::node_schema_id;
use jian_core::document::{NodeData, NodeKey, NodeTree};
use jian_ops_schema::node::{PenNode, TextContent};

/// Errors the resolver can return up-front. Phase-2 callers map
/// these onto `OutcomePayload::invalid` before the verb runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// `first: true` AND `index: Some(_)` together — exactly one
    /// final-pick rule is allowed. Surface up so the agent can
    /// drop one of them rather than silently dropping its match.
    BothFirstAndIndex,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::BothFirstAndIndex => f.write_str(
                "selector provides both `first` and `index`; choose one final-pick rule",
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

impl Selector {
    /// Resolve this selector against `tree` and return the matching
    /// `NodeKey`s in **document order**. Empty result is not an
    /// error — the verb-impl turns a zero-match into
    /// `OutcomePayload::not_found`.
    pub fn resolve(&self, tree: &NodeTree) -> Result<Vec<NodeKey>, ResolveError> {
        // Final-pick mutual exclusion check up-front so a malformed
        // selector aborts before any tree walk.
        if matches!(self.first, Some(true)) && self.index.is_some() {
            return Err(ResolveError::BothFirstAndIndex);
        }

        // Phase 1: collect every node whose per-node predicates match.
        let mut hits: Vec<NodeKey> = Vec::new();
        for &root in &tree.roots {
            walk_collect(tree, root, self, &mut hits)?;
        }

        // Phase 2: combinators (`all_of` / `any_of` / `not`) operate
        // on sub-selectors; intersect / union / difference against
        // the candidate set.
        if let Some(subs) = &self.all_of {
            for sub in subs {
                let sub_hits = sub.resolve(tree)?;
                hits.retain(|k| sub_hits.contains(k));
            }
        }
        if let Some(subs) = &self.any_of {
            // Union: candidates satisfying any sub-selector pass.
            // We start with `hits` (which already passed the
            // outer's own per-node predicates) and intersect with
            // the union of sub-results.
            let mut union: Vec<NodeKey> = Vec::new();
            for sub in subs {
                let sub_hits = sub.resolve(tree)?;
                for k in sub_hits {
                    if !union.contains(&k) {
                        union.push(k);
                    }
                }
            }
            hits.retain(|k| union.contains(k));
        }
        if let Some(sub) = &self.not {
            let sub_hits = sub.resolve(tree)?;
            hits.retain(|k| !sub_hits.contains(k));
        }

        // Phase 3: relational filters that need ancestor / descendant
        // walks against the inner selector's matches.
        if let Some(inner) = &self.child_of {
            let inner_hits = inner.resolve(tree)?;
            hits.retain(|&k| has_ancestor_in(tree, k, &inner_hits));
        }
        if let Some(inner) = &self.parent_of {
            let inner_hits = inner.resolve(tree)?;
            hits.retain(|&k| has_descendant_in(tree, k, &inner_hits));
        }

        // Phase 4: final-pick.
        if matches!(self.first, Some(true)) {
            hits.truncate(1);
        } else if let Some(i) = self.index {
            let i = i as usize;
            if i < hits.len() {
                hits = vec![hits[i]];
            } else {
                hits.clear();
            }
        }

        Ok(hits)
    }
}

/// Walk `tree` from `node` in document order, pushing every key
/// whose per-node predicates match `sel`. The combinators in `sel`
/// are evaluated separately by the caller — this helper handles
/// only the leaf predicates.
fn walk_collect(
    tree: &NodeTree,
    node: NodeKey,
    sel: &Selector,
    out: &mut Vec<NodeKey>,
) -> Result<(), ResolveError> {
    let data = match tree.nodes.get(node) {
        Some(d) => d,
        None => return Ok(()),
    };
    if matches_leaf_predicates(data, sel) {
        out.push(node);
    }
    for &child in &data.children {
        walk_collect(tree, child, sel, out)?;
    }
    Ok(())
}

/// Apply just the leaf predicates (`id` / `alias` / `role` / `text` /
/// `text_contains`) to `data`. Combinators / relational fields /
/// final-pick are handled at the top level.
fn matches_leaf_predicates(data: &NodeData, sel: &Selector) -> bool {
    let id = node_schema_id(&data.schema);
    if let Some(want) = sel.id.as_deref() {
        if id != want {
            return false;
        }
    }
    if let Some(want) = sel.alias.as_deref() {
        // No runtime alias map yet — alias falls through to id.
        // Plan 22's action-surface alias resolution lights this
        // path up properly.
        if id != want {
            return false;
        }
    }
    if let Some(want) = sel.role.as_deref() {
        if node_role(&data.schema) != want {
            return false;
        }
    }
    if sel.text.is_some() || sel.text_contains.is_some() {
        let text = node_visible_text(&data.schema);
        if let Some(want) = sel.text.as_deref() {
            if text.as_deref() != Some(want) {
                return false;
            }
        }
        if let Some(want) = sel.text_contains.as_deref() {
            match text.as_deref() {
                Some(t) if t.contains(want) => {}
                _ => return false,
            }
        }
    }
    // `visible` / `focused` / `enabled` / `near` need state /
    // spatial access — Phase 2.5 wires them. Skip silently for now;
    // a selector that only constrains those fields matches every
    // node, which is more useful than rejecting every node.
    true
}

/// Map a `PenNode` variant to its wire-shape role string. Mirrors
/// the schema's `"type"` discriminator.
fn node_role(node: &PenNode) -> &'static str {
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

/// Concatenate the visible-text content of a Text node. Returns
/// `None` for non-text nodes (the leaf-predicate matcher uses the
/// `None` to short-circuit `text` / `text_contains` to "no match").
fn node_visible_text(node: &PenNode) -> Option<String> {
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
        PenNode::TextInput(t) => {
            // text_input carries Option<String> `value` and
            // `placeholder`. Match against value when present;
            // placeholder otherwise so an agent matching against
            // placeholder "Search…" works. Empty fallback ensures
            // a `text_contains: ""` selector is harmless.
            t.value
                .as_ref()
                .filter(|s| !s.is_empty())
                .or(t.placeholder.as_ref())
                .cloned()
                .or_else(|| Some(String::new()))
        }
        _ => None,
    }
}

/// True iff `node`'s ancestor chain contains any key in `targets`.
/// Walks `parent` links until reaching a root.
///
/// Cycle-bounded: `NodeTree`'s `parent` / `children` fields are
/// `pub`, so a buggy mutation could install a parent cycle that
/// would otherwise loop here forever. Cap the walk at the tree's
/// node count — a legitimate ancestor chain is at most that long.
fn has_ancestor_in(tree: &NodeTree, node: NodeKey, targets: &[NodeKey]) -> bool {
    let max_depth = tree.nodes.len();
    let mut cursor = tree.nodes.get(node).and_then(|d| d.parent);
    let mut steps = 0usize;
    while let Some(p) = cursor {
        if steps > max_depth {
            // Cycle detected — refuse rather than hang. The caller
            // gets `false`, which is the safe answer for the
            // `child_of` filter (don't claim a cyclic node is a
            // descendant of anything).
            return false;
        }
        if targets.contains(&p) {
            return true;
        }
        cursor = tree.nodes.get(p).and_then(|d| d.parent);
        steps += 1;
    }
    false
}

/// True iff `node`'s subtree contains any key in `targets`.
///
/// Cycle-bounded the same way as `has_ancestor_in`: track visited
/// keys so a `children` cycle doesn't recurse forever.
fn has_descendant_in(tree: &NodeTree, node: NodeKey, targets: &[NodeKey]) -> bool {
    let mut visited: Vec<NodeKey> = Vec::new();
    has_descendant_in_inner(tree, node, targets, &mut visited)
}

fn has_descendant_in_inner(
    tree: &NodeTree,
    node: NodeKey,
    targets: &[NodeKey],
    visited: &mut Vec<NodeKey>,
) -> bool {
    let data = match tree.nodes.get(node) {
        Some(d) => d,
        None => return false,
    };
    for &child in &data.children {
        if visited.contains(&child) {
            continue;
        }
        visited.push(child);
        if targets.contains(&child) {
            return true;
        }
        if has_descendant_in_inner(tree, child, targets, visited) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector::Selector;
    use jian_ops_schema::node::PenNode;

    /// Build a tree from a JSON blob — the schema's natural input form.
    fn tree_from_json(items: serde_json::Value) -> NodeTree {
        let mut tree = NodeTree::new();
        let arr = items.as_array().expect("json array").clone();
        for item in arr {
            let node: PenNode = serde_json::from_value(item).expect("PenNode parse");
            tree.insert_subtree(node, None);
        }
        tree
    }

    fn fixture() -> NodeTree {
        // A small but interesting tree:
        //   frame "root"
        //     rectangle "save-btn"
        //       text "save-label" "Save"
        //     rectangle "cancel-btn"
        //       text "cancel-label" "Cancel"
        //     text "header"  "Document"
        tree_from_json(serde_json::json!([
            {
                "type": "frame", "id": "root",
                "children": [
                    {
                        "type": "rectangle", "id": "save-btn",
                        "children": [
                            { "type": "text", "id": "save-label", "content": "Save" }
                        ]
                    },
                    {
                        "type": "rectangle", "id": "cancel-btn",
                        "children": [
                            { "type": "text", "id": "cancel-label", "content": "Cancel" }
                        ]
                    },
                    { "type": "text", "id": "header", "content": "Document" }
                ]
            }
        ]))
    }

    #[test]
    fn id_match_returns_one_node() {
        let tree = fixture();
        let s = Selector {
            id: Some("save-btn".into()),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(tree.get("save-btn"), hits.first().copied());
    }

    #[test]
    fn role_match_returns_all_text_nodes() {
        let tree = fixture();
        let s = Selector {
            role: Some("text".into()),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn text_match_finds_specific_label() {
        let tree = fixture();
        let s = Selector {
            text: Some("Save".into()),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(tree.get("save-label"), hits.first().copied());
    }

    #[test]
    fn text_contains_matches_substring() {
        let tree = fixture();
        let s = Selector {
            text_contains: Some("ave".into()),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(tree.get("save-label"), hits.first().copied());
    }

    #[test]
    fn child_of_filters_by_ancestor() {
        // Match all `text` nodes whose ancestor chain contains
        // `save-btn`. Only `save-label` qualifies.
        let tree = fixture();
        let s = Selector {
            role: Some("text".into()),
            child_of: Some(Box::new(Selector {
                id: Some("save-btn".into()),
                ..Default::default()
            })),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(tree.get("save-label"), hits.first().copied());
    }

    #[test]
    fn parent_of_filters_by_descendant() {
        // Match rectangles whose subtree contains a "Save" text.
        let tree = fixture();
        let s = Selector {
            role: Some("rectangle".into()),
            parent_of: Some(Box::new(Selector {
                text: Some("Save".into()),
                ..Default::default()
            })),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(tree.get("save-btn"), hits.first().copied());
    }

    #[test]
    fn all_of_intersects_sub_selectors() {
        // role=text AND text_contains=ave → only save-label.
        let tree = fixture();
        let s = Selector {
            all_of: Some(vec![
                Selector {
                    role: Some("text".into()),
                    ..Default::default()
                },
                Selector {
                    text_contains: Some("ave".into()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(tree.get("save-label"), hits.first().copied());
    }

    #[test]
    fn any_of_unions_sub_selectors() {
        // text=Save OR text=Cancel → both labels.
        let tree = fixture();
        let s = Selector {
            any_of: Some(vec![
                Selector {
                    text: Some("Save".into()),
                    ..Default::default()
                },
                Selector {
                    text: Some("Cancel".into()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn not_excludes_sub_selector() {
        // role=text NOT id=header → save-label, cancel-label.
        let tree = fixture();
        let s = Selector {
            role: Some("text".into()),
            not: Some(Box::new(Selector {
                id: Some("header".into()),
                ..Default::default()
            })),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn first_picks_document_order_match() {
        let tree = fixture();
        let s = Selector {
            role: Some("rectangle".into()),
            first: Some(true),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(tree.get("save-btn"), hits.first().copied());
    }

    #[test]
    fn index_picks_specific_match() {
        let tree = fixture();
        let s = Selector {
            role: Some("text".into()),
            index: Some(2),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(tree.get("header"), hits.first().copied());
    }

    #[test]
    fn index_out_of_range_returns_empty() {
        let tree = fixture();
        let s = Selector {
            role: Some("text".into()),
            index: Some(99),
            ..Default::default()
        };
        let hits = s.resolve(&tree).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn first_plus_index_errors_up_front() {
        let s = Selector {
            first: Some(true),
            index: Some(0),
            ..Default::default()
        };
        let tree = fixture();
        let err = s.resolve(&tree).unwrap_err();
        assert_eq!(err, ResolveError::BothFirstAndIndex);
    }

    #[test]
    fn empty_selector_matches_every_node() {
        let tree = fixture();
        let s = Selector::default();
        let hits = s.resolve(&tree).unwrap();
        // 1 frame + 2 rectangles + 3 text = 6 nodes.
        assert_eq!(hits.len(), 6);
    }
}
