#[test]
fn dispatch_wheel_finds_on_scroll_target() {
    use crate::geometry::point;
    use crate::gesture::pointer::WheelEvent;
    let mut rt = Runtime::new();
    rt.load_str(
        r#"{
              "version":"0.8.0",
              "children":[
                { "type":"frame","id":"viewport","width":400,"height":300,
                  "events":{ "onScroll": [ { "set": { "$state.scrolled": "true" } } ] }
                }
              ]
            }"#,
    )
    .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();
    rt.rebuild_spatial();
    let emitted = rt.dispatch_wheel(WheelEvent::simple(point(100.0, 100.0), point(0.0, -10.0)));
    assert_eq!(emitted.len(), 1);
    assert!(matches!(
        emitted[0],
        crate::gesture::semantic::SemanticEvent::Scroll { .. }
    ));
}

#[test]
fn dispatch_wheel_ignores_nodes_without_handler() {
    use crate::geometry::point;
    use crate::gesture::pointer::WheelEvent;
    let mut rt = Runtime::new();
    rt.load_str(
        r#"{
              "version":"0.8.0",
              "children":[
                { "type":"frame","id":"plain","width":400,"height":300 }
              ]
            }"#,
    )
    .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();
    rt.rebuild_spatial();
    let emitted = rt.dispatch_wheel(WheelEvent::simple(point(100.0, 100.0), point(0.0, -10.0)));
    assert!(emitted.is_empty());
}

/// `replace_document` should swap in the new tree without disturbing
/// the existing StateGraph or service Rcs — Plan 9 hot-reload relies
/// on this so `$state.*` survives `.op` edits.
#[test]
fn replace_document_swaps_tree_keeps_state() {
    let mut rt = Runtime::new();
    rt.load_str(
        r#"{
          "version":"0.8.0",
          "children":[{"type":"rectangle","id":"r1","width":100,"height":50}]
        }"#,
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    let original_state = Rc::as_ptr(&rt.state);

    let new_schema: PenDocument = serde_json::from_str(
        r#"{
          "version":"0.8.0",
          "children":[
            {"type":"rectangle","id":"a","width":40,"height":30},
            {"type":"rectangle","id":"b","width":40,"height":30}
          ]
        }"#,
    )
    .unwrap();
    rt.replace_document(new_schema).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();

    // Same StateGraph instance — Rc didn't get rebuilt.
    assert_eq!(Rc::as_ptr(&rt.state), original_state);
    // Tree contents reflect the new schema.
    assert_eq!(rt.spatial.len(), 2);
}

/// Tab walks the focus chain in DFS pre-order and emits
/// `FocusLost` (for the previous node) followed by `FocusGained`
/// (for the new node) — the documented blur-then-focus order.
#[test]
fn dispatch_keyboard_tab_walks_focus_chain() {
    use crate::gesture::pointer::Modifiers;
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"0.8.0",
              "children":[
                { "type":"frame","id":"root","width":400,"height":300,"children":[
                  { "type":"rectangle","id":"a","width":50,"height":20,
                    "semantics":{"role":"button","label":"A"} },
                  { "type":"rectangle","id":"b","width":50,"height":20,
                    "gestures":{"focusable":true} },
                  { "type":"rectangle","id":"c","width":50,"height":20,
                    "semantics":{"role":"input"} }
                ]}
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();

    let chain = rt.focus.chain().to_vec();
    assert_eq!(chain.len(), 3);
    // Snapshot the id-by-key lookup once so the closure doesn't
    // hold a borrow on `rt` across `dispatch_keyboard` calls.
    let id_of = |rt: &Runtime, k: crate::document::NodeKey| -> String {
        crate::document::tree::node_schema_id(&rt.document.as_ref().unwrap().tree.nodes[k].schema)
            .to_owned()
    };
    let chain_ids: Vec<String> = chain.iter().map(|k| id_of(&rt, *k)).collect();
    assert_eq!(chain_ids, vec!["a", "b", "c"]);

    // First Tab — no previous focus → only FocusGained on "a".
    let evs = rt.dispatch_keyboard("Tab", Modifiers::empty());
    assert_eq!(evs.len(), 1);
    assert!(matches!(evs[0], SemanticEvent::FocusGained { .. }));
    assert_eq!(id_of(&rt, evs[0].node()), "a");

    // Second Tab — blur "a", focus "b".
    let evs = rt.dispatch_keyboard("Tab", Modifiers::empty());
    assert_eq!(evs.len(), 2);
    assert!(matches!(evs[0], SemanticEvent::FocusLost { .. }));
    assert!(matches!(evs[1], SemanticEvent::FocusGained { .. }));
    assert_eq!(id_of(&rt, evs[0].node()), "a");
    assert_eq!(id_of(&rt, evs[1].node()), "b");

    // Shift+Tab — blur "b", focus "a" (step backward).
    let evs = rt.dispatch_keyboard("Tab", Modifiers::SHIFT);
    assert_eq!(evs.len(), 2);
    assert_eq!(id_of(&rt, evs[0].node()), "b");
    assert_eq!(id_of(&rt, evs[1].node()), "a");
}

/// Non-Tab keys forward to the currently-focused node — Tab is the
/// only key consumed by the focus traversal.
#[test]
fn dispatch_keyboard_non_tab_routes_to_focused_node() {
    use crate::gesture::pointer::Modifiers;
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"0.8.0",
              "state":{"hits":{"type":"int","default":0}},
              "children":[
                { "type":"rectangle","id":"input",
                  "width":50,"height":20,
                  "semantics":{"role":"input"},
                  "events":{
                    "onKey":[
                      { "set": { "$app.hits": "$state.hits + 1" } }
                    ]
                  }
                }
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();

    // Tab in to focus the input.
    rt.dispatch_keyboard("Tab", Modifiers::empty());
    assert!(rt.focus.current().is_some());

    let evs = rt.dispatch_keyboard("Enter", Modifiers::empty());
    assert_eq!(evs.len(), 1);
    assert!(matches!(evs[0], SemanticEvent::KeyDown { .. }));

    let hits = rt
        .state
        .app_get("hits")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);
    assert_eq!(hits, 1);
}

/// onFocus / onBlur ActionLists fire when the chain advances —
/// closes the loop end-to-end (gesture event → dispatcher →
/// expression VM → state graph write).
#[test]
fn focus_handlers_fire_on_chain_step() {
    use crate::gesture::pointer::Modifiers;
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"0.8.0",
              "state":{
                "gained":{"type":"int","default":0},
                "lost":{"type":"int","default":0}
              },
              "children":[
                { "type":"rectangle","id":"a","width":50,"height":20,
                  "semantics":{"role":"button","label":"A"},
                  "events":{
                    "onFocus":[ { "set": { "$app.gained": "$state.gained + 1" } } ],
                    "onBlur":[ { "set": { "$app.lost": "$state.lost + 1" } } ]
                  } },
                { "type":"rectangle","id":"b","width":50,"height":20,
                  "semantics":{"role":"button","label":"B"} }
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();

    // Tab in → gained == 1, lost == 0.
    rt.dispatch_keyboard("Tab", Modifiers::empty());
    assert_eq!(
        rt.state.app_get("gained").and_then(|v| v.as_i64()).unwrap(),
        1
    );
    assert_eq!(
        rt.state.app_get("lost").and_then(|v| v.as_i64()).unwrap(),
        0
    );

    // Tab to "b" → "a" loses focus, "b" gains. Only "a" has
    // handlers, so gained stays at 1 and lost ticks to 1.
    rt.dispatch_keyboard("Tab", Modifiers::empty());
    assert_eq!(
        rt.state.app_get("gained").and_then(|v| v.as_i64()).unwrap(),
        1
    );
    assert_eq!(
        rt.state.app_get("lost").and_then(|v| v.as_i64()).unwrap(),
        1
    );
}

/// Hot-reload swaps the runtime tree underneath; the focus chain
/// must rebuild against the new keys and any prior focus must
/// drop (the old `NodeKey` no longer maps to a real node).
#[test]
fn replace_document_rebuilds_focus_chain() {
    use crate::gesture::pointer::Modifiers;
    let mut rt = Runtime::new();
    rt.load_str(
        r#"{
          "version":"0.8.0",
          "children":[
            { "type":"rectangle","id":"old-btn","width":50,"height":20,
              "semantics":{"role":"button"} }
          ]
        }"#,
    )
    .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();
    rt.dispatch_keyboard("Tab", Modifiers::empty());
    assert!(rt.focus.current().is_some());

    rt.replace_document(
        serde_json::from_str(
            r#"{
              "version":"0.8.0",
              "children":[
                { "type":"rectangle","id":"new-input","width":50,"height":20,
                  "semantics":{"role":"input"} },
                { "type":"rectangle","id":"new-link","width":50,"height":20,
                  "semantics":{"role":"link"} }
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    // No carry-over focus.
    assert!(rt.focus.current().is_none());
    let chain_len = rt.focus.chain().len();
    assert_eq!(chain_len, 2);

    rt.dispatch_keyboard("Tab", Modifiers::empty());
    // First Tab post-reload focuses the new chain's first node.
    let cur = rt.focus.current().unwrap();
    let id = crate::document::tree::node_schema_id(
        &rt.document.as_ref().unwrap().tree.nodes[cur].schema,
    );
    assert_eq!(id, "new-input");
}

/// Hot-reload must reset PointerRouter caches alongside focus state.
/// Pre-fix, the router kept `last_hover_target` from the old tree;
/// after a doc swap, the next hover with the same SlotMap-equal
/// (but semantically different) key would emit `HoverLeave`
/// against the wrong node. We assert the smaller-but-sufficient
/// invariant: `replace_document` zeroes the router's
/// `last_hover_target` so the next off-target hover doesn't fire
/// a stale `HoverLeave`.
#[test]
fn replace_document_resets_pointer_router_state() {
    use crate::geometry::point;
    use crate::gesture::pointer::{PointerEvent, PointerPhase};
    let mut rt = Runtime::new();
    rt.load_str(
        r#"{
          "version":"0.8.0",
          "children":[
            { "type":"rectangle","id":"hover-target","width":100,"height":50 }
          ]
        }"#,
    )
    .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();
    rt.rebuild_spatial();
    // Hover into the rectangle — stamps the router's
    // `last_hover_target` regardless of whether the node carries
    // an `onHover*` handler (handle_hover unconditionally
    // updates `last_hover_target` to the topmost hit).
    // NOTE: hover must be a Mouse/Pen pointer — Touch is contractually
    // excluded from hover processing (never emits hover actions nor
    // mutates the hover cache).
    let mouse_hover = |id: u32, x: f32, y: f32| PointerEvent {
        id: crate::gesture::PointerId(id),
        kind: crate::gesture::PointerKind::Mouse,
        phase: PointerPhase::Hover,
        position: point(x, y),
        pressure: 0.0,
        buttons: Default::default(),
        modifiers: Default::default(),
        tilt: None,
        t_ms: 0,
    };
    let _enter = rt.dispatch_pointer(mouse_hover(0, 20.0, 20.0));
    // Sanity: a second hover off the rectangle would normally
    // emit `HoverLeave` for the stamped target — that's the
    // path that goes wrong on hot-reload without the reset.
    let leave = rt.dispatch_pointer(mouse_hover(0, 500.0, 500.0));
    assert!(
        leave
            .iter()
            .any(|e| matches!(e, SemanticEvent::HoverLeave { .. })),
        "pre-reload sanity: off-target hover should emit HoverLeave, got {:?}",
        leave
    );

    // Re-stamp last_hover_target by hovering over the rect again.
    rt.dispatch_pointer(mouse_hover(0, 20.0, 20.0));

    // Hot-reload to a different document.
    rt.replace_document(
        serde_json::from_str(
            r#"{
              "version":"0.8.0",
              "children":[
                { "type":"rectangle","id":"plain","width":100,"height":50 }
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();
    rt.rebuild_spatial();
    // Hover off-target. Pre-fix, the stale `last_hover_target`
    // from the old tree would still cause a `HoverLeave` to fire
    // (against a SlotMap key that may or may not alias a real
    // node in the new tree). Post-fix the router is reset, so
    // the off-target hover emits nothing.
    let off = rt.dispatch_pointer(mouse_hover(0, 500.0, 500.0));
    assert!(
        !off.iter()
            .any(|e| matches!(e, SemanticEvent::HoverLeave { .. })),
        "router state from prior tree leaked through reload, got {:?}",
        off
    );
}

/// Re-entrancy guard. An `onBlur` handler that itself calls
/// `focus_request` (or any focus-mutating action) must take the
/// transition over — the outer call's `FocusGained` for the
/// originally-targeted node would otherwise fire its `onFocus`
/// even though focus has already moved on.
///
/// Default action builtins don't yet expose focus-mutating verbs
/// — the only focus mutator is `Runtime`'s own `focus_*` API,
/// which is unreachable from inside `dispatch_semantic` without
/// a custom `LogicProvider`. So the test uses the
/// equivalent-but-direct shape: synthesise a `FocusChange`,
/// pre-mutate `self.focus.current` to simulate exactly what a
/// nested re-entry would have done, and call
/// [`Self::emit_focus_change`] directly. The mutation is the
/// only state difference between "guarded" and "unguarded"
/// behaviour, so this covers the entire invariant the guard
/// exists to enforce. (When focus actions become first-class
/// builtins, an end-to-end test through `dispatch_keyboard`
/// becomes possible — TODO follow-up.)
#[test]
fn focus_change_re_entrant_blur_redirects_skips_stale_focus_gained() {
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"0.8.0",
              "children":[
                { "type":"rectangle","id":"a","width":50,"height":20,
                  "semantics":{"role":"button"} },
                { "type":"rectangle","id":"b","width":50,"height":20,
                  "semantics":{"role":"button"} }
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();
    let chain = rt.focus.chain().to_vec();
    let key_a = chain[0];
    let key_b = chain[1];

    // Pin focus on A so the synthetic change below has a real
    // previous to fire FocusLost against.
    let evs = rt.focus_request(key_a).unwrap();
    assert_eq!(evs.len(), 1);
    assert!(matches!(evs[0], SemanticEvent::FocusGained { .. }));
    assert_eq!(evs[0].node(), key_a);

    // Synthesise the racy state. `change` says "moved A → B",
    // but before we call emit_focus_change we pre-mutate the
    // manager's `current` to A (the `request` returns a
    // FocusChange we deliberately ignore — this is just state
    // installation, not a real transition). At emit time the
    // outer call sees:
    //   - change.previous = Some(A) → fires FocusLost{A}
    //   - change.current  = Some(B) but focus.current() = A
    //     → guard suppresses FocusGained{B}.
    // Without the guard, FocusGained{B} would fire even though
    // focus is on A — the exact stale-event surface that a
    // re-entrant onBlur would hit.
    let change = crate::gesture::FocusChange {
        previous: Some(key_a),
        current: Some(key_b),
    };
    let _ = rt.focus.request(key_a);
    let evs = rt.emit_focus_change(change);
    assert_eq!(
        evs.len(),
        1,
        "expected only FocusLost when focus.current != change.current, got {:?}",
        evs
    );
    assert!(matches!(evs[0], SemanticEvent::FocusLost { .. }));
    assert_eq!(evs[0].node(), key_a);
    // Focus state untouched by the suppressed FocusGained.
    assert_eq!(rt.focus.current(), Some(key_a));

    // Positive-control: when focus.current() *does* match
    // change.current, the FocusGained fires as normal.
    let _ = rt.focus.request(key_b);
    let change_match = crate::gesture::FocusChange {
        previous: Some(key_a),
        current: Some(key_b),
    };
    let evs = rt.emit_focus_change(change_match);
    assert_eq!(evs.len(), 2);
    assert!(matches!(evs[0], SemanticEvent::FocusLost { .. }));
    assert!(matches!(evs[1], SemanticEvent::FocusGained { .. }));
    assert_eq!(evs[0].node(), key_a);
    assert_eq!(evs[1].node(), key_b);
}

#[test]
fn responsive_viewport_root_takes_available_size() {
    let document: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","formatVersion":"1.2","responsive":true,"children":[
                {"type":"frame","id":"root","x":50,"y":50,"width":400,"height":300}]}"#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(document).unwrap();
    runtime.build_layout((800.0, 600.0)).unwrap();
    let key = runtime.document.as_ref().unwrap().tree.get("root").unwrap();
    let rect = runtime.layout.node_rect(key).unwrap();
    assert_eq!(
        (
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height
        ),
        (0.0, 0.0, 800.0, 600.0)
    );
    assert!(runtime.layout.is_origin_normalized(key));
}

#[test]
fn responsive_root_min_max_is_ignored_with_warning() {
    let document: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","formatVersion":"1.2","responsive":true,"children":[
                {"type":"frame","id":"root","width":400,"height":300,"minWidth":900}]}"#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(document).unwrap();
    runtime.build_layout((200.0, 600.0)).unwrap();
    let key = runtime.document.as_ref().unwrap().tree.get("root").unwrap();
    assert_eq!(runtime.layout.node_rect(key).unwrap().size.width, 200.0);
    assert!(runtime
        .load_warnings()
        .iter()
        .any(|warning| warning.contains("min/max")));
}

#[test]
fn non_responsive_root_keeps_authored_size() {
    let document: PenDocument = serde_json::from_str(
        r#"{"version":"1.1","children":[
                {"type":"frame","id":"root","x":50,"y":50,"width":400,"height":300}]}"#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(document).unwrap();
    runtime.build_layout((800.0, 600.0)).unwrap();
    let key = runtime.document.as_ref().unwrap().tree.get("root").unwrap();
    assert_eq!(runtime.layout.node_rect(key).unwrap().size.width, 400.0);
    assert!(!runtime.layout.is_origin_normalized(key));
}

#[test]
fn responsive_constraints_run_when_first_root_is_not_a_frame() {
    let document: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","formatVersion":"1.2","responsive":true,"children":[
                {"type":"text","id":"heading","content":"Heading"},
                {"type":"frame","id":"root","width":100,"height":100,"children":[
                    {"type":"rectangle","id":"c","x":80,"y":0,"width":30,"height":10,
                    "maxWidth":20,"constraints":{"h":"right","v":"top"}}]}]}"#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(document).unwrap();
    runtime.build_layout((800.0, 600.0)).unwrap();
    let key = runtime.document.as_ref().unwrap().tree.get("c").unwrap();
    let rect = runtime.layout.node_rect(key).unwrap();
    assert_eq!((rect.origin.x, rect.size.width), (90.0, 20.0));
}

#[test]
fn non_responsive_build_does_not_mutate_runtime_viewport() {
    let document: PenDocument = serde_json::from_str(
        r#"{"version":"1.1","children":[
                {"type":"frame","id":"root","width":400,"height":300}]}"#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(document).unwrap();
    runtime.build_layout((123.0, 456.0)).unwrap();
    assert_eq!(
        (runtime.viewport.size.width, runtime.viewport.size.height),
        (800.0, 600.0)
    );
}

#[test]
fn responsive_origin_normalization_aligns_scene_and_hit_test() {
    let document: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","formatVersion":"1.2","responsive":true,"children":[
                {"type":"frame","id":"root","x":50,"y":60,"width":100,"height":100,
                "children":[{"type":"rectangle","id":"child","x":10,"y":10,
                "width":20,"height":20}]}]}"#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(document).unwrap();
    runtime.build_layout((100.0, 100.0)).unwrap();
    runtime.rebuild_spatial();
    let child = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("child")
        .unwrap();
    let rect = runtime.layout.node_rect(child).unwrap();
    assert_eq!((rect.origin.x, rect.origin.y), (10.0, 10.0));
    assert!(runtime
        .spatial
        .hit(crate::geometry::point(15.0, 15.0))
        .contains(&child));
    assert!(!runtime
        .spatial
        .hit(crate::geometry::point(65.0, 75.0))
        .contains(&child));
}

#[test]
fn projected_screen_root_is_viewport_sized() {
    let source: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","formatVersion":"1.2","responsive":true,"children":[
                {"type":"frame","id":"screen","screen":"/","x":50,"y":60,
                "width":400,"height":300}]}"#,
    )
    .unwrap();
    let (projected, _) = jian_ops_schema::screen_projection::project_screens(&source);
    let (projected, _) = projected.unwrap();
    let mut runtime = Runtime::new_from_document(projected).unwrap();
    runtime.build_layout((320.0, 480.0)).unwrap();
    let root = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("screen")
        .unwrap();
    let rect = runtime.layout.node_rect(root).unwrap();
    assert_eq!((rect.size.width, rect.size.height), (320.0, 480.0));
    assert!(runtime.layout.is_origin_normalized(root));
}

/// Raw-pointer escape hatch end-to-end: a pointer Down inside a
/// `gestures.rawPointer` subtree routes as `SemanticEvent::RawPointer`
/// and the dispatcher must resolve and execute the authored
/// `onRawPointer` ActionList (gesture event → dispatcher → expression
/// VM → state write). R1 Blocker 1 runtime regression — `semantic.rs`
/// already maps `RawPointer` → `onRawPointer`, so this pins the whole
/// execution path independent of AOT coverage.
#[test]
fn raw_pointer_handler_executes_end_to_end() {
    use crate::geometry::point;
    use crate::gesture::pointer::{PointerEvent, PointerPhase};
    let mut rt = Runtime::new();
    rt.load_str(
        r#"{
              "version":"0.8.0",
              "state":{ "raws": { "type":"int", "default":0 } },
              "children":[
                { "type":"frame","id":"pad","x":0,"y":0,"width":200,"height":200,
                  "gestures":{ "rawPointer":true },
                  "events":{ "onRawPointer": [ { "set": { "$app.raws": "$state.raws + 1" } } ] }
                }
              ]
            }"#,
    )
    .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();
    rt.rebuild_spatial();

    // Down inside the rawPointer subtree → one `RawPointer` semantic
    // event, and the handler's `set` writes the state immediately.
    let emitted = rt.dispatch_pointer(PointerEvent::simple(
        0,
        PointerPhase::Down,
        point(50.0, 50.0),
    ));
    assert_eq!(emitted.len(), 1, "raw subtree must emit exactly RawPointer");
    assert!(matches!(
        emitted[0],
        SemanticEvent::RawPointer {
            phase: PointerPhase::Down,
            ..
        }
    ));
    assert_eq!(
        rt.state.app_get("raws").and_then(|v| v.as_i64()).unwrap(),
        1,
        "onRawPointer ActionList must execute on the Down phase"
    );

    // The same pointer's subsequent Move keeps flowing to the raw root
    // (no arena re-arming) and fires the handler again.
    let emitted = rt.dispatch_pointer(PointerEvent::simple(
        0,
        PointerPhase::Move,
        point(60.0, 60.0),
    ));
    assert!(matches!(
        emitted[0],
        SemanticEvent::RawPointer {
            phase: PointerPhase::Move,
            ..
        }
    ));
    assert_eq!(
        rt.state.app_get("raws").and_then(|v| v.as_i64()).unwrap(),
        2,
        "onRawPointer ActionList must execute on Move phases too"
    );
}
