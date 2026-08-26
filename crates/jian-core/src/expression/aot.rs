//! `Chunk` ↔ `PackedChunk` glue (Plan 19 D2).
//!
//! The wire-stable mirror types live in `jian_ops_schema::pack::
//! expressions`; this module converts between them and the live
//! `jian_core::expression::bytecode` types so the AOT writer + reader
//! never need to import each other's internals.
//!
//! Conversion is total in both directions — there's no fallible
//! state because the mirror enum's variant set matches `OpCode`'s
//! variant set 1:1 (a pinned-tag invariant tested in
//! `pack::expressions::tests::tag_assignments_are_pinned`).

use super::bytecode::{Chunk, OpCode};
use super::cache::ExpressionCache;
use jian_ops_schema::document::PenDocument;
use jian_ops_schema::events::{Action, ActionList, Bindings, EventHandlers};
use jian_ops_schema::lifecycle::{AppLifecycleHooks, NodeLifecycleHooks, PageLifecycleHooks};
use jian_ops_schema::node::base::{BoolOrExpression, NumberOrExpression};
use jian_ops_schema::node::PenNode;
use jian_ops_schema::pack::{ExpressionsSnapshot, PackedChunk, PackedOpCode};
use serde_json::Value;
use std::collections::BTreeMap;

impl From<&OpCode> for PackedOpCode {
    fn from(op: &OpCode) -> Self {
        match op {
            OpCode::PushNum(n) => PackedOpCode::PushNum(*n),
            OpCode::PushBool(b) => PackedOpCode::PushBool(*b),
            OpCode::PushNull => PackedOpCode::PushNull,
            OpCode::PushString(i) => PackedOpCode::PushString(*i),
            OpCode::PushScopeRef(i) => PackedOpCode::PushScopeRef(*i),
            OpCode::MakeArray(i) => PackedOpCode::MakeArray(*i),
            OpCode::MakeObject(i) => PackedOpCode::MakeObject(*i),
            OpCode::PushObjectKey(i) => PackedOpCode::PushObjectKey(*i),
            OpCode::MemberGet(i) => PackedOpCode::MemberGet(*i),
            OpCode::IndexGet => PackedOpCode::IndexGet,
            OpCode::Not => PackedOpCode::Not,
            OpCode::Negate => PackedOpCode::Negate,
            OpCode::UnaryPlus => PackedOpCode::UnaryPlus,
            OpCode::Add => PackedOpCode::Add,
            OpCode::Sub => PackedOpCode::Sub,
            OpCode::Mul => PackedOpCode::Mul,
            OpCode::Div => PackedOpCode::Div,
            OpCode::Mod => PackedOpCode::Mod,
            OpCode::Eq => PackedOpCode::Eq,
            OpCode::NotEq => PackedOpCode::NotEq,
            OpCode::EqStrict => PackedOpCode::EqStrict,
            OpCode::NotEqStrict => PackedOpCode::NotEqStrict,
            OpCode::Lt => PackedOpCode::Lt,
            OpCode::Gt => PackedOpCode::Gt,
            OpCode::LtEq => PackedOpCode::LtEq,
            OpCode::GtEq => PackedOpCode::GtEq,
            OpCode::JumpIfFalse(o) => PackedOpCode::JumpIfFalse(*o),
            OpCode::JumpIfTrue(o) => PackedOpCode::JumpIfTrue(*o),
            OpCode::Jump(o) => PackedOpCode::Jump(*o),
            OpCode::NullCoalesce => PackedOpCode::NullCoalesce,
            OpCode::TemplateAppend => PackedOpCode::TemplateAppend,
            OpCode::CallBuiltin(name, argc) => PackedOpCode::CallBuiltin(*name, *argc),
            OpCode::Return => PackedOpCode::Return,
        }
    }
}

impl From<&PackedOpCode> for OpCode {
    fn from(op: &PackedOpCode) -> Self {
        match op {
            PackedOpCode::PushNum(n) => OpCode::PushNum(*n),
            PackedOpCode::PushBool(b) => OpCode::PushBool(*b),
            PackedOpCode::PushNull => OpCode::PushNull,
            PackedOpCode::PushString(i) => OpCode::PushString(*i),
            PackedOpCode::PushScopeRef(i) => OpCode::PushScopeRef(*i),
            PackedOpCode::MakeArray(i) => OpCode::MakeArray(*i),
            PackedOpCode::MakeObject(i) => OpCode::MakeObject(*i),
            PackedOpCode::PushObjectKey(i) => OpCode::PushObjectKey(*i),
            PackedOpCode::MemberGet(i) => OpCode::MemberGet(*i),
            PackedOpCode::IndexGet => OpCode::IndexGet,
            PackedOpCode::Not => OpCode::Not,
            PackedOpCode::Negate => OpCode::Negate,
            PackedOpCode::UnaryPlus => OpCode::UnaryPlus,
            PackedOpCode::Add => OpCode::Add,
            PackedOpCode::Sub => OpCode::Sub,
            PackedOpCode::Mul => OpCode::Mul,
            PackedOpCode::Div => OpCode::Div,
            PackedOpCode::Mod => OpCode::Mod,
            PackedOpCode::Eq => OpCode::Eq,
            PackedOpCode::NotEq => OpCode::NotEq,
            PackedOpCode::EqStrict => OpCode::EqStrict,
            PackedOpCode::NotEqStrict => OpCode::NotEqStrict,
            PackedOpCode::Lt => OpCode::Lt,
            PackedOpCode::Gt => OpCode::Gt,
            PackedOpCode::LtEq => OpCode::LtEq,
            PackedOpCode::GtEq => OpCode::GtEq,
            PackedOpCode::JumpIfFalse(o) => OpCode::JumpIfFalse(*o),
            PackedOpCode::JumpIfTrue(o) => OpCode::JumpIfTrue(*o),
            PackedOpCode::Jump(o) => OpCode::Jump(*o),
            PackedOpCode::NullCoalesce => OpCode::NullCoalesce,
            PackedOpCode::TemplateAppend => OpCode::TemplateAppend,
            PackedOpCode::CallBuiltin(name, argc) => OpCode::CallBuiltin(*name, *argc),
            PackedOpCode::Return => OpCode::Return,
        }
    }
}

impl From<&Chunk> for PackedChunk {
    fn from(c: &Chunk) -> Self {
        PackedChunk {
            ops: c.ops.iter().map(PackedOpCode::from).collect(),
            strings: c.strings.clone(),
            scope_paths: c.scope_paths.clone(),
        }
    }
}

impl From<&PackedChunk> for Chunk {
    fn from(c: &PackedChunk) -> Self {
        Chunk {
            ops: c.ops.iter().map(OpCode::from).collect(),
            strings: c.strings.clone(),
            scope_paths: c.scope_paths.clone(),
        }
    }
}

/// Typed walk over `doc`'s expression-typed schema fields,
/// compiling each into `cache` via `get_or_compile`. Returns the
/// count of compile-or-hit visits.
///
/// ## Why typed — and what changed across review rounds
///
/// Round 1 used a `$` / backtick heuristic gate on every string in
/// the doc (codex CONCERN: dropped parser-valid literals like `42`,
/// `true`, `"/detail"`).
///
/// Round 2 dropped the gate, walked every JSON string, and added a
/// post-compile filter for trivial bare-id chunks (codex CONCERN
/// round 3: hyphenated ids like `child-a` parse as subtraction and
/// produce multi-op chunks the filter can't catch; `$`-prefixed
/// node ids slip through too).
///
/// Round 3 (this iteration) walks ONLY the schema fields the type
/// system marks as expression-typed:
///
/// - every node's `bindings: BTreeMap<String, Expression>` map
///   values (the canonical home of binding expressions),
/// - every node's `opacity: NumberOrExpression::Expression(s)` and
///   `enabled: BoolOrExpression::Expression(s)` union variants,
/// - every node's `events: EventHandlers` action arrays — for each
///   `Action`, a structural walk over the body `Value` collecting
///   string leaves (still imprecise vs typed action shapes but
///   bounded to event-handler subtrees, not the whole doc).
///
/// This eliminates the round-2 false-positives entirely: node ids,
/// type enums, font families, and color literals are NEVER visited.
/// The remaining imprecision is inside action bodies — until
/// per-action-type extractors land, all string leaves under
/// `events.<onName>[].<action>` get compile-tested. Inside that
/// subtree the `Action::body` keys (`condition`, `message`, `value`,
/// `url`, etc.) are themselves not expression sources, but the
/// VALUES they point at are — and walking values picks them up.
///
/// ## Coverage caveat (still applies)
///
/// Action body walking is structural. A future iteration that wires
/// the schema-declared `Action(BTreeMap<String, Value>)` body to
/// per-action typed extractors (mirroring `crate::action::actions::
/// {state,feedback,navigation,...}::factory_*`) will replace
/// `walk_action_value_for_strings`. Today's version is
/// "best-effort coverage of action expression sources" — the
/// post-compile filter catches the bare-id pollution that survives
/// the structural walk.
pub fn warm_cache_from_document(doc: &PenDocument, cache: &ExpressionCache) -> usize {
    let mut compiled = 0usize;
    // Codex round 4 CONCERN: app/page/node lifecycle hooks ARE
    // expression-bearing (each is an `ActionList`). Walk them
    // alongside the node tree so the AOT snapshot covers
    // launch / resume / enter / mount / unmount expressions.
    if let Some(lifecycle) = doc.lifecycle.as_ref() {
        walk_app_lifecycle(lifecycle, cache, &mut compiled);
    }
    // Single pass over `pages`: every page's lifecycle hooks AND its
    // node tree are walked here, so nothing below is visited twice.
    // (Pre-fix shape walked all pages' lifecycle hooks but only
    // `pages[0].children` — the multi-page AOT gap.)
    match doc.pages.as_ref() {
        Some(pages) if !pages.is_empty() => {
            for page in pages {
                if let Some(lifecycle) = page.lifecycle.as_ref() {
                    walk_page_lifecycle(lifecycle, cache, &mut compiled);
                }
                walk_children(&page.children, cache, &mut compiled);
            }
        }
        // Documents without pages keep walking the top-level `children`
        // (legacy single-page shape).
        _ => walk_children(doc.children.as_slice(), cache, &mut compiled),
    }
    compiled
}

fn walk_children(roots: &[PenNode], cache: &ExpressionCache, compiled: &mut usize) {
    for node in roots {
        walk_node(node, cache, compiled);
    }
}

fn walk_app_lifecycle(hooks: &AppLifecycleHooks, cache: &ExpressionCache, compiled: &mut usize) {
    for list in [
        hooks.on_launch.as_ref(),
        hooks.on_resume.as_ref(),
        hooks.on_background.as_ref(),
        hooks.on_terminate.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        walk_action_list(list, cache, compiled);
    }
}

fn walk_page_lifecycle(hooks: &PageLifecycleHooks, cache: &ExpressionCache, compiled: &mut usize) {
    for list in [
        hooks.on_enter.as_ref(),
        hooks.on_leave.as_ref(),
        hooks.on_foreground.as_ref(),
        hooks.on_background.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        walk_action_list(list, cache, compiled);
    }
}

fn walk_node_lifecycle(hooks: &NodeLifecycleHooks, cache: &ExpressionCache, compiled: &mut usize) {
    for list in [hooks.on_mount.as_ref(), hooks.on_unmount.as_ref()]
        .into_iter()
        .flatten()
    {
        walk_action_list(list, cache, compiled);
    }
}

fn walk_action_list(list: &ActionList, cache: &ExpressionCache, compiled: &mut usize) {
    for action in list {
        walk_action(action, cache, compiled);
    }
}

fn walk_node(node: &PenNode, cache: &ExpressionCache, compiled: &mut usize) {
    let (opacity, enabled, bindings, events, lifecycle, children) = node_expression_surface(node);
    if let Some(NumberOrExpression::Expression(s)) = opacity {
        try_compile(s, cache, compiled);
    }
    if let Some(BoolOrExpression::Expression(s)) = enabled {
        try_compile(s, cache, compiled);
    }
    if let Some(b) = bindings {
        walk_bindings(b, cache, compiled);
    }
    if let Some(e) = events {
        walk_events(e, cache, compiled);
    }
    if let Some(l) = lifecycle {
        walk_node_lifecycle(l, cache, compiled);
    }
    if let Some(kids) = children {
        for kid in kids {
            walk_node(kid, cache, compiled);
        }
    }
}

/// Returns the expression-typed surface of a `PenNode` so the
/// walker stays one match-arm wide. Returning options keeps the
/// caller branchless — a leaf node passes `None` for `children`
/// and the loop doesn't recurse.
#[allow(clippy::type_complexity)]
fn node_expression_surface(
    node: &PenNode,
) -> (
    Option<&NumberOrExpression>,
    Option<&BoolOrExpression>,
    Option<&Bindings>,
    Option<&EventHandlers>,
    Option<&NodeLifecycleHooks>,
    Option<&Vec<PenNode>>,
) {
    macro_rules! surface {
        ($n:expr) => {
            (
                $n.base.opacity.as_ref(),
                $n.base.enabled.as_ref(),
                $n.bindings.as_ref(),
                $n.events.as_ref(),
                $n.lifecycle.as_ref(),
                None,
            )
        };
        ($n:expr, with_children) => {
            (
                $n.base.opacity.as_ref(),
                $n.base.enabled.as_ref(),
                $n.bindings.as_ref(),
                $n.events.as_ref(),
                $n.lifecycle.as_ref(),
                $n.children.as_ref(),
            )
        };
    }
    match node {
        PenNode::Frame(n) => surface!(n, with_children),
        PenNode::Group(n) => surface!(n, with_children),
        PenNode::Rectangle(n) => surface!(n, with_children),
        PenNode::Ref(n) => surface!(n, with_children),
        PenNode::Ellipse(n) => surface!(n),
        PenNode::Line(n) => surface!(n),
        PenNode::Polygon(n) => surface!(n),
        PenNode::Path(n) => surface!(n),
        PenNode::Text(n) => surface!(n),
        PenNode::TextInput(n) => surface!(n),
        PenNode::Image(n) => surface!(n),
        PenNode::IconFont(n) => surface!(n),
        PenNode::TextArea(n) => surface!(n),
        PenNode::Select(n) => surface!(n),
        PenNode::Switch(n) => surface!(n),
        PenNode::Checkbox(n) => surface!(n),
        PenNode::Slider(n) => surface!(n),
        PenNode::RadioGroup(n) => surface!(n),
        PenNode::NumberInput(n) => surface!(n),
        PenNode::Progress(n) => surface!(n),
        PenNode::Tabs(n) => surface!(n, with_children),
    }
}

fn walk_bindings(bindings: &Bindings, cache: &ExpressionCache, compiled: &mut usize) {
    for expr in bindings.values() {
        try_compile(&expr.0, cache, compiled);
    }
}

/// All 27 typed event hooks on `EventHandlers`. Returning a
/// fixed-size array (rather than serde-iterating) keeps this
/// walker compile-checked: adding a new hook to `EventHandlers`
/// without updating this list would silently miss it for AOT.
/// `event_handler_lists_match_struct_field_count` test pins the
/// count so the omission is caught at test time, not at runtime
/// on a real pack.
fn event_handler_lists(events: &EventHandlers) -> [Option<&ActionList>; 27] {
    [
        events.on_tap.as_ref(),
        events.on_double_tap.as_ref(),
        events.on_long_press.as_ref(),
        events.on_pan_start.as_ref(),
        events.on_pan_update.as_ref(),
        events.on_pan_end.as_ref(),
        events.on_scale_start.as_ref(),
        events.on_scale_update.as_ref(),
        events.on_scale_end.as_ref(),
        events.on_rotate_start.as_ref(),
        events.on_rotate_update.as_ref(),
        events.on_rotate_end.as_ref(),
        events.on_hover_enter.as_ref(),
        events.on_hover_leave.as_ref(),
        events.on_press_start.as_ref(),
        events.on_press_end.as_ref(),
        events.on_press_cancel.as_ref(),
        events.on_swipe.as_ref(),
        events.on_context_menu.as_ref(),
        events.on_raw_pointer.as_ref(),
        events.on_change.as_ref(),
        events.on_submit.as_ref(),
        events.on_focus.as_ref(),
        events.on_blur.as_ref(),
        events.on_key.as_ref(),
        events.on_scroll.as_ref(),
        events.on_reach_end.as_ref(),
    ]
}

fn walk_events(events: &EventHandlers, cache: &ExpressionCache, compiled: &mut usize) {
    for list in event_handler_lists(events).into_iter().flatten() {
        walk_action_list(list, cache, compiled);
    }
}

fn walk_action(action: &Action, cache: &ExpressionCache, compiled: &mut usize) {
    // `Action::body` is `BTreeMap<String, serde_json::Value>` —
    // each action type interprets its keys differently. Until
    // per-action typed extractors land, walk the body's string
    // leaves and post-filter via `is_trivial_bare_id_chunk`. The
    // false-positive surface is bounded to action-body subtrees
    // (no whole-doc string scan), so node ids / enum values /
    // colors never reach this path.
    for value in action.0.values() {
        walk_action_value_for_strings(value, cache, compiled);
    }
}

fn walk_action_value_for_strings(value: &Value, cache: &ExpressionCache, compiled: &mut usize) {
    match value {
        Value::String(s) => {
            try_compile(s, cache, compiled);
        }
        Value::Array(arr) => {
            for v in arr {
                walk_action_value_for_strings(v, cache, compiled);
            }
        }
        Value::Object(obj) => {
            for v in obj.values() {
                walk_action_value_for_strings(v, cache, compiled);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn try_compile(source: &str, cache: &ExpressionCache, compiled: &mut usize) {
    if source.is_empty() {
        return;
    }
    // Compile via `Expression::compile` first so we can inspect
    // the chunk and reject bare-id pollution from action bodies
    // (codex round 2/3 CONCERNs). Bindings / opacity-expr /
    // enabled-expr are typed Expression strings — the filter is
    // a no-op for those because the schema only tags real
    // expressions there. The cost is a single parse+compile per
    // distinct source on first sighting; subsequent visits hit
    // the cache via `get_or_compile`.
    if let Ok(expr) = super::Expression::compile(source) {
        if !is_trivial_bare_id_chunk(&expr.chunk, source) && cache.get_or_compile(source).is_ok() {
            *compiled += 1;
        }
    }
}

/// Drop chunks whose only effect is `PushScopeRef + Return` AND
/// whose source string doesn't start with `$`. The action-body
/// walker can hit non-expression strings (action data fields like
/// fetch's `method: "GET"` or set's `target` shorthand keys); the
/// filter prevents those from polluting the AOT snapshot. Real
/// `$self.x` / `$app.y` / `$state.z` shorthands keep — the
/// `source.starts_with('$')` check is the discriminator.
///
/// Round 3 note: with the typed walk now bounded to the
/// expression-typed surface (bindings / opacity / enabled) PLUS
/// action body subtrees, this filter mostly fires on action body
/// false-positives. `bindings` and `opacity`/`enabled`
/// `Expression` values are author-tagged as expressions and any
/// chunk shape they produce is intentional — the filter is a
/// no-op there.
fn is_trivial_bare_id_chunk(chunk: &Chunk, source: &str) -> bool {
    if chunk.ops.len() != 2 {
        return false;
    }
    if !matches!(chunk.ops[0], OpCode::PushScopeRef(_)) {
        return false;
    }
    if !matches!(chunk.ops[1], OpCode::Return) {
        return false;
    }
    !source.trim_start().starts_with('$')
}

/// Convert the full [`ExpressionsSnapshot`] into a
/// `BTreeMap<String, Chunk>` ready for
/// [`super::cache::ExpressionCache::install_precompiled`]. Stand-
/// alone helper so call-sites don't repeat the
/// `iter().map(...).collect()` chain.
pub fn snapshot_to_chunks(snap: &ExpressionsSnapshot) -> BTreeMap<String, Chunk> {
    snap.entries
        .iter()
        .map(|(src, packed)| (src.clone(), Chunk::from(packed)))
        .collect()
}

/// Convert a `BTreeMap<String, Chunk>` (e.g. from
/// [`super::cache::ExpressionCache::dump`]) into an
/// [`ExpressionsSnapshot`] ready for the writer.
pub fn chunks_to_snapshot(chunks: &BTreeMap<String, Chunk>) -> ExpressionsSnapshot {
    ExpressionsSnapshot {
        entries: chunks
            .iter()
            .map(|(src, chunk)| (src.clone(), PackedChunk::from(chunk)))
            .collect(),
    }
}

// Inline tests moved out to keep this file under the 800-line limit;
// the sibling module is registered through a path attribute so the
// exhaustive-walker tests stay adjacent to the code they pin.
#[cfg(test)]
#[path = "aot_tests.rs"]
mod aot_tests;
