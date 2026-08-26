//! Tests for `super::aot` — moved out of `aot.rs` so the production
//! file stays under the 800-line convention. Registered through the
//! `#[path = "aot_tests.rs"]` module declaration in `aot.rs`, so
//! `super::*` still resolves to the `aot` module and private walkers
//! stay pin-able.

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
        on_press_start: Some(dummy.clone()),
        on_press_end: Some(dummy.clone()),
        on_press_cancel: Some(dummy.clone()),
        on_swipe: Some(dummy.clone()),
        on_context_menu: Some(dummy.clone()),
        on_raw_pointer: Some(dummy.clone()),
        on_change: Some(dummy.clone()),
        on_submit: Some(dummy.clone()),
        on_focus: Some(dummy.clone()),
        on_blur: Some(dummy.clone()),
        on_key: Some(dummy.clone()),
        on_scroll: Some(dummy.clone()),
        on_reach_end: Some(dummy),
        extra: Default::default(),
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
        on_press_start,
        on_press_end,
        on_press_cancel,
        on_swipe,
        on_context_menu,
        on_raw_pointer,
        on_change,
        on_submit,
        on_focus,
        on_blur,
        on_key,
        on_scroll,
        on_reach_end,
        extra,
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
        on_press_start.is_some(),
        on_press_end.is_some(),
        on_press_cancel.is_some(),
        on_swipe.is_some(),
        on_context_menu.is_some(),
        on_raw_pointer.is_some(),
        on_change.is_some(),
        on_submit.is_some(),
        on_focus.is_some(),
        on_blur.is_some(),
        on_key.is_some(),
        on_scroll.is_some(),
        on_reach_end.is_some(),
    ];
    assert_eq!(bound.iter().filter(|x| **x).count(), 27);
    assert!(extra.is_empty());

    let lists = event_handler_lists(&handlers);
    assert_eq!(
        lists.iter().filter(|o| o.is_some()).count(),
        27,
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

#[test]
fn aot_compiles_new_event_hooks() {
    // R1: the five new known EventHandlers hooks must all reach
    // the AOT walker. If `event_handler_lists` misses one, the
    // walker never visits its ActionList and the expression
    // below never lands in the cache.
    let src = r##"{
      "formatVersion": "1.0",
      "version": "1.0.0",
      "id": "press",
      "app": { "name": "Press", "version": "1", "id": "press" },
      "state": { "down": { "type": "bool", "default": false } },
      "children": [
        { "type": "rectangle", "id": "btn",
          "x": 0, "y": 0, "width": 10, "height": 10,
          "events": {
            "onPressStart": [ { "set": { "$app.down": "$app.down || true" } } ],
            "onPressEnd": [ { "set": { "$app.down": "$app.downEnd" } } ],
            "onPressCancel": [ { "set": { "$app.cancelled": "$app.cancel" } } ],
            "onSwipe": [ { "set": { "$app.direction": "$event.direction" } } ],
            "onContextMenu": [ { "set": { "$app.ctx": "$app.ctx + 1" } } ]
          } }
      ]
    }"##;
    let doc = load_str(src).expect("parse fixture").value;
    let cache = ExpressionCache::new();
    let _ = warm_cache_from_document(&doc, &cache);
    let dump = cache.dump();
    for expected in [
        "$app.down || true",
        "$app.downEnd",
        "$app.cancel",
        "$event.direction",
        "$app.ctx + 1",
    ] {
        assert!(
            dump.contains_key(expected),
            "new-hook expression `{expected}` missing from dump: {:?}",
            dump.keys().collect::<Vec<_>>()
        );
    }
}

#[test]
fn aot_discovers_nested_delay_and_parallel_bodies_in_new_hooks() {
    // R1/async-action discovery: actions nested inside `delay` /
    // `parallel` / `if` bodies must still be walked so their
    // expression sources land in the AOT snapshot. The structural
    // walker recurses arrays (`parallel` entries can be ActionLists)
    // and objects (`if` branches), so the inner `set` values below
    // must be found without any per-action typed extractor.
    let src = r##"{
      "formatVersion": "1.0",
      "version": "1.0.0",
      "id": "nested",
      "app": { "name": "Nested", "version": "1", "id": "nested" },
      "state": { "swiped": { "type": "int", "default": 0 } },
      "children": [
        { "type": "rectangle", "id": "btn",
          "x": 0, "y": 0, "width": 10, "height": 10,
          "events": {
            "onSwipe": [ {
              "parallel": [
                [ { "delay": { "ms": 120 } }, { "set": { "$app.swiped": "$app.swiped + 1" } } ],
                [ { "set": { "$app.swipedFlag": "$app.swipedFlag || true" } } ]
              ]
            } ],
            "onPressStart": [ {
              "if": {
                "expr": "$app.enabled",
                "then": [ { "delay": { "ms": 5 } }, { "set": { "$app.started": "$app.started + 1" } } ]
              }
            } ]
          } }
      ]
    }"##;
    let doc = load_str(src).expect("parse fixture").value;
    let cache = ExpressionCache::new();
    let _ = warm_cache_from_document(&doc, &cache);
    let dump = cache.dump();
    for expected in [
        "$app.swiped + 1",
        "$app.swipedFlag || true",
        "$app.enabled",
        "$app.started + 1",
    ] {
        assert!(
            dump.contains_key(expected),
            "nested action expression `{expected}` missing from dump: {:?}",
            dump.keys().collect::<Vec<_>>()
        );
    }
}

#[test]
fn aot_compiles_raw_pointer_handler() {
    // R1 Blocker 1: `SemanticEvent::RawPointer` already maps to
    // `onRawPointer` and the dispatcher can execute it, but the AOT
    // walker must cover the typed handler too. If
    // `event_handler_lists` misses `on_raw_pointer`, the action
    // expression below never lands in the cache and the cold-start
    // pack ships without it.
    let src = r##"{
      "formatVersion": "1.0",
      "version": "1.0.0",
      "id": "raw",
      "app": { "name": "Raw", "version": "1", "id": "raw" },
      "state": { "raws": { "type": "int", "default": 0 } },
      "children": [
        { "type": "frame", "id": "pad", "x": 0, "y": 0, "width": 200, "height": 200,
          "gestures": { "rawPointer": true },
          "events": {
            "onRawPointer": [ { "set": { "$app.raws": "$state.raws + 1" } } ]
          } }
      ]
    }"##;
    let doc = load_str(src).expect("parse fixture").value;
    let cache = ExpressionCache::new();
    let _ = warm_cache_from_document(&doc, &cache);
    assert!(
        cache.dump().contains_key("$state.raws + 1"),
        "onRawPointer expression missing from dump: {:?}",
        cache.dump().keys().collect::<Vec<_>>()
    );
}

#[test]
fn warm_cache_walks_every_page_children() {
    // R1 Blocker 2: pre-fix only `pages[0].children` were walked — an
    // expression authored ONLY on page 2 (node event handler AND node
    // lifecycle hook) never reached the AOT snapshot. Page lifecycle
    // hooks stay walked exactly once (the fix keeps one pass over
    // `pages`, no double-walk).
    let src = r##"{
      "formatVersion": "1.0",
      "version": "1.0.0",
      "id": "pages",
      "app": { "name": "Pages", "version": "1", "id": "pages" },
      "state": { "n": { "type": "int", "default": 0 } },
      "pages": [
        { "id": "p1", "name": "P1",
          "children": [
            { "type": "rectangle", "id": "a", "x": 0, "y": 0, "width": 10, "height": 10,
              "bindings": { "x": "$state.n + 1" } }
          ] },
        { "id": "p2", "name": "P2",
          "children": [
            { "type": "rectangle", "id": "b", "x": 0, "y": 0, "width": 10, "height": 10,
              "events": { "onRawPointer": [ { "set": { "$app.touched": "$state.n + 2" } } ] },
              "lifecycle": { "onMount": [ { "set": { "$self.mounted": "$state.n + 3" } } ] } }
          ] }
      ]
    }"##;
    let doc = load_str(src).expect("parse fixture").value;
    let cache = ExpressionCache::new();
    let compiled = warm_cache_from_document(&doc, &cache);
    let dump = cache.dump();
    assert!(
        dump.contains_key("$state.n + 1"),
        "page-1 child expression missing from dump: {:?}",
        dump.keys().collect::<Vec<_>>()
    );
    assert!(
        dump.contains_key("$state.n + 2"),
        "page-2 event expression missing from dump: {:?}",
        dump.keys().collect::<Vec<_>>()
    );
    assert!(
        dump.contains_key("$state.n + 3"),
        "page-2 node-lifecycle expression missing from dump: {:?}",
        dump.keys().collect::<Vec<_>>()
    );
    // Exactly-once pin: the fixture's only kept expression sources are
    // the three above and each successful visit (compile or cache hit)
    // is counted, so double-walking any page would make this 5, not 3.
    assert_eq!(
        compiled, 3,
        "expected one walk visit per expression source, got {compiled}"
    );
}

#[test]
fn aot_compiles_all_lifecycle_hooks_and_leaves_unknown_opaque() {
    // R1: every existing app/page/node lifecycle ActionList is
    // walked (the three `walk_*_lifecycle` helpers enumerate the
    // full field set), while flattened unknown hooks stay opaque —
    // they round-trip but are never compiled or executed by the
    // older runtime.
    let src = r##"{
      "formatVersion": "1.0",
      "version": "1.0.0",
      "id": "lc2",
      "app": { "name": "LC2", "version": "1", "id": "lc2" },
      "state": { "on": { "type": "bool", "default": false } },
      "lifecycle": {
        "onLaunch": [ { "set": { "$app.launched": "$app.launched || true" } } ],
        "onResume": [ { "set": { "$app.resumed": "$app.resumed" } } ],
        "onBackground": [ { "set": { "$app.bg": "$app.bg || true" } } ],
        "onTerminate": [ { "set": { "$app.term": "$app.term || true" } } ],
        "disabledEvents": ["onTerminate"],
        "interactionOrder": ["onLaunch", "onResume"],
        "onFutureApp": [ { "futureAction": { "value": "$app.future + 1" } } ]
      },
      "pages": [
        { "id": "p1", "name": "P1",
          "lifecycle": {
            "onEnter": [ { "set": { "$page.entered": "$page.entered || true" } } ],
            "onLeave": [ { "set": { "$page.left": "$page.left || true" } } ],
            "onForeground": [ { "set": { "$page.fg": "$page.fg || true" } } ],
            "onBackground": [ { "set": { "$page.bg2": "$page.bg2 || true" } } ],
            "onFuturePage": [ { "futureAction": { "value": "$app.futurePage + 1" } } ]
          },
          "children": [
            { "type": "rectangle", "id": "n", "x": 0, "y": 0, "width": 10, "height": 10,
              "lifecycle": {
                "onMount": [ { "set": { "$self.mounted": "$self.mounted || true" } } ],
                "onUnmount": [ { "set": { "$self.unmounted": "$self.unmounted || true" } } ],
                "disabledEvents": ["onUnmount"],
                "interactionOrder": ["onMount", "onUnmount"],
                "onFutureVisibility": [ { "futureAction": { "value": "$app.futureVis + 1" } } ]
              } }
          ]
        }
      ]
    }"##;
    let doc = load_str(src).expect("parse fixture").value;
    let cache = ExpressionCache::new();
    let _ = warm_cache_from_document(&doc, &cache);
    let dump = cache.dump();
    for expected in [
        "$app.launched || true",
        "$app.resumed",
        "$app.bg || true",
        "$app.term || true",
        "$page.entered || true",
        "$page.left || true",
        "$page.fg || true",
        "$page.bg2 || true",
        "$self.mounted || true",
        "$self.unmounted || true",
    ] {
        assert!(
            dump.contains_key(expected),
            "lifecycle expression `{expected}` missing from dump: {:?}",
            dump.keys().collect::<Vec<_>>()
        );
    }
    // Unknown hooks are opaque: their expression sources must NOT be
    // compiled by the older runtime's AOT walker.
    for opaque in [
        "$app.future + 1",
        "$app.futurePage + 1",
        "$app.futureVis + 1",
    ] {
        assert!(
            !dump.contains_key(opaque),
            "unknown-hook expression `{opaque}` must stay opaque: {:?}",
            dump.keys().collect::<Vec<_>>()
        );
    }
}
