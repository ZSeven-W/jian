//! LayoutEngine — wraps `taffy::TaffyTree` and maps SlotMap keys ↔
//! taffy NodeIds.
//!
//! Text leaves measure through a pluggable [`measure::MeasureBackend`].
//! The default [`measure::EstimateBackend`] is a character-count
//! heuristic — fast, font-engine-agnostic, accurate to ~10% on Latin
//! script. Hosts that want real shaping (CJK / emoji glyph width,
//! kerning, custom-font metrics) install an alternative backend at
//! runtime via `Runtime::build_layout_with`; jian-skia ships
//! `SkiaMeasure` under the `textlayout` cargo feature.
//!
//! Wrapping is governed by the text node's `text_growth` field
//! (`Auto` / `FixedWidth` / `FixedWidthHeight`); the budget-resolution
//! rules live in the private `measure_text_for_taffy` callback.

pub mod constraints;
pub mod measure;
pub mod resolve;
mod responsive;

use crate::document::{NodeKey, NodeTree};
use crate::error::{CoreError, CoreResult};
use crate::geometry::{rect, Rect};
use jian_ops_schema::pack::initial_layout::InitialLayoutSnapshot;
use measure::{default_backend, FontStyleKind, MeasureBackend, MeasureRequest, StyledRun};
use slotmap::SecondaryMap;
#[cfg(test)]
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use taffy::prelude::*;

/// Per-node measurer context — populated for text-like leaves so the
/// Taffy callback can hand styled segments off to a `MeasureBackend`.
/// `runs` owns its own strings so the context outlives the schema
/// borrow taffy's tree expects.
#[derive(Debug, Clone)]
pub struct TextMeasure {
    pub runs: Vec<OwnedRun>,
    pub line_height: f32, // multiplier; 0.0 → 1.3 default
    pub growth: TextGrowth,
    pub input_chrome: Option<InputChromeMeasure>,
    pub checkbox_chrome: Option<CheckboxChromeMeasure>,
}

/// Static input anatomy used by text_input / number_input / select
/// fit-content measurement. Values intentionally match the scene
/// painter's input inset constants: pad 8, icon box 20.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputChromeMeasure {
    pub leading_icon: bool,
    pub trailing_icon: bool,
}

/// Checkbox anatomy used for labelled checkbox fit-content measurement.
/// The 18px indicator plus 8px label gap mirrors the scene painter; unlike an
/// input this chrome adds no field padding and no 36px minimum height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckboxChromeMeasure;

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

const INPUT_PAD_X: f32 = 8.0;
const INPUT_ICON_BOX: f32 = 20.0;
const CHECKBOX_INDICATOR: f32 = 18.0;
const CHECKBOX_LABEL_GAP: f32 = 8.0;

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
    /// Plan 19 D1 cold-start: absolute scene-coord rects loaded from
    /// `aot/initial_layout.bin` ahead of any taffy compute pass.
    /// `node_rect` short-circuits against this map so the first paint
    /// can read pre-baked geometry without touching taffy. Cleared on
    /// the next `build()` because a relayout invalidates these rects.
    pub(crate) preload: SecondaryMap<NodeKey, Rect>,
    pub(crate) reference: Option<constraints::ReferenceTable>,
    pub(crate) constraint_lints: Vec<String>,
    pub(crate) node_order: Vec<NodeKey>,
    pub(crate) overrides: SecondaryMap<NodeKey, responsive::ResolvedBox>,
    pub(crate) compute_count: usize,
    pub(crate) bound_hit: bool,
    pub(crate) root_owner: SecondaryMap<NodeKey, NodeKey>,
    /// Authored document-root origins. Taffy has no containing block for a
    /// root, so it drops root `x`/`y`; `node_rect` restores that offset after
    /// accumulating the computed parent chain.
    root_origins: SecondaryMap<NodeKey, (f32, f32)>,
    pub(crate) base_styles: SecondaryMap<NodeKey, Style>,
    origin_normalized: HashSet<NodeKey>,
    #[cfg(test)]
    fail_next_staged_build: Cell<bool>,
}

pub struct StagedLayout {
    pub(crate) engine: LayoutEngine,
    pub(crate) roots: Vec<NodeId>,
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
            preload: SecondaryMap::new(),
            reference: None,
            constraint_lints: Vec::new(),
            node_order: Vec::new(),
            overrides: SecondaryMap::new(),
            compute_count: 0,
            bound_hit: false,
            root_owner: SecondaryMap::new(),
            root_origins: SecondaryMap::new(),
            base_styles: SecondaryMap::new(),
            origin_normalized: HashSet::new(),
            #[cfg(test)]
            fail_next_staged_build: Cell::new(false),
        }
    }

    /// Swap the measurement backend in place. Mutates only the
    /// backend slot — `compute()` clones the `Rc` on entry, so a
    /// swap *before* the next `compute()` takes effect on that
    /// pass. The taffy tree + node-id map + parent map are *not*
    /// preserved across `Self::build()` (which always rebuilds
    /// from scratch); this method only matters between a `build()`
    /// and `compute()` pair. Hosts typically call `set_backend`
    /// once at startup, then drive layout via the regular
    /// `build_layout` path.
    pub fn set_backend(&mut self, measure: Rc<dyn MeasureBackend>) {
        self.measure = measure;
    }

    /// Build a fresh tree aside. The receiver remains usable if any build or
    /// later compute step fails; callers install only after the transaction
    /// has completed successfully.
    pub fn build_staged(&self, doc: &crate::document::RuntimeDocument) -> CoreResult<StagedLayout> {
        #[cfg(test)]
        if self.fail_next_staged_build.replace(false) {
            return Err(CoreError::Layout("injected staged build failure".into()));
        }
        let mut engine = Self::with_backend(self.measure.clone());
        let roots = engine.build_responsive(&doc.tree, doc.schema.is_responsive())?;
        Ok(StagedLayout { engine, roots })
    }

    pub fn install(&mut self, staged: StagedLayout) {
        *self = staged.engine;
    }

    #[cfg(test)]
    pub(crate) fn inject_staged_build_failure(&self) {
        self.fail_next_staged_build.set(true);
    }

    /// Build a taffy tree mirroring the NodeTree. Returns the root NodeIds.
    pub fn build(&mut self, doc_tree: &NodeTree) -> CoreResult<Vec<NodeId>> {
        self.build_with_mode(doc_tree, false)
    }

    fn build_with_mode(
        &mut self,
        doc_tree: &NodeTree,
        responsive: bool,
    ) -> CoreResult<Vec<NodeId>> {
        // A real taffy compute supersedes any preload snapshot —
        // post-compute `node_rect` reads must come from taffy, not
        // stale AOT geometry. Plan 19 D1.
        self.preload = SecondaryMap::new();
        self.tree = TaffyTree::new();
        self.map = SecondaryMap::new();
        self.parent = SecondaryMap::new();
        self.reference = None;
        self.constraint_lints.clear();
        self.node_order.clear();
        self.overrides = SecondaryMap::new();
        self.compute_count = 0;
        self.bound_hit = false;
        self.root_owner = SecondaryMap::new();
        self.root_origins = SecondaryMap::new();
        self.base_styles = SecondaryMap::new();
        self.origin_normalized.clear();
        self.node_order = doc_tree.keys_top_down();
        for &root in &doc_tree.roots {
            if let Some(origin) = doc_tree
                .nodes
                .get(root)
                .and_then(|node| resolve::explicit_position(&node.schema))
            {
                self.root_origins.insert(root, origin);
            }
            let mut stack = vec![root];
            while let Some(key) = stack.pop() {
                // First owner wins; a duplicate cross-root child or cycle
                // is malformed input — skip instead of looping/overwriting.
                if self.root_owner.insert(key, root).is_some() {
                    debug_assert!(false, "node reachable from multiple roots or cycle");
                    continue;
                }
                stack.extend(doc_tree.nodes[key].children.iter().rev().copied());
            }
        }

        // Pass 1: create a taffy node for each doc node. `node_to_style`
        // handles both containers (Frame/Group/Rectangle) and leaves
        // (Text / IconFont / Image / …) so leaf sizes propagate into
        // flex measurements.
        for (key, data) in doc_tree.nodes.iter() {
            let mut style = if responsive {
                resolve::node_to_style_responsive(&data.schema, &mut self.constraint_lints)
            } else {
                resolve::node_to_style(&data.schema)
            };
            // Direction-aware flex_shrink: a child whose MAIN-AXIS size is a
            // fixed Number must not be shrunk below it. `node_to_style` only
            // pins fully-fixed squares (both axes Number); here, with the parent
            // in hand, we also pin a `width:200, height:fit_content` card in a
            // horizontal scroll row so it keeps its width — its title doesn't
            // wrap and sibling cards stay equal-height (otherwise an overflowing
            // row squeezes the cards into ragged, uneven heights).
            if let Some(p) = data.parent {
                if resolve::node_is_stack_container(&doc_tree.nodes[p].schema) {
                    // `layout: "none"` parent: children share one grid cell
                    // and overlap. The flex hints below all assume a flow
                    // line and would fight the stack (`main_axis` has no
                    // meaning when every child starts at the same origin).
                    resolve::apply_stack_child(&mut style);
                } else {
                    let parent_horizontal = resolve::node_is_horizontal(&doc_tree.nodes[p].schema);
                    if resolve::main_axis_is_fixed_number(&data.schema, parent_horizontal) {
                        style.flex_shrink = 0.0;
                    }
                    // A fill-height child in a row stretches to the row's height
                    // (so a space_between sidebar's footer reaches the bottom)
                    // instead of collapsing on an indefinite-height parent.
                    resolve::apply_fill_container_axes(&mut style, &data.schema, parent_horizontal);
                }
            }
            self.base_styles.insert(key, style.clone());
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

    /// Build layout plus the authored reference geometry used by responsive constraints.
    pub fn build_responsive(
        &mut self,
        doc_tree: &NodeTree,
        responsive: bool,
    ) -> CoreResult<Vec<NodeId>> {
        let roots = self.build_with_mode(doc_tree, responsive)?;
        if responsive {
            let (reference, lints) = constraints::ReferenceTable::build(doc_tree);
            self.reference = Some(reference);
            self.constraint_lints.extend(lints);
        }
        Ok(roots)
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

    /// Plan 19 D1 cold-start: load pre-computed first-frame rects from
    /// `aot/initial_layout.bin` so the very first paint can skip
    /// `compute_layout_with_measure`. The host calls this between
    /// `Runtime::new_from_document` and the first `node_rect` read.
    ///
    /// Each id in the snapshot is resolved against `doc_tree.by_id`;
    /// ids absent from the document are silently dropped (the writer
    /// runs ahead of any document-level mutation, but a `.op.pack`
    /// landing on a slightly newer schema must not panic). Returns
    /// the count of rects that were resolved + populated.
    ///
    /// The next `build()` clears the preload, so a relayout caused by
    /// resize / hot-reload falls back to taffy compute as usual.
    /// Hosts that want to keep the preload across resizes should
    /// re-call `preload_initial` after the resize-driven `build()`.
    pub fn preload_initial(
        &mut self,
        snapshot: &InitialLayoutSnapshot,
        doc_tree: &NodeTree,
    ) -> usize {
        // Codex follow-up reminder: this is a wholesale replacement,
        // not a merge — a stale preload from a previous `.op.pack`
        // load would otherwise serve rects from the old document.
        self.preload = SecondaryMap::new();
        let mut count = 0usize;
        for (id, packed) in snapshot.rects.iter() {
            let Some(key) = doc_tree.by_id.get(id).copied() else {
                continue;
            };
            let (x, y, w, h) = packed.into_xywh();
            self.preload.insert(key, rect(x, y, w, h));
            count += 1;
        }
        count
    }

    /// True when [`Self::preload_initial`] has populated at least one
    /// rect and no `build()` has cleared the cache since. Hosts use
    /// this to decide whether to skip the `ComputeFirstLayout`
    /// startup phase. **Prefer [`Self::preload_covers`]** when
    /// deciding to short-circuit a real layout pass — partial
    /// coverage paired with a `ComputeFirstLayout` skip would leave
    /// new doc nodes rect-less (codex round 1 MEDIUM).
    pub fn has_preload(&self) -> bool {
        !self.preload.is_empty()
    }

    /// Number of preload entries currently cached. `0` after
    /// `build()` (which clears the preload) or before any call to
    /// [`Self::preload_initial`].
    pub fn preload_len(&self) -> usize {
        self.preload.len()
    }

    /// True when the cached preload covers every node in `doc_tree`.
    /// The bootstrap path uses this to gate the `ComputeFirstLayout`
    /// short-circuit: a partial preload (older `.op.pack` + newer
    /// `.op` schema, slot keys reused) must NOT skip compute, or
    /// the new nodes serve `None` from `node_rect` and disappear
    /// from first-frame spatial / render paths. Plan 19 D1 codex
    /// round 1 MEDIUM.
    pub fn preload_covers(&self, doc_tree: &NodeTree) -> bool {
        if self.preload.len() != doc_tree.nodes.len() {
            return false;
        }
        // Length parity isn't enough: SecondaryMap uses the same
        // SlotMap key space as `doc_tree.nodes`, but a stale preload
        // could in theory carry equal-count keys that don't all
        // belong to the current doc. Verify each doc key has a
        // preload entry.
        doc_tree
            .nodes
            .keys()
            .all(|key| self.preload.contains_key(key))
    }

    /// Drop the cached preload without running a real compute pass.
    /// Used by the bootstrap when the preload coverage is incomplete:
    /// the partial cache must not poison the next `node_rect` read,
    /// and the host runs `build_layout` to populate taffy from
    /// scratch. Plan 19 D1 codex round 1 MEDIUM.
    pub fn drop_preload(&mut self) {
        self.preload = SecondaryMap::new();
    }

    /// Absolute scene-coord rect for `key`: taffy's `layout.location` is
    /// relative to the node's flex parent, so we walk up the parent
    /// chain and accumulate each ancestor's location offset.
    ///
    /// Cycle-bound the walk at the parent map's len: a legitimate
    /// ancestor chain is at most that long. The map is `pub(crate)`
    /// so a logic bug elsewhere could in principle install a cycle;
    /// detecting it returns `None` (same shape as the existing
    /// "missing layout" early-out) rather than hanging every paint
    /// frame.
    pub fn node_rect(&self, key: NodeKey) -> Option<Rect> {
        // Plan 19 D1: preload short-circuit. Snapshot rects are
        // already in absolute scene coords (mirroring `node_rect`'s
        // post-compute output), so no parent-chain walk is needed.
        if let Some(r) = self.preload.get(key) {
            return Some(*r);
        }
        let id = self.map.get(key)?;
        let l = self.tree.layout(*id).ok()?;
        let (mut ax, mut ay) = (l.location.x, l.location.y);
        let (w, h) = (l.size.width, l.size.height);
        let mut cur = key;
        let max_steps = self.parent.len();
        let mut steps = 0usize;
        while let Some(&p) = self.parent.get(cur) {
            if steps > max_steps {
                return None;
            }
            let pid = self.map.get(p)?;
            let pl = self.tree.layout(*pid).ok()?;
            ax += pl.location.x;
            ay += pl.location.y;
            cur = p;
            steps += 1;
        }
        if !self.is_origin_normalized(cur) {
            if let Some((x, y)) = self.root_origins.get(cur) {
                ax += x;
                ay += y;
            }
        }
        Some(rect(ax, ay, w, h))
    }

    /// Absolute scene-coordinate rect used by runtime hit-testing and host
    /// overlays. `node_rect` already restores a non-responsive document
    /// root's authored origin; responsive viewport roots are normalized to
    /// `(0, 0)` by `override_root_for_viewport`.
    pub(crate) fn node_scene_rect(
        &self,
        doc: &crate::document::RuntimeDocument,
        key: NodeKey,
    ) -> Option<Rect> {
        doc.tree.nodes.get(key)?;
        self.node_rect(key)
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
    use jian_ops_schema::node::{base::NumberOrExpression, text::TextContent, PenNode};

    fn plain_input_measure(text: String, leading_icon: bool, trailing_icon: bool) -> TextMeasure {
        TextMeasure {
            runs: vec![OwnedRun {
                text,
                font_family: None,
                font_size: 14.0,
                font_weight: 400,
                font_style: FontStyleKind::Normal,
                letter_spacing: 0.0,
            }],
            line_height: 0.0,
            growth: TextGrowth::Auto,
            input_chrome: Some(InputChromeMeasure {
                leading_icon,
                trailing_icon,
            }),
            checkbox_chrome: None,
        }
    }

    fn checkbox_measure(label: String) -> TextMeasure {
        TextMeasure {
            runs: vec![OwnedRun {
                text: label,
                font_family: None,
                font_size: 14.0,
                font_weight: 400,
                font_style: FontStyleKind::Normal,
                letter_spacing: 0.0,
            }],
            // The adjacent label painter uses a 14px one-line box rather
            // than input/body text's 1.3x default leading.
            line_height: 1.0,
            growth: TextGrowth::Auto,
            input_chrome: None,
            checkbox_chrome: Some(CheckboxChromeMeasure),
        }
    }

    let PenNode::Text(t) = n else {
        return match n {
            PenNode::TextInput(input) => {
                let text = input
                    .value
                    .clone()
                    .or_else(|| input.placeholder.clone())
                    .unwrap_or_default();
                Some(plain_input_measure(
                    text,
                    input.leading_icon.is_some(),
                    input.trailing_icon.is_some(),
                ))
            }
            PenNode::NumberInput(input) => {
                let text = input
                    .value
                    .as_ref()
                    .and_then(|value| match value {
                        NumberOrExpression::Number(n) => Some(format!("{n}")),
                        NumberOrExpression::Expression(_) => None,
                    })
                    .or_else(|| input.placeholder.clone())
                    .unwrap_or_default();
                Some(plain_input_measure(
                    text,
                    input.leading_icon.is_some(),
                    input.trailing_icon.is_some(),
                ))
            }
            PenNode::Select(select) => {
                let text = select
                    .value
                    .clone()
                    .or_else(|| select.placeholder.clone())
                    .unwrap_or_default();
                Some(plain_input_measure(text, false, true))
            }
            PenNode::Checkbox(checkbox) => checkbox
                .label
                .as_ref()
                .filter(|label| !label.is_empty())
                .map(|label| checkbox_measure(label.clone())),
            _ => None,
        };
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
                    font_size: s.font_size.unwrap_or(node_size),
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

    let line_height = t
        .layout_line_height_multiplier()
        .map(|v| v as f32)
        .unwrap_or(0.0);
    let growth = match t.text_growth {
        Some(jian_ops_schema::node::TextGrowth::FixedWidth) => TextGrowth::FixedWidth,
        Some(jian_ops_schema::node::TextGrowth::FixedWidthHeight) => TextGrowth::FixedWidthHeight,
        Some(jian_ops_schema::node::TextGrowth::Auto) | None => TextGrowth::Auto,
    };
    Some(TextMeasure {
        runs,
        line_height,
        growth,
        input_chrome: None,
        checkbox_chrome: None,
    })
}

fn resolve_weight(w: Option<&jian_ops_schema::node::FontWeight>) -> u16 {
    use jian_ops_schema::node::text::FontWeight;
    match w {
        Some(FontWeight::Number(n)) => *n as u16,
        Some(FontWeight::Keyword(s)) => {
            // Real-world `.op` files emit `"fontWeight":"700"` etc. as
            // STRINGS (the canonical schema's untagged enum picks
            // `Keyword(String)` when the value is a JSON string,
            // even when its contents are numeric). Try numeric
            // parse first so 100..900 round-trip, then fall back
            // to lucide-style keywords.
            if let Ok(n) = s.parse::<u16>() {
                return n;
            }
            match s.as_str() {
                "bold" => 700,
                "semibold" | "semi-bold" | "demibold" => 600,
                "medium" => 500,
                "normal" | "regular" => 400,
                "light" => 300,
                "extralight" | "extra-light" | "ultralight" | "ultra-light" => 200,
                "thin" | "hairline" => 100,
                "black" | "heavy" => 900,
                "extrabold" | "extra-bold" | "ultrabold" | "ultra-bold" => 800,
                _ => 400,
            }
        }
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
        // Auto is content-sized and NEVER wraps (Pencil semantics: a
        // `textGrowth: auto` node reports its natural single-line extent;
        // designs that want wrapping author `fixed-width`). Wrapping it to
        // the container split single-line labels / subtitles onto two lines,
        // inflating the measured box while the painter (which honours
        // `text_wrap = false` for auto) still drew one line — so every
        // section holding auto text grew taller than Pencil's.
        TextGrowth::Auto => None,
    };

    let req = MeasureRequest {
        runs: &runs,
        line_height: tm.line_height,
        max_width,
    };
    let res = backend.measure(&req);
    let (measured_width, measured_height) = if tm.checkbox_chrome.is_some() {
        (
            CHECKBOX_INDICATOR + CHECKBOX_LABEL_GAP + res.width,
            res.height.max(CHECKBOX_INDICATOR),
        )
    } else if let Some(chrome) = tm.input_chrome {
        let left = if chrome.leading_icon {
            INPUT_PAD_X + INPUT_ICON_BOX + INPUT_PAD_X
        } else {
            INPUT_PAD_X
        };
        let right = if chrome.trailing_icon {
            INPUT_PAD_X + INPUT_ICON_BOX + INPUT_PAD_X
        } else {
            INPUT_PAD_X
        };
        (
            left + res.width + right,
            res.height.max(INPUT_ICON_BOX) + INPUT_PAD_X * 2.0,
        )
    } else {
        (res.width, res.height)
    };
    let width = match known.width {
        Some(w) => w,
        None => measured_width,
    };
    let height = known.height.unwrap_or(measured_height);
    Size { width, height }
}

#[cfg(test)]
mod preload_tests;
