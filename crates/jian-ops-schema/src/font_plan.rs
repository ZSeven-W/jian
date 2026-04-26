//! Codepoint scanner for `.op` documents (Plan 19 Task 4 foundation).
//!
//! Walks every text-bearing node in a [`PenDocument`] and produces a
//! [`FontPlan`] — a per-family record of which Unicode codepoints the
//! document actually uses. Plan 19 §C19 needs this in two places:
//!
//! 1. **Runtime cold-start.** Scan the *first-frame* visible subtree and
//!    only load the codepoint subset of each font that the user is about
//!    to see. The remainder loads on a post-paint background frame.
//! 2. **`.op.pack` AOT.** Scan the *whole* document at pack time, hand
//!    each `(family, codepoints)` pair to a font subsetter, and embed
//!    the minimal subset alongside the full font. This commit ships
//!    only the scanner + the data structure — Plan 19 Task 4 follow-ups
//!    add the subsetter dep (e.g. `subsetter` / `ttf-parser`) and the
//!    pack format extension.
//!
//! ## What the scanner does
//!
//! - Walks every child of the document recursively, descending into
//!   `frame` / `group` / `rectangle` containers.
//! - For [`PenNode::Text`], iterates the content — `Plain(String)`
//!   contributes against the node-level `font_family`; styled segments
//!   contribute against `segment.font_family` if set, else the parent
//!   text node's family.
//! - For [`PenNode::TextInput`], the placeholder + value contribute
//!   against the default family (the schema doesn't yet expose a
//!   per-input `fontFamily`; if one is added later, plumb it through here).
//! - Other node types (`Image`, `IconFont`, `Path`, `Line`, `Ellipse`,
//!   `Polygon`, `Ref`) do not contribute. `IconFont` is intentionally
//!   excluded: icon glyphs come from a curated icon family whose
//!   subsetting policy is different (we ship the whole curated SVG
//!   pack rather than per-codepoint subsets).
//!
//! ## "Default" family
//!
//! Text nodes whose `font_family` is `None` are recorded under the empty
//! string `""`. The runtime treats that as "host's default sans-serif",
//! and the AOT pack writer either subsets the platform default or skips
//! subsetting (the host's font is already on disk). Use
//! [`FontPlan::for_family`] with `""` to inspect the default-family
//! usage explicitly.

use crate::document::PenDocument;
use crate::node::text::TextContent;
use crate::node::PenNode;
use std::collections::{BTreeMap, BTreeSet};

/// One family's usage in a document scan.
///
/// Fields are `pub` for ergonomic destructuring + serialisation, but
/// callers should **treat them as read-only** once a scan finishes —
/// the scanner upholds an internal invariant (`run_count > 0` whenever
/// `codepoints` is non-empty, and vice versa) that mutating from
/// outside would break. See [`FontPlan::scan`] for the canonical
/// construction path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontUsage {
    /// Unicode codepoints used by this family. Sorted set so callers
    /// (subsetters, AOT writers) get a stable iteration order.
    pub codepoints: BTreeSet<u32>,
    /// Number of distinct text runs that referenced this family. Useful
    /// signal for prioritisation: a family that backs 100 runs is more
    /// important to preload than one that backs a single tooltip.
    pub run_count: u32,
}

impl FontUsage {
    pub fn is_empty(&self) -> bool {
        self.codepoints.is_empty() && self.run_count == 0
    }
}

/// Per-document codepoint plan. See module docs for full semantics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontPlan {
    per_family: BTreeMap<String, FontUsage>,
}

impl FontPlan {
    /// Construct an empty plan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Scan an entire document — every text run in every visible node
    /// type contributes to the plan.
    pub fn scan(doc: &PenDocument) -> Self {
        let mut plan = Self::new();
        for node in &doc.children {
            scan_node(node, None, &mut plan);
        }
        plan
    }

    /// Scan a subtree rooted at `node`. Useful for the runtime
    /// "first-frame visible" path: the host pre-filters to the visible
    /// subset, then this method records codepoints from just those
    /// nodes. The full-document scan ([`Self::scan`]) is the AOT path.
    pub fn scan_subtree(root: &PenNode) -> Self {
        let mut plan = Self::new();
        scan_node(root, None, &mut plan);
        plan
    }

    /// Scan multiple subtree roots into a single plan. The runtime's
    /// first-frame path typically computes a `Vec<&PenNode>` of visible
    /// roots (e.g. one per active page + any open modals); this method
    /// folds them all into one plan without exposing the private
    /// scanner internals.
    pub fn scan_subtrees<'a, I>(roots: I) -> Self
    where
        I: IntoIterator<Item = &'a PenNode>,
    {
        let mut plan = Self::new();
        for root in roots {
            scan_node(root, None, &mut plan);
        }
        plan
    }

    /// Iterate every recorded `(family, usage)` pair. Empty-string
    /// family means "host default sans-serif" (see module docs).
    pub fn families(&self) -> impl Iterator<Item = (&str, &FontUsage)> {
        self.per_family.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Look up a single family's usage. Pass `""` for the default.
    pub fn for_family(&self, family: &str) -> Option<&FontUsage> {
        self.per_family.get(family)
    }

    /// Total codepoints across every family (counting duplicates across
    /// families separately — same character in two families counts twice).
    pub fn total_codepoints(&self) -> usize {
        self.per_family.values().map(|u| u.codepoints.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.per_family.is_empty()
    }

    pub fn len(&self) -> usize {
        self.per_family.len()
    }

    /// Record one text run. Empty `text` is a no-op (won't bump run
    /// count or insert codepoints) so authors who write
    /// `<text content="">` placeholders don't pollute the plan.
    fn record_run(&mut self, family: Option<&str>, text: &str) {
        if text.is_empty() {
            return;
        }
        let key = family.unwrap_or("").to_owned();
        let entry = self.per_family.entry(key).or_default();
        entry.run_count += 1;
        for ch in text.chars() {
            entry.codepoints.insert(ch as u32);
        }
    }
}

/// Recursive walker. `inherited_family` lets a styled segment fall back
/// to the parent text node's `font_family` when its own is `None`. The
/// caller passes `None` at the document root; the function passes
/// nothing useful through container nodes (containers don't carry a
/// font family — only text nodes do).
fn scan_node(node: &PenNode, inherited_family: Option<&str>, plan: &mut FontPlan) {
    match node {
        PenNode::Text(t) => {
            let family = t
                .font_family
                .as_deref()
                .or(inherited_family)
                .map(str::to_owned);
            match &t.content {
                TextContent::Plain(s) => plan.record_run(family.as_deref(), s),
                TextContent::Styled(segments) => {
                    for seg in segments {
                        // Segment-level font_family overrides node-level.
                        let seg_family = seg
                            .font_family
                            .as_deref()
                            .or(family.as_deref());
                        plan.record_run(seg_family, &seg.text);
                    }
                }
            }
        }
        PenNode::TextInput(ti) => {
            // text_input doesn't carry its own font_family yet (schema
            // doesn't expose one). Bind to the inherited family so a
            // future host-level default can plumb through; for now this
            // is always None → empty-string ("default") family.
            if let Some(p) = &ti.placeholder {
                plan.record_run(inherited_family, p);
            }
            if let Some(v) = &ti.value {
                plan.record_run(inherited_family, v);
            }
        }
        PenNode::Frame(f) => {
            if let Some(children) = &f.children {
                for c in children {
                    scan_node(c, inherited_family, plan);
                }
            }
        }
        PenNode::Group(g) => {
            if let Some(children) = &g.children {
                for c in children {
                    scan_node(c, inherited_family, plan);
                }
            }
        }
        PenNode::Rectangle(r) => {
            if let Some(children) = &r.children {
                for c in children {
                    scan_node(c, inherited_family, plan);
                }
            }
        }
        PenNode::Ref(r) => {
            // Ref nodes carry both `children` (typed PenNode trees that
            // sit alongside the referenced template — same shape as
            // Frame) and `descendants` (raw JSON overrides keyed by
            // descendant id, applied after the template expands at
            // runtime). We only walk `children`. Override JSON in
            // `descendants` may introduce or replace text content but
            // its values are not typed `PenNode`s — scanning it
            // accurately requires the template-expansion pass that
            // runs in `jian_core::document::loader`. Hosts that need
            // that path should scan the *post-expansion* runtime tree
            // via `scan_subtrees(...)`.
            if let Some(children) = &r.children {
                for c in children {
                    scan_node(c, inherited_family, plan);
                }
            }
        }
        // Image / IconFont / Path / Line / Ellipse / Polygon don't
        // contribute. See module docs for the IconFont rationale.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc_from(value: serde_json::Value) -> PenDocument {
        serde_json::from_value(value).expect("valid document JSON")
    }

    #[test]
    fn empty_document_yields_empty_plan() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": []
        }));
        let plan = FontPlan::scan(&doc);
        assert!(plan.is_empty());
        assert_eq!(plan.len(), 0);
        assert_eq!(plan.total_codepoints(), 0);
    }

    #[test]
    fn single_plain_text_records_codepoints() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                { "type": "text", "id": "t", "content": "Hi", "fontFamily": "Inter" }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        let usage = plan.for_family("Inter").expect("Inter recorded");
        assert_eq!(usage.run_count, 1);
        assert_eq!(usage.codepoints.len(), 2);
        assert!(usage.codepoints.contains(&('H' as u32)));
        assert!(usage.codepoints.contains(&('i' as u32)));
    }

    #[test]
    fn missing_font_family_records_under_default() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                { "type": "text", "id": "t", "content": "Hi" }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        let usage = plan.for_family("").expect("default family recorded");
        assert_eq!(usage.run_count, 1);
        assert_eq!(usage.codepoints.len(), 2);
    }

    #[test]
    fn duplicate_codepoints_dedupe_and_run_count_climbs() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                { "type": "text", "id": "a", "content": "AAA", "fontFamily": "Inter" },
                { "type": "text", "id": "b", "content": "AB",  "fontFamily": "Inter" }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        let usage = plan.for_family("Inter").unwrap();
        assert_eq!(usage.run_count, 2);
        // Codepoints: A, B (deduped from across the two runs).
        assert_eq!(usage.codepoints.len(), 2);
        assert!(usage.codepoints.contains(&('A' as u32)));
        assert!(usage.codepoints.contains(&('B' as u32)));
    }

    #[test]
    fn multiple_families_get_separate_entries() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                { "type": "text", "id": "a", "content": "A", "fontFamily": "Inter" },
                { "type": "text", "id": "b", "content": "B", "fontFamily": "Roboto" }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        assert_eq!(plan.len(), 2);
        assert!(plan.for_family("Inter").is_some());
        assert!(plan.for_family("Roboto").is_some());
    }

    #[test]
    fn styled_segments_contribute_with_per_segment_family() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                {
                    "type": "text",
                    "id": "rich",
                    "fontFamily": "Inter",
                    "content": [
                        { "text": "Hi ", "fontFamily": "Inter" },
                        { "text": "there",  "fontFamily": "Roboto" }
                    ]
                }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        let inter = plan.for_family("Inter").unwrap();
        let roboto = plan.for_family("Roboto").unwrap();
        // "Hi " → H, i, space
        assert_eq!(inter.codepoints.len(), 3);
        // "there" → t, h, e, r
        assert_eq!(roboto.codepoints.len(), 4);
    }

    #[test]
    fn segment_inherits_node_family_when_unset() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                {
                    "type": "text",
                    "id": "rich",
                    "fontFamily": "Inter",
                    "content": [
                        { "text": "Hi" }
                    ]
                }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        // Segment had no fontFamily → falls back to node's "Inter".
        assert!(plan.for_family("Inter").is_some());
        assert!(plan.for_family("").is_none());
    }

    #[test]
    fn text_input_records_placeholder_and_value() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                {
                    "type": "text_input",
                    "id": "email",
                    "width": 200,
                    "height": 40,
                    "placeholder": "name",
                    "value": "JD"
                }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        let usage = plan.for_family("").expect("default family");
        // Placeholder + value contribute as two runs.
        assert_eq!(usage.run_count, 2);
        // 'n','a','m','e','J','D' = 6 distinct codepoints.
        assert_eq!(usage.codepoints.len(), 6);
    }

    #[test]
    fn nested_text_in_frame_is_walked() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                {
                    "type": "frame",
                    "id": "root",
                    "children": [
                        { "type": "text", "id": "deep", "content": "X", "fontFamily": "Inter" }
                    ]
                }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        assert!(plan.for_family("Inter").is_some());
    }

    #[test]
    fn rectangle_with_text_child_is_walked() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                {
                    "type": "rectangle",
                    "id": "btn",
                    "width": 100,
                    "height": 40,
                    "children": [
                        { "type": "text", "id": "label", "content": "OK", "fontFamily": "Inter" }
                    ]
                }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        let usage = plan.for_family("Inter").unwrap();
        assert_eq!(usage.run_count, 1);
        assert_eq!(usage.codepoints.len(), 2);
    }

    #[test]
    fn empty_string_text_is_skipped() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                { "type": "text", "id": "blank", "content": "", "fontFamily": "Inter" }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        // Empty content shouldn't even create a family entry.
        assert!(plan.is_empty());
    }

    #[test]
    fn icon_font_does_not_contribute() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                { "type": "icon_font", "id": "ic", "iconFontName": "search", "iconFontFamily": "Lucide" }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        assert!(plan.is_empty(), "icon_font glyphs are out of scope for the codepoint plan");
    }

    #[test]
    fn scan_subtree_records_only_named_root() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                { "type": "text", "id": "t1", "content": "ABCD", "fontFamily": "Inter" },
                { "type": "text", "id": "t2", "content": "EFGH", "fontFamily": "Inter" }
            ]
        }));
        // Scan only the second subtree.
        let only_second = FontPlan::scan_subtree(&doc.children[1]);
        let usage = only_second.for_family("Inter").unwrap();
        assert_eq!(usage.run_count, 1);
        assert_eq!(usage.codepoints.len(), 4);
        for ch in ['E', 'F', 'G', 'H'] {
            assert!(usage.codepoints.contains(&(ch as u32)));
        }
        // The first subtree's codepoints are absent.
        for ch in ['A', 'B', 'C', 'D'] {
            assert!(!usage.codepoints.contains(&(ch as u32)));
        }
    }

    #[test]
    fn unicode_codepoints_record_correctly() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                { "type": "text", "id": "cjk", "content": "你好", "fontFamily": "NotoSansCJK" }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        let usage = plan.for_family("NotoSansCJK").unwrap();
        assert_eq!(usage.codepoints.len(), 2);
        assert!(usage.codepoints.contains(&('你' as u32)));
        assert!(usage.codepoints.contains(&('好' as u32)));
    }

    #[test]
    fn ref_node_children_are_walked_for_text() {
        // Codex round 1 MAJOR: RefNode has its own `children` Vec
        // (alongside the referenced template) — we must recurse into
        // those typed nodes. `descendants` overrides remain out of
        // scope until template expansion runs.
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                {
                    "type": "ref",
                    "id": "rf",
                    "ref": "button-primary",
                    "children": [
                        { "type": "text", "id": "label", "content": "Refed", "fontFamily": "Inter" }
                    ]
                }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        let usage = plan.for_family("Inter").expect("Ref child text reached");
        assert_eq!(usage.run_count, 1);
        // 'R','e','f','d' = 4 distinct codepoints.
        assert_eq!(usage.codepoints.len(), 4);
    }

    #[test]
    fn scan_subtrees_aggregates_multiple_roots() {
        // Codex round 1 MAJOR: external callers couldn't merge
        // multi-root scans without exposing the private fields.
        // `scan_subtrees` provides the canonical aggregation path.
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                { "type": "text", "id": "a", "content": "ABC", "fontFamily": "Inter" },
                { "type": "text", "id": "b", "content": "XYZ", "fontFamily": "Inter" },
                { "type": "text", "id": "c", "content": "skip me", "fontFamily": "Roboto" }
            ]
        }));
        // Aggregate the first two; skip the third.
        let roots: Vec<&PenNode> = vec![&doc.children[0], &doc.children[1]];
        let plan = FontPlan::scan_subtrees(roots);
        let inter = plan.for_family("Inter").expect("Inter recorded");
        assert_eq!(inter.run_count, 2);
        assert_eq!(inter.codepoints.len(), 6); // A,B,C,X,Y,Z
        assert!(plan.for_family("Roboto").is_none(), "third subtree skipped");
    }

    #[test]
    fn scan_subtrees_empty_iterator_yields_empty_plan() {
        let plan = FontPlan::scan_subtrees(std::iter::empty::<&PenNode>());
        assert!(plan.is_empty());
    }

    #[test]
    fn families_iterator_yields_every_recorded_family() {
        let doc = doc_from(json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "demo",
            "children": [
                { "type": "text", "id": "a", "content": "A", "fontFamily": "Inter" },
                { "type": "text", "id": "b", "content": "B", "fontFamily": "Roboto" },
                { "type": "text", "id": "c", "content": "C" }
            ]
        }));
        let plan = FontPlan::scan(&doc);
        let names: BTreeSet<_> = plan.families().map(|(n, _)| n.to_owned()).collect();
        assert!(names.contains("Inter"));
        assert!(names.contains("Roboto"));
        assert!(names.contains(""));
    }
}
