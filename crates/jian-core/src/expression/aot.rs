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
    if let Some(pages) = doc.pages.as_ref() {
        for page in pages {
            if let Some(lifecycle) = page.lifecycle.as_ref() {
                walk_page_lifecycle(lifecycle, cache, &mut compiled);
            }
        }
    }
    let roots: &[PenNode] = match (&doc.pages, &doc.children) {
        (Some(pages), _) if !pages.is_empty() => &pages[0].children,
        _ => doc.children.as_slice(),
    };
    for node in roots {
        walk_node(node, cache, &mut compiled);
    }
    compiled
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
    }
}

fn walk_bindings(bindings: &Bindings, cache: &ExpressionCache, compiled: &mut usize) {
    for expr in bindings.values() {
        try_compile(&expr.0, cache, compiled);
    }
}

/// All 21 typed event hooks on `EventHandlers`. Returning a
/// fixed-size array (rather than serde-iterating) keeps this
/// walker compile-checked: adding a new hook to `EventHandlers`
/// without updating this list would silently miss it for AOT.
/// `event_handler_lists_match_struct_field_count` test pins the
/// count so the omission is caught at test time, not at runtime
/// on a real pack.
fn event_handler_lists(events: &EventHandlers) -> [Option<&ActionList>; 21] {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::Expression;
    use jian_ops_schema::load_str;

    #[test]
    fn warm_cache_picks_up_literal_expressions_without_dollar_or_backtick() {
        // Codex round 3 CONCERN: gate-free walker captures
        // parser-valid literal expressions that earlier
        // `$`/backtick gating dropped. `true`, `42`, and
        // string literals all compile; navigation paths like
        // `"detail"` (a bare identifier) parse as PushScopeRef
        // and lazy-resolve at runtime.
        let src = r##"{
          "formatVersion": "1.0",
          "version": "1.0.0",
          "id": "lit",
          "app": { "name": "Lit", "version": "1", "id": "lit" },
          "children": [
            { "type": "rectangle", "id": "r",
              "x": 0, "y": 0, "width": 10, "height": 10,
              "bindings": { "x": "42", "y": "true" } }
          ]
        }"##;
        let doc = load_str(src).expect("parse fixture").value;
        let cache = ExpressionCache::new();
        let _ = warm_cache_from_document(&doc, &cache);
        let dump = cache.dump();
        assert!(
            dump.contains_key("42"),
            "literal `42` missing from dump: {:?}",
            dump.keys().collect::<Vec<_>>()
        );
        assert!(
            dump.contains_key("true"),
            "literal `true` missing from dump: {:?}",
            dump.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn event_handler_lists_match_struct_field_count() {
        // Codex round 4 NIT + round 5 CONCERN: exhaustive
        // destructure of `EventHandlers` so any future field
        // addition forces a compile error in this test (missing
        // pattern). Without exhaustive matching, a new
        // `on_long_press_2` field would not break compilation
        // but `event_handler_lists` would silently miss it.
        let dummy: jian_ops_schema::events::ActionList = vec![];
        let handlers = jian_ops_schema::events::EventHandlers {
            on_tap: Some(dummy.clone()),
            on_double_tap: Some(dummy.clone()),
            on_long_press: Some(dummy.clone()),
            on_pan_start: Some(dummy.clone()),
            on_pan_update: Some(dummy.clone()),
            on_pan_end: Some(dummy.clone()),
            on_scale_start: Some(dummy.clone()),
            on_scale_update: Some(dummy.clone()),
            on_scale_end: Some(dummy.clone()),
            on_rotate_start: Some(dummy.clone()),
            on_rotate_update: Some(dummy.clone()),
            on_rotate_end: Some(dummy.clone()),
            on_hover_enter: Some(dummy.clone()),
            on_hover_leave: Some(dummy.clone()),
            on_change: Some(dummy.clone()),
            on_submit: Some(dummy.clone()),
            on_focus: Some(dummy.clone()),
            on_blur: Some(dummy.clone()),
            on_key: Some(dummy.clone()),
            on_scroll: Some(dummy.clone()),
            on_reach_end: Some(dummy),
        };
        // Exhaustive destructure: rust enforces every field is
        // bound by name. Adding a new field to `EventHandlers`
        // without updating this pattern triggers a "missing
        // field" compile error here, forcing the contributor to
        // also update `event_handler_lists` above.
        let jian_ops_schema::events::EventHandlers {
            on_tap,
            on_double_tap,
            on_long_press,
            on_pan_start,
            on_pan_update,
            on_pan_end,
            on_scale_start,
            on_scale_update,
            on_scale_end,
            on_rotate_start,
            on_rotate_update,
            on_rotate_end,
            on_hover_enter,
            on_hover_leave,
            on_change,
            on_submit,
            on_focus,
            on_blur,
            on_key,
            on_scroll,
            on_reach_end,
        } = handlers.clone();
        let bound = [
            on_tap.is_some(),
            on_double_tap.is_some(),
            on_long_press.is_some(),
            on_pan_start.is_some(),
            on_pan_update.is_some(),
            on_pan_end.is_some(),
            on_scale_start.is_some(),
            on_scale_update.is_some(),
            on_scale_end.is_some(),
            on_rotate_start.is_some(),
            on_rotate_update.is_some(),
            on_rotate_end.is_some(),
            on_hover_enter.is_some(),
            on_hover_leave.is_some(),
            on_change.is_some(),
            on_submit.is_some(),
            on_focus.is_some(),
            on_blur.is_some(),
            on_key.is_some(),
            on_scroll.is_some(),
            on_reach_end.is_some(),
        ];
        assert_eq!(bound.iter().filter(|x| **x).count(), 21);

        let lists = event_handler_lists(&handlers);
        assert_eq!(
            lists.iter().filter(|o| o.is_some()).count(),
            21,
            "if EventHandlers grew an `on_*` field, update event_handler_lists()"
        );
    }

    #[test]
    fn warm_cache_picks_up_lifecycle_expressions() {
        // Codex round 4 CONCERN: app/page/node lifecycle hooks
        // must contribute to AOT coverage. Author writes
        // `onLaunch: [{ set: { ... } }]` — the set value is
        // expression-typed and must land in the cache.
        let src = r##"{
          "formatVersion": "1.0",
          "version": "1.0.0",
          "id": "lc",
          "app": { "name": "LC", "version": "1", "id": "lc" },
          "state": { "launched": { "type": "bool", "default": false } },
          "lifecycle": {
            "onLaunch": [ { "set": { "$app.launched": "$app.launched || true" } } ]
          },
          "children": []
        }"##;
        let doc = load_str(src).expect("parse fixture").value;
        let cache = ExpressionCache::new();
        let _ = warm_cache_from_document(&doc, &cache);
        assert!(
            cache.dump().contains_key("$app.launched || true"),
            "app lifecycle expression missing from cache: {:?}",
            cache.dump().keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn warm_cache_drops_bare_identifier_chunks() {
        // Codex round 2 CONCERN: schema string-typed leaves like
        // node ids (`"root"`), type enums (`"int"`), and style
        // enums (`"solid"`) parse as bare scope refs without `$`.
        // The post-compile filter must drop them so they don't
        // pollute the AOT snapshot.
        let src = r##"{
          "formatVersion": "1.0",
          "version": "1.0.0",
          "id": "ids",
          "app": { "name": "Ids", "version": "1", "id": "ids" },
          "state": { "n": { "type": "int", "default": 0 } },
          "children": [
            { "type": "rectangle", "id": "root",
              "x": 0, "y": 0, "width": 10, "height": 10,
              "bindings": { "x": "$app.n + 1" } }
          ]
        }"##;
        let doc = load_str(src).expect("parse fixture").value;
        let cache = ExpressionCache::new();
        let _ = warm_cache_from_document(&doc, &cache);
        let dump = cache.dump();
        assert!(
            dump.contains_key("$app.n + 1"),
            "valid expression must be cached: {:?}",
            dump.keys().collect::<Vec<_>>()
        );
        // None of the bare ids / enum values should land.
        for junk in ["root", "ids", "Ids", "int", "rectangle"] {
            assert!(
                !dump.contains_key(junk),
                "bare-id `{junk}` must NOT be cached: {:?}",
                dump.keys().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn warm_cache_skips_object_keys() {
        // Codex round 2 CONCERN: tests must catch a future
        // regression where the walker accidentally walks object
        // keys. The fixture has `set: { "$app.count": "..." }`;
        // the key `"$app.count"` is a scope-target string the
        // runtime never compiles as a Tier-1 chunk, and the
        // walker must not treat it as one.
        let src = r##"{
          "formatVersion": "1.0",
          "version": "1.0.0",
          "id": "keys",
          "app": { "name": "Keys", "version": "1", "id": "keys" },
          "state": { "count": { "type": "int", "default": 0 } },
          "children": [
            { "type": "rectangle", "id": "btn",
              "x": 0, "y": 0, "width": 10, "height": 10,
              "events": { "onTap": [ { "set": { "$app.count": "$app.count + 1" } } ] } }
          ]
        }"##;
        let doc = load_str(src).expect("parse fixture").value;
        let cache = ExpressionCache::new();
        let _ = warm_cache_from_document(&doc, &cache);
        let dump = cache.dump();
        assert!(
            dump.contains_key("$app.count + 1"),
            "set value must be cached"
        );
        // The set KEY `"$app.count"` is a `$`-prefixed string
        // and DOES pass `is_trivial_bare_id_chunk` (passes the
        // filter — `$` prefix means "keep"). So if the walker
        // walked keys, this assertion would FAIL. The walker
        // skipping keys means `"$app.count"` arrives only via
        // its appearance INSIDE the `set` value's RHS string —
        // which IS the substring `$app.count` of `$app.count + 1`,
        // not an independent string-leaf in the JSON tree. So
        // the cache should NOT contain `$app.count` as a separate
        // key.
        assert!(
            !dump.contains_key("$app.count"),
            "set object key `$app.count` leaked into cache: {:?}",
            dump.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn warm_cache_picks_up_binding_and_action_expressions() {
        // A doc with both a binding (`$app.count + 1` in a
        // `bindings.content` slot) AND an action expression
        // (`$app.count + 1` in onTap.set body). Both should
        // dedupe to the same single source in the cache.
        let src = r##"{
          "formatVersion": "1.0",
          "version": "1.0.0",
          "id": "warm",
          "app": { "name": "Warm", "version": "1", "id": "warm" },
          "state": { "count": { "type": "int", "default": 0 } },
          "children": [
            { "type": "frame", "id": "root", "width": 320, "height": 240, "x": 0, "y": 0,
              "children": [
                { "type": "text", "id": "label",
                  "x": 16, "y": 16, "width": 200, "height": 32,
                  "content": "0",
                  "bindings": { "content": "$app.count + 1" } },
                { "type": "rectangle", "id": "btn",
                  "x": 16, "y": 64, "width": 100, "height": 40,
                  "events": { "onTap": [ { "set": { "$app.count": "$app.count + 1" } } ] } }
              ]
            }
          ]
        }"##;
        let doc = load_str(src).expect("parse fixture").value;
        let cache = ExpressionCache::new();
        let count = warm_cache_from_document(&doc, &cache);

        // The same source string `$app.count + 1` shows up twice:
        // once as a binding value, once as the `set` action's
        // value. The cache dedupes by source, so exactly one
        // entry for that source. Other parser-valid string-typed
        // schema fields (e.g. an `id`, the `state` `type` enum)
        // can also compile and land in the cache — assertion is
        // "the binding+action shared source IS present", not
        // "exactly one entry total". Object KEYS (`"$app.count"`
        // in the set body) are not walked — they're scope
        // targets in the action constructor, never compiled as
        // Tier-1 expressions.
        let dump = cache.dump();
        assert!(
            dump.contains_key("$app.count + 1"),
            "binding+action shared source missing from dump: {:?}",
            dump.keys().collect::<Vec<_>>()
        );
        // The walker counts every successful compile-or-cache-hit;
        // first sighting compiles, the second is a cache hit.
        // `count` reports compiles+hits — both >= 1 for the
        // shared source, total >= 2.
        assert!(count >= 2, "expected ≥2 walker visits, got {count}");
    }

    #[test]
    fn warm_cache_drops_unparseable_strings() {
        // Strings that look like text content / hex colors /
        // SVG path data fail to parse as Tier-1 expressions
        // and must NOT enter the cache. The gate-free walker
        // tries each one but `cache::compile_error_not_cached`
        // pins that errored compiles don't insert.
        let src = r##"{
          "formatVersion": "1.0",
          "version": "1.0.0",
          "id": "plain",
          "app": { "name": "Plain", "version": "1", "id": "plain" },
          "children": [
            { "type": "text", "id": "label",
              "x": 16, "y": 16, "width": 200, "height": 32,
              "content": "Hello world!" }
          ]
        }"##;
        let doc = load_str(src).expect("parse fixture").value;
        let cache = ExpressionCache::new();
        let _ = warm_cache_from_document(&doc, &cache);
        // `Hello world!` parses as `Hello` followed by garbage —
        // a parse error. Cache must not contain it.
        assert!(
            !cache.dump().contains_key("Hello world!"),
            "unparseable text content leaked into cache"
        );
    }

    #[test]
    fn warm_cache_failure_does_not_block_peer_success() {
        // Codex round 3 CONCERN: pin that a gate-passing parse
        // failure beside a valid expression in the same doc
        // doesn't poison the walker — the valid peer must still
        // land in the cache.
        let src = r##"{
          "formatVersion": "1.0",
          "version": "1.0.0",
          "id": "mix",
          "app": { "name": "Mix", "version": "1", "id": "mix" },
          "state": { "n": { "type": "int", "default": 0 } },
          "children": [
            { "type": "rectangle", "id": "good", "x": 0, "y": 0, "width": 10, "height": 10,
              "bindings": { "x": "$app.n + 1" } },
            { "type": "rectangle", "id": "bad", "x": 0, "y": 20, "width": 10, "height": 10,
              "bindings": { "x": "$app.n +" } }
          ]
        }"##;
        let doc = load_str(src).expect("parse fixture").value;
        let cache = ExpressionCache::new();
        let _ = warm_cache_from_document(&doc, &cache);
        let dump = cache.dump();
        assert!(
            dump.contains_key("$app.n + 1"),
            "valid peer expression missing from dump: {:?}",
            dump.keys().collect::<Vec<_>>()
        );
        assert!(
            !dump.contains_key("$app.n +"),
            "errored expression leaked into cache"
        );
    }

    #[test]
    fn warm_cache_compile_failures_are_silent() {
        // A doc with a `$`-bearing string that fails parse (e.g.
        // a malformed expression). The walker must not panic;
        // the cache must not retain the failed source.
        let src = r##"{
          "formatVersion": "1.0",
          "version": "1.0.0",
          "id": "bad",
          "app": { "name": "Bad", "version": "1", "id": "bad" },
          "children": [
            { "type": "rectangle", "id": "r",
              "x": 0, "y": 0, "width": 10, "height": 10,
              "bindings": { "x": "$app.x +" } }
          ]
        }"##;
        let doc = load_str(src).expect("parse fixture").value;
        let cache = ExpressionCache::new();
        let _ = warm_cache_from_document(&doc, &cache);
        // The failing `$app.x +` source MUST NOT appear in the
        // cache (matches `super::cache::tests::compile_error_not_
        // cached` invariant).
        assert!(!cache.dump().contains_key("$app.x +"));
    }

    #[test]
    fn warm_cache_dedupes_repeated_sources() {
        // Same expression `$app.n` appears 3× across nodes —
        // BTreeMap-keyed by source so the cache holds exactly one
        // entry for that source (other parser-valid strings in
        // the doc — node ids, enum values — can also compile and
        // land independently; the dedup invariant is per-source).
        let src = r##"{
          "formatVersion": "1.0",
          "version": "1.0.0",
          "id": "dedup",
          "app": { "name": "Dedup", "version": "1", "id": "dedup" },
          "state": { "n": { "type": "int", "default": 0 } },
          "children": [
            { "type": "rectangle", "id": "a", "x": 0, "y": 0, "width": 10, "height": 10,
              "bindings": { "x": "$app.n" } },
            { "type": "rectangle", "id": "b", "x": 0, "y": 20, "width": 10, "height": 10,
              "bindings": { "x": "$app.n" } },
            { "type": "rectangle", "id": "c", "x": 0, "y": 40, "width": 10, "height": 10,
              "bindings": { "x": "$app.n" } }
          ]
        }"##;
        let doc = load_str(src).expect("parse fixture").value;
        let cache = ExpressionCache::new();
        let count = warm_cache_from_document(&doc, &cache);
        let dump = cache.dump();
        // 3 sightings of `$app.n` deduped to 1 entry; first
        // compile succeeds, next two are cache hits.
        assert!(dump.contains_key("$app.n"));
        // Walker count >= 3 (3 visits to `$app.n` plus any
        // other parser-valid string visits).
        assert!(count >= 3, "expected ≥3 walker visits, got {count}");
    }

    #[test]
    fn round_trip_compiled_expression_through_snapshot() {
        // Take a real compiled expression, snapshot it, decode back,
        // confirm bit-for-bit equality of the chunk.
        let expr = Expression::compile("$app.count + 1").expect("compile");
        let mut chunks = BTreeMap::new();
        chunks.insert(expr.source.clone(), expr.chunk.clone());
        let snap = chunks_to_snapshot(&chunks);
        let bytes = snap.write_bytes().expect("encode");
        let back = jian_ops_schema::pack::ExpressionsSnapshot::read_bytes(&bytes).expect("decode");
        let restored = snapshot_to_chunks(&back);
        let restored_chunk = restored.get(&expr.source).expect("source preserved");
        assert_eq!(restored_chunk.ops, expr.chunk.ops);
        assert_eq!(restored_chunk.strings, expr.chunk.strings);
        assert_eq!(restored_chunk.scope_paths, expr.chunk.scope_paths);
    }

    #[test]
    fn round_trip_preserves_string_pool() {
        // Template expressions intern strings; round-trip must
        // preserve them as-is for `PushString(idx)` to resolve to
        // the same byte sequence.
        let expr = Expression::compile("\"hello \" + $app.name").expect("compile");
        let mut chunks = BTreeMap::new();
        chunks.insert(expr.source.clone(), expr.chunk.clone());
        let snap = chunks_to_snapshot(&chunks);
        let restored = snapshot_to_chunks(&snap);
        let r = restored.get(&expr.source).unwrap();
        assert_eq!(r.strings, expr.chunk.strings);
    }
}
