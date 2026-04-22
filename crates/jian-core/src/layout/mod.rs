//! LayoutEngine — wraps `taffy::TaffyTree` and maps SlotMap keys ↔ taffy NodeIds.

pub mod measure;
pub mod resolve;

use crate::document::{NodeKey, NodeTree};
use crate::error::{CoreError, CoreResult};
use crate::geometry::{rect, Rect};
use slotmap::SecondaryMap;
use taffy::prelude::*;

/// Per-node measurer context — only populated for Text leaves so that
/// taffy can size them based on the content string + font size.
#[derive(Debug, Clone)]
pub struct TextMeasure {
    pub content: String,
    pub font_size: f32,
    /// CSS font weight — heavier faces widen glyphs, so the measure
    /// heuristic varies the per-glyph ratio by weight.
    pub font_weight: u16,
    pub line_height: f32, // multiplier; 0.0 → 1.3 default
}

pub struct LayoutEngine {
    pub(crate) tree: TaffyTree<Option<TextMeasure>>,
    pub(crate) map: SecondaryMap<NodeKey, NodeId>,
    /// Parent-node lookup, mirrored from `NodeTree` so `node_rect` can
    /// accumulate per-parent offsets into an absolute scene coordinate.
    pub(crate) parent: SecondaryMap<NodeKey, NodeKey>,
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self {
            tree: TaffyTree::new(),
            map: SecondaryMap::new(),
            parent: SecondaryMap::new(),
        }
    }

    /// Build a taffy tree mirroring the NodeTree. Returns the root NodeIds.
    pub fn build(&mut self, doc_tree: &NodeTree) -> CoreResult<Vec<NodeId>> {
        self.tree = TaffyTree::new();
        self.map = SecondaryMap::new();
        self.parent = SecondaryMap::new();

        // Pass 1: create a taffy node for each doc node. `node_to_style`
        // handles both containers (Frame/Group/Rectangle) and leaves
        // (Text / IconFont / Image / …) so leaf sizes propagate into
        // flex measurements.
        for (key, data) in doc_tree.nodes.iter() {
            let style = resolve::node_to_style(&data.schema);
            let ctx = text_measure_for(&data.schema);
            let id = self
                .tree
                .new_leaf_with_context(style, ctx)
                .map_err(|e| CoreError::Layout(e.to_string()))?;
            self.map.insert(key, id);
            if let Some(p) = data.parent {
                self.parent.insert(key, p);
            }
        }

        // Pass 2: wire parent/child relationships.
        for (key, data) in doc_tree.nodes.iter() {
            if !data.children.is_empty() {
                let parent = self.map[key];
                let child_ids: Vec<NodeId> = data.children.iter().map(|k| self.map[*k]).collect();
                self.tree
                    .set_children(parent, &child_ids)
                    .map_err(|e| CoreError::Layout(e.to_string()))?;
            }
        }

        Ok(doc_tree.roots.iter().map(|k| self.map[*k]).collect())
    }

    pub fn compute(&mut self, root: NodeId, available: (f32, f32)) -> CoreResult<()> {
        let space = Size {
            width: AvailableSpace::Definite(available.0),
            height: AvailableSpace::Definite(available.1),
        };
        self.tree
            .compute_layout_with_measure(root, space, |known, avail, _node_id, ctx, _style| {
                // `ctx` is `Option<&mut Option<TextMeasure>>` — taffy
                // gives us the NodeContext slot for the node being
                // measured. Only Text leaves store a populated inner
                // Option; everything else is None.
                if let Some(inner) = ctx {
                    if let Some(tm) = inner.as_ref() {
                        return measure_text_for_taffy(tm, known, avail);
                    }
                }
                Size::ZERO
            })
            .map_err(|e| CoreError::Layout(e.to_string()))
    }

    /// Absolute scene-coord rect for `key`: taffy's `layout.location` is
    /// relative to the node's flex parent, so we walk up the parent
    /// chain and accumulate each ancestor's location offset.
    pub fn node_rect(&self, key: NodeKey) -> Option<Rect> {
        let id = self.map.get(key)?;
        let l = self.tree.layout(*id).ok()?;
        let (mut ax, mut ay) = (l.location.x, l.location.y);
        let (w, h) = (l.size.width, l.size.height);
        let mut cur = key;
        while let Some(&p) = self.parent.get(cur) {
            let pid = self.map.get(p)?;
            let pl = self.tree.layout(*pid).ok()?;
            ax += pl.location.x;
            ay += pl.location.y;
            cur = p;
        }
        Some(rect(ax, ay, w, h))
    }
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a `TextMeasure` context for Text nodes, None for everything
/// else. Extracts the plain-string content + font size from the schema
/// via the typed accessor (styled segments are concatenated to their
/// `.text` field for measurement purposes).
fn text_measure_for(n: &jian_ops_schema::node::PenNode) -> Option<TextMeasure> {
    use jian_ops_schema::node::{
        text::{FontWeight, TextContent},
        PenNode,
    };
    let PenNode::Text(t) = n else {
        return None;
    };
    let content = match &t.content {
        TextContent::Plain(s) => s.clone(),
        TextContent::Styled(segs) => segs
            .iter()
            .map(|s| s.text.clone())
            .collect::<Vec<_>>()
            .join(""),
    };
    if content.is_empty() {
        return None;
    }
    let font_size = t.font_size.map(|v| v as f32).unwrap_or(14.0);
    let font_weight = match &t.font_weight {
        Some(FontWeight::Number(n)) => *n as u16,
        Some(FontWeight::Keyword(s)) => match s.as_str() {
            "bold" => 700,
            "semibold" | "semi-bold" => 600,
            "medium" => 500,
            "light" => 300,
            "thin" => 100,
            _ => 400,
        },
        None => 400,
    };
    let line_height = t.line_height.map(|v| v as f32).unwrap_or(0.0);
    Some(TextMeasure {
        content,
        font_size,
        font_weight,
        line_height,
    })
}

/// Taffy callback: given the text node's context + container's known
/// dimensions + available space, return the estimated size.
fn measure_text_for_taffy(
    tm: &TextMeasure,
    known: Size<Option<f32>>,
    avail: Size<AvailableSpace>,
) -> Size<f32> {
    // Natural (single-line) metrics from our character-count heuristic.
    let (natural_w, _natural_h) =
        measure::estimate_text_size_weighted(&tm.content, tm.font_size, tm.font_weight);
    let line_mult = if tm.line_height > 0.0 {
        tm.line_height
    } else {
        1.3
    };

    // Width: prefer known, else clamp to available, else natural.
    let width = match known.width {
        Some(w) => w,
        None => match avail.width {
            AvailableSpace::Definite(w) => natural_w.min(w),
            _ => natural_w,
        },
    };

    // Height: if the text wraps into the available width we need to
    // project the natural width onto the available width in lines.
    let lines = if natural_w > width + 0.5 && width > 0.0 {
        (natural_w / width).ceil().max(1.0)
    } else {
        1.0
    };
    let height = match known.height {
        Some(h) => h,
        None => tm.font_size * line_mult * lines,
    };
    Size { width, height }
}
