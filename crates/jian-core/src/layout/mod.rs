//! LayoutEngine — wraps `taffy::TaffyTree` and maps SlotMap keys ↔
//! taffy NodeIds.
//!
//! Text leaves measure through a pluggable [`MeasureBackend`]
//! ([`measure`] module). The default `EstimateBackend` is a
//! character-count heuristic — fast, font-engine-agnostic, accurate
//! to ~10% on Latin script. Hosts that want real shaping (CJK / emoji
//! glyph width, kerning, custom-font metrics) install an alternative
//! backend at runtime via [`Runtime::build_layout_with`]; jian-skia
//! ships `SkiaMeasure` under the `textlayout` cargo feature.
//!
//! Wrapping is governed by the text node's `text_growth` field
//! (`Auto` / `FixedWidth` / `FixedWidthHeight`). See
//! [`measure_text_for_taffy`] for the budget-resolution rules.

pub mod measure;
pub mod resolve;

use crate::document::{NodeKey, NodeTree};
use crate::error::{CoreError, CoreResult};
use crate::geometry::{rect, Rect};
use measure::{
    default_backend, FontStyleKind, MeasureBackend, MeasureRequest, StyledRun,
};
use slotmap::SecondaryMap;
use std::rc::Rc;
use taffy::prelude::*;

/// Per-node measurer context — only populated for Text leaves so the
/// Taffy callback can hand styled segments off to a `MeasureBackend`.
/// `runs` owns its own strings so the context outlives the schema
/// borrow taffy's tree expects.
#[derive(Debug, Clone)]
pub struct TextMeasure {
    pub runs: Vec<OwnedRun>,
    pub line_height: f32, // multiplier; 0.0 → 1.3 default
    pub growth: TextGrowth,
}

/// Mirror of `jian_ops_schema::node::TextGrowth` re-exported into
/// the layout module so the schema dep doesn't leak into measure
/// callsites. Default `Auto` matches the schema default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextGrowth {
    /// Wrap to the container's available width; height grows to
    /// fit. Most common case — body text in a flex column.
    #[default]
    Auto,
    /// Wrap to the node's authored width; height grows to fit. Use
    /// when the author has a fixed column layout.
    FixedWidth,
    /// No wrap; report the natural single-line extent and let the
    /// renderer clip. Use for one-line labels / chips.
    FixedWidthHeight,
}

#[derive(Debug, Clone)]
pub struct OwnedRun {
    pub text: String,
    pub font_family: Option<String>,
    pub font_size: f32,
    pub font_weight: u16,
    pub font_style: FontStyleKind,
    pub letter_spacing: f32,
}

impl OwnedRun {
    fn as_styled(&self) -> StyledRun<'_> {
        StyledRun {
            text: &self.text,
            font_family: self.font_family.as_deref(),
            font_size: self.font_size,
            font_weight: self.font_weight,
            font_style: self.font_style,
            letter_spacing: self.letter_spacing,
        }
    }
}

pub struct LayoutEngine {
    pub(crate) tree: TaffyTree<Option<TextMeasure>>,
    pub(crate) map: SecondaryMap<NodeKey, NodeId>,
    /// Parent-node lookup, mirrored from `NodeTree` so `node_rect` can
    /// accumulate per-parent offsets into an absolute scene coordinate.
    pub(crate) parent: SecondaryMap<NodeKey, NodeKey>,
    pub(crate) measure: Rc<dyn MeasureBackend>,
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self::with_backend(default_backend())
    }

    /// Build with a host-supplied measurement backend. Use this from
    /// hosts that have a real shaper available (e.g. jian-skia's
    /// `SkiaMeasure` under the `textlayout` feature). Headless tests
    /// keep the default `EstimateBackend` via `LayoutEngine::new`.
    pub fn with_backend(measure: Rc<dyn MeasureBackend>) -> Self {
        Self {
            tree: TaffyTree::new(),
            map: SecondaryMap::new(),
            parent: SecondaryMap::new(),
            measure,
        }
    }

    /// Swap the measurement backend in place. Mutates only the
    /// backend slot — `compute()` clones the `Rc` on entry, so a
    /// swap *before* the next `compute()` takes effect on that
    /// pass. The taffy tree + node-id map + parent map are *not*
    /// preserved across `Self::build()` (which always rebuilds
    /// from scratch); this method only matters between a `build()`
    /// + `compute()` pair. Hosts typically call `set_backend` once
    /// at startup, then drive layout via the regular
    /// `build_layout` path.
    pub fn set_backend(&mut self, measure: Rc<dyn MeasureBackend>) {
        self.measure = measure;
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
        let backend = self.measure.clone();
        self.tree
            .compute_layout_with_measure(root, space, |known, avail, _node_id, ctx, _style| {
                // `ctx` is `Option<&mut Option<TextMeasure>>` — taffy
                // gives us the NodeContext slot for the node being
                // measured. Only Text leaves store a populated inner
                // Option; everything else is None.
                if let Some(inner) = ctx {
                    if let Some(tm) = inner.as_ref() {
                        return measure_text_for_taffy(backend.as_ref(), tm, known, avail);
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
/// else. Fans `TextContent::Styled` out into per-segment owned runs
/// so the measure backend can shape each segment with its own
/// weight / size / family — single-string concatenation would
/// diverge from the renderer (which uses ParagraphBuilder
/// `push_style` per segment under the textlayout feature).
fn text_measure_for(n: &jian_ops_schema::node::PenNode) -> Option<TextMeasure> {
    use jian_ops_schema::node::{text::TextContent, PenNode};
    let PenNode::Text(t) = n else {
        return None;
    };
    let node_size = t.font_size.map(|v| v as f32).unwrap_or(14.0);
    let node_weight = resolve_weight(t.font_weight.as_ref());
    let node_style = resolve_style(t.font_style.as_ref());
    let node_family = t.font_family.clone();
    let node_letter_spacing = t.letter_spacing.map(|v| v as f32).unwrap_or(0.0);

    let runs: Vec<OwnedRun> = match &t.content {
        TextContent::Plain(s) => {
            if s.is_empty() {
                return None;
            }
            vec![OwnedRun {
                text: s.clone(),
                font_family: node_family,
                font_size: node_size,
                font_weight: node_weight,
                font_style: node_style,
                letter_spacing: node_letter_spacing,
            }]
        }
        TextContent::Styled(segs) => {
            // `StyledTextSegment` (from `jian_ops_schema::style`) uses
            // a flat `Option<u32>` for weight, the `style::FontStyleKind`
            // enum for italic/normal, and has no per-segment letter
            // spacing. Each segment inherits node-level defaults when
            // its own override is absent.
            let resolved: Vec<OwnedRun> = segs
                .iter()
                .filter(|s| !s.text.is_empty())
                .map(|s| OwnedRun {
                    text: s.text.clone(),
                    font_family: s.font_family.clone().or_else(|| node_family.clone()),
                    font_size: s.font_size.map(|v| v).unwrap_or(node_size),
                    font_weight: s.font_weight.map(|n| n as u16).unwrap_or(node_weight),
                    font_style: match s.font_style {
                        Some(jian_ops_schema::style::FontStyleKind::Italic) => {
                            FontStyleKind::Italic
                        }
                        Some(jian_ops_schema::style::FontStyleKind::Normal) => {
                            FontStyleKind::Normal
                        }
                        None => node_style,
                    },
                    letter_spacing: node_letter_spacing,
                })
                .collect();
            if resolved.is_empty() {
                return None;
            }
            resolved
        }
    };

    let line_height = t.line_height.map(|v| v as f32).unwrap_or(0.0);
    let growth = match t.text_growth {
        Some(jian_ops_schema::node::TextGrowth::FixedWidth) => TextGrowth::FixedWidth,
        Some(jian_ops_schema::node::TextGrowth::FixedWidthHeight) => {
            TextGrowth::FixedWidthHeight
        }
        Some(jian_ops_schema::node::TextGrowth::Auto) | None => TextGrowth::Auto,
    };
    Some(TextMeasure {
        runs,
        line_height,
        growth,
    })
}

fn resolve_weight(w: Option<&jian_ops_schema::node::FontWeight>) -> u16 {
    use jian_ops_schema::node::text::FontWeight;
    match w {
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
    }
}

fn resolve_style(s: Option<&jian_ops_schema::node::TextFontStyle>) -> FontStyleKind {
    use jian_ops_schema::node::TextFontStyle;
    match s {
        Some(TextFontStyle::Italic) => FontStyleKind::Italic,
        _ => FontStyleKind::Normal,
    }
}

/// Taffy callback: given the text node's context + container's known
/// dimensions + available space, hand off to the measure backend.
///
/// The `text_growth` field on the node decides how the wrap budget
/// is computed:
/// - `Auto`: use the container's available width (default).
/// - `FixedWidth`: use the node's authored width *only*. When the
///   node was authored as `width: auto` the budget falls back to
///   a *definite* available width (taffy's `MinContent` /
///   `MaxContent` intrinsic probes pass through as `None`, so the
///   backend reports natural extent during sizing rounds) — same
///   effective behaviour as `Auto` in the definite corner, since
///   there's no fixed budget to honour. Authors who want a hard
///   wrap to the container should use `Auto`; `FixedWidth` is
///   intended for nodes with an explicit numeric width.
/// - `FixedWidthHeight`: no wrap; report the natural single-line
///   extent. The renderer is responsible for clipping at the
///   authored bounds.
fn measure_text_for_taffy(
    backend: &dyn MeasureBackend,
    tm: &TextMeasure,
    known: Size<Option<f32>>,
    avail: Size<AvailableSpace>,
) -> Size<f32> {
    let runs: Vec<StyledRun<'_>> = tm.runs.iter().map(|r| r.as_styled()).collect();

    let max_width = match tm.growth {
        // Hard "no wrap" — the renderer clips at the authored
        // bounds; we report natural single-line extent.
        TextGrowth::FixedWidthHeight => None,
        // FixedWidth honours an *authored* width when taffy
        // resolved it (passed in via `known.width`); otherwise the
        // node has no authoritative budget and we fall back to
        // available — matches Auto's behaviour in that corner.
        TextGrowth::FixedWidth => match known.width {
            Some(w) => Some(w),
            None => match avail.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            },
        },
        // Auto wraps to the container's available width.
        TextGrowth::Auto => match (known.width, avail.width) {
            (Some(w), _) => Some(w),
            (None, AvailableSpace::Definite(w)) => Some(w),
            _ => None,
        },
    };

    let req = MeasureRequest {
        runs: &runs,
        line_height: tm.line_height,
        max_width,
    };
    let res = backend.measure(&req);
    let width = match known.width {
        Some(w) => w,
        None => res.width,
    };
    let height = known.height.unwrap_or(res.height);
    Size { width, height }
}
