fn variant_runtime() -> Runtime {
    let source: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","responsive":true,"state":{"long":{"type":"int","default":0}},"children":[
              {"type":"frame","id":"desktop","screen":"/","width":300,"height":200,"children":[{"type":"text_input","id":"field","value":"abIMEz","width":100,"height":30,"events":{"onLongPress":[{"set":{"$app.long":"1"}}]}}]},
              {"type":"frame","id":"mobile","screen":"/","breakpoint":{"maxWidth":480},"children":[{"type":"text_input","id":"field","value":"mobile"}]}]}"#,
        ).unwrap();
    let (projected, _) = jian_ops_schema::screen_projection::project_screens(&source);
    let (normalized, variants) = projected.unwrap();
    let desktop = normalized
        .pages
        .as_ref()
        .unwrap()
        .iter()
        .find(|page| page.id == "desktop")
        .unwrap()
        .clone();
    let mut mounted = normalized.clone();
    mounted.pages = Some(vec![desktop]);
    let mut runtime = Runtime::new_from_document(mounted).unwrap();
    runtime.configure_variant_source(normalized, "/", variants);
    runtime
}

fn freeze_variant_runtime(runtime: &mut Runtime) {
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    let node = runtime.document.as_ref().unwrap().tree.nodes[key]
        .schema
        .clone();
    let state = runtime
        .widget_states
        .get_or_init(&node, &runtime.state)
        .unwrap();
    let crate::widget_state::WidgetState::TextInput(state) = state else {
        panic!()
    };
    state.set_composition("pending", 7, 0);
    runtime.switch_variant("mobile@0-480").unwrap();
    assert!(runtime.input_frozen());
}

#[test]
fn focus_entry_points_return_busy_while_variant_input_is_frozen() {
    let mut runtime = variant_runtime();
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    freeze_variant_runtime(&mut runtime);
    assert!(matches!(runtime.focus_next(), Err(CoreError::Busy)));
    assert!(matches!(runtime.focus_previous(), Err(CoreError::Busy)));
    assert!(matches!(runtime.focus_request(key), Err(CoreError::Busy)));
    assert!(matches!(runtime.focus_clear(), Err(CoreError::Busy)));
}

#[test]
fn websocket_messages_wait_until_variant_freeze_lifts() {
    use crate::action::context::WsHandle;
    use crate::action::services::WebSocketSession;
    use async_trait::async_trait;

    struct Session(Rc<RefCell<Vec<String>>>);
    #[async_trait(?Send)]
    impl WebSocketSession for Session {
        async fn send(&self, _: String) -> Result<(), String> {
            Ok(())
        }
        async fn close(&self) -> Result<(), String> {
            Ok(())
        }
        async fn receive(&self) -> Vec<String> {
            std::mem::take(&mut *self.0.borrow_mut())
        }
    }

    let mut runtime = variant_runtime();
    runtime.state.app_set("last", serde_json::json!(""));
    let inbox = Rc::new(RefCell::new(vec!["later".to_owned()]));
    runtime.ws_sessions.borrow_mut().insert(
        "chat".into(),
        WsHandle {
            session: Rc::new(Session(inbox.clone())),
            on_message: Some(serde_json::json!([{ "set": { "$app.last": "$event.data" } }])),
            generation: runtime.document_generation,
        },
    );
    freeze_variant_runtime(&mut runtime);
    assert_eq!(runtime.pump_websockets(), 0);
    assert_eq!(inbox.borrow().as_slice(), ["later"]);
    let request = match runtime.swap_state {
        SwapState::AwaitingIme { request_id, .. } => request_id,
        _ => unreachable!(),
    };
    runtime.confirm_ime_cancel(request);
    assert_eq!(runtime.pump_websockets(), 1);
    assert_eq!(
        runtime.state.app_get("last").unwrap().as_str(),
        Some("later")
    );
}

#[test]
fn pending_long_press_is_dropped_when_tick_occurs_during_freeze() {
    let mut runtime = variant_runtime();
    runtime.build_layout((300.0, 200.0)).unwrap();
    runtime.rebuild_spatial();
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    let rect = runtime.layout.node_rect(key).unwrap();
    runtime.dispatch_pointer(PointerEvent::simple(
        1,
        crate::gesture::PointerPhase::Down,
        crate::geometry::point(rect.min_x() + 1.0, rect.min_y() + 1.0),
    ));
    freeze_variant_runtime(&mut runtime);

    let emitted = runtime.tick(800);
    assert!(emitted.is_empty());
    assert_eq!(runtime.state.app_get("long").unwrap().as_i64(), Some(0));
}

/// Pointer input DURING the freeze preserves the R2A rule too: only a
/// pending deferred Tap may flush — arena timers (LongPress) stay inert,
/// so a Move at t=800 after the 500ms deadline never claims inside the
/// parked swap, and the current event is rejected before any arbitration.
#[test]
fn pointer_event_during_freeze_flushes_only_pending_tap_and_never_claims_timers() {
    use crate::gesture::pointer::PointerPhase;
    let mut runtime = variant_runtime();
    runtime.build_layout((300.0, 200.0)).unwrap();
    runtime.rebuild_spatial();
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    let rect = runtime.layout.node_rect(key).unwrap();
    let at = crate::geometry::point(rect.min_x() + 1.0, rect.min_y() + 1.0);
    runtime.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Down, at));
    freeze_variant_runtime(&mut runtime);

    // A Move past the LongPress deadline: the frozen pointer path flushes
    // only pending-Tap state and rejects the current event — no LongPress
    // claim, no arena dispatch at all.
    let emitted = runtime.dispatch_pointer(PointerEvent::simple_at(
        1,
        PointerPhase::Move,
        at,
        800,
    ));
    assert!(emitted.is_empty());
    assert_eq!(runtime.state.app_get("long").unwrap().as_i64(), Some(0));
}

#[test]
fn transactional_variant_switch_updates_page_context() {
    let mut runtime = variant_runtime();
    assert!(runtime.switch_variant("mobile@0-480").unwrap());
    assert_eq!(runtime.selected_variant(), Some("mobile@0-480"));
    assert_eq!(runtime.active_page_key(), "mobile@0-480");
    assert!(!runtime.input_frozen());
}

#[test]
fn deferred_viewport_update_leaves_one_layout_for_variant_swap() {
    let mut runtime = variant_runtime();
    runtime.build_layout((800.0, 600.0)).unwrap();
    let field = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    let desktop_rect = runtime.layout.node_rect(field).unwrap();
    assert_eq!(
        runtime.needs_variant_swap(320.0).as_deref(),
        Some("mobile@0-480")
    );

    runtime.set_viewport_size_without_relayout((320.0, 600.0));

    assert_eq!(runtime.layout.node_rect(field), Some(desktop_rect));
    assert_eq!(runtime.layout_mutation_seen, runtime.mutation_counter());
    assert!(runtime.switch_variant("mobile@0-480").unwrap());
    assert_eq!(runtime.viewport.size, size(320.0, 600.0));
    assert_eq!(runtime.selected_variant(), Some("mobile@0-480"));
    assert_eq!(runtime.layout_mutation_seen, runtime.mutation_counter());
}

#[test]
fn failed_detached_build_leaves_every_live_variant_context_untouched() {
    let mut runtime = variant_runtime();
    let document_schema = runtime.document.as_ref().unwrap().schema.clone();
    let page_key = runtime.active_page_key().to_owned();
    let selected = runtime.selected_variant().map(str::to_owned);
    let counter = runtime.mutation_counter();
    let capabilities = runtime.capabilities.clone();

    let error = runtime.switch_variant("missing-variant").unwrap_err();
    assert!(matches!(error, CoreError::Layout(_)));
    assert_eq!(runtime.document.as_ref().unwrap().schema, document_schema);
    assert_eq!(runtime.active_page_key(), page_key);
    assert_eq!(runtime.selected_variant(), selected.as_deref());
    assert_eq!(runtime.mutation_counter(), counter);
    assert!(Rc::ptr_eq(&runtime.capabilities, &capabilities));
    assert!(!runtime.input_frozen());
}

#[test]
fn failed_rebuild_while_awaiting_ime_abandons_and_detaches_swap() {
    let mut runtime = variant_runtime();
    freeze_variant_runtime(&mut runtime);
    let request = match runtime.swap_state {
        SwapState::AwaitingIme { request_id, .. } => request_id,
        _ => unreachable!(),
    };

    assert!(runtime.switch_variant("missing-variant").is_err());
    assert!(!runtime.input_frozen());
    assert_eq!(runtime.selected_variant(), Some("desktop"));
    assert!(runtime
        .take_layout_errors()
        .iter()
        .any(|error| error.contains("parked variant rebuild failed")));
    assert_eq!(
        runtime.confirm_ime_cancel(request),
        ImeConfirmOutcome::Applied
    );
    assert_eq!(runtime.selected_variant(), Some("desktop"));
}

#[test]
fn composition_parks_and_confirmation_commits_swap() {
    let mut runtime = variant_runtime();
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    let node = runtime.document.as_ref().unwrap().tree.nodes[key]
        .schema
        .clone();
    let field = runtime
        .widget_states
        .get_or_init(&node, &runtime.state)
        .unwrap();
    let crate::widget_state::WidgetState::TextInput(field) = field else {
        panic!()
    };
    field.set_caret(2, 0);
    runtime.focus_request(key).unwrap();
    runtime
        .dispatch_ime(crate::gesture::ime::ImeEvent {
            kind: crate::gesture::ime::ImeKind::CompositionStart,
            text: String::new(),
        })
        .unwrap();
    runtime
        .dispatch_ime(crate::gesture::ime::ImeEvent {
            kind: crate::gesture::ime::ImeKind::CompositionUpdate { selection: None },
            text: "IME".into(),
        })
        .unwrap();
    assert!(!runtime.switch_variant("mobile@0-480").unwrap());
    assert!(runtime.input_frozen());
    assert!(matches!(
        runtime.dispatch_text_input("blocked"),
        Err(CoreError::Busy)
    ));
    assert!(matches!(
        runtime.dispatch_ime(crate::gesture::ime::ImeEvent {
            kind: crate::gesture::ime::ImeKind::CompositionEnd,
            text: "blocked".into(),
        }),
        Err(CoreError::Busy)
    ));
    let request_id = match &runtime.swap_state {
        SwapState::AwaitingIme { request_id, .. } => *request_id,
        _ => panic!(),
    };
    assert_eq!(
        runtime.confirm_ime_commit(request_id, "OK"),
        ImeConfirmOutcome::Applied
    );
    assert_eq!(runtime.selected_variant(), Some("mobile@0-480"));
    assert!(!runtime.input_frozen());
    assert_eq!(runtime.last_variant_build_count(), 2);
    match runtime.widget_states.get_for_page("desktop", "field") {
        Some(crate::widget_state::WidgetState::TextInput(field)) => {
            assert_eq!(field.text(), "abOKIMEz");
            assert!(field.composition().is_none());
        }
        _ => panic!(),
    }
}

#[test]
fn pump_reports_swap_deadline_and_times_out_parked_ime_swap() {
    let mut runtime = variant_runtime();
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    let node = runtime.document.as_ref().unwrap().tree.nodes[key]
        .schema
        .clone();
    let field = runtime
        .widget_states
        .get_or_init(&node, &runtime.state)
        .unwrap();
    let crate::widget_state::WidgetState::TextInput(field) = field else {
        panic!()
    };
    field.set_caret(2, 100);
    runtime.focus_request(key).unwrap();
    runtime
        .dispatch_ime(crate::gesture::ime::ImeEvent {
            kind: crate::gesture::ime::ImeKind::CompositionStart,
            text: String::new(),
        })
        .unwrap();
    runtime
        .dispatch_ime(crate::gesture::ime::ImeEvent {
            kind: crate::gesture::ime::ImeKind::CompositionUpdate { selection: None },
            text: "IME".into(),
        })
        .unwrap();
    assert!(!runtime.switch_variant("mobile@0-480").unwrap());

    let directive = runtime.pump(100);
    assert_eq!(directive.next_wake_ms, Some(500));
    assert!(runtime.input_frozen());

    let directive = runtime.pump(500);
    assert!(directive.needs_paint);
    assert!(!runtime.input_frozen());
    assert_eq!(runtime.selected_variant(), Some("mobile@0-480"));
}

#[test]
fn event_actions_receive_active_page_and_source_node_context() {
    let schema: PenDocument = serde_json::from_str(
        r#"{
          "version":"1.2","responsive":true,
          "pages":[{"id":"responsive-page","name":"P","children":[
            {"type":"frame","id":"button","width":100,"height":50,
             "events":{"onTap":[{"set":{"$page.hit":"1"}},{"set":{"$self.hit":"2"}}]}}
          ]}]}
        "#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.build_layout((100.0, 50.0)).unwrap();
    runtime.rebuild_spatial();
    runtime.dispatch_pointer(PointerEvent::simple(
        0,
        crate::gesture::PointerPhase::Down,
        crate::geometry::point(10.0, 10.0),
    ));
    runtime.dispatch_pointer(PointerEvent::simple(
        0,
        crate::gesture::PointerPhase::Up,
        crate::geometry::point(10.0, 10.0),
    ));
    assert_eq!(
        runtime.state.page.borrow()["responsive-page"]["hit"]
            .get()
            .0,
        serde_json::json!(1)
    );
    assert_eq!(
        runtime
            .state
            .self_get("responsive-page", "button", "hit")
            .unwrap()
            .0,
        serde_json::json!(2)
    );
}

#[test]
fn unprojected_responsive_initial_load_normalizes_page_ids() {
    let schema: PenDocument = serde_json::from_str(
        r#"{
          "version":"1.2","responsive":true,
          "pages":[
            {"id":"","name":"A","children":[]},
            {"id":"","name":"B","children":[]},
            {"id":"~root","name":"Reserved","children":[]}
          ]}
        "#,
    )
    .unwrap();
    let runtime = Runtime::new_from_document(schema).unwrap();
    let ids: Vec<&str> = runtime
        .document
        .as_ref()
        .unwrap()
        .schema
        .pages
        .as_ref()
        .unwrap()
        .iter()
        .map(|page| page.id.as_str())
        .collect();
    assert_eq!(ids, ["~root~2", "~root~3", "~root"]);
    assert_eq!(runtime.active_page_key(), "~root~2");
}

// Regression: a host that knows its first-frame size must get the breakpoint
// variant for that size at construction, not the 800x600 default (which lands
// on desktop). Non-responsive documents ignore the viewport argument so their
// construction stays identical to `new_from_document`.
#[test]
fn constructor_with_viewport_selects_breakpoint_variant_for_that_width() {
    let source: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","responsive":true,"children":[
          {"type":"frame","id":"desktop","screen":"/","width":800,"height":600,
           "children":[{"type":"rectangle","id":"probe","width":90,"height":10}]},
          {"type":"frame","id":"mobile","screen":"/","breakpoint":{"maxWidth":480},
           "width":320,"height":600,
           "children":[{"type":"rectangle","id":"probe","width":60,"height":10}]}]}"#,
    )
    .unwrap();

    let narrow = Runtime::new_from_document_with_viewport(source.clone(), (320.0, 600.0)).unwrap();
    assert_eq!(narrow.active_page_key(), "mobile@0-480");

    let wide = Runtime::new_from_document_with_viewport(source.clone(), (1280.0, 800.0)).unwrap();
    assert_eq!(wide.active_page_key(), "desktop");

    let defaulted = Runtime::new_from_document(source).unwrap();
    assert_eq!(defaulted.active_page_key(), "desktop");
}

// Regression: while a swap is parked on an IME handshake, a second resize
// that crosses back into the live variant's breakpoint must replace the
// pending target. Previously `needs_variant_swap` compared only against the
// live variant, returned `None`, and the confirmation committed the stale
// parked variant for a viewport it no longer matched.
#[test]
fn resize_back_while_parked_reparks_the_correct_target() {
    let mut runtime = variant_runtime();
    freeze_variant_runtime(&mut runtime); // parks mobile@0-480, composition active

    let target = runtime.needs_variant_swap(800.0);
    assert_eq!(
        target.as_deref(),
        Some("desktop"),
        "a parked swap must track the latest breakpoint selection"
    );
    assert!(!runtime.switch_variant("desktop").unwrap());
    assert!(runtime.input_frozen());

    let request = match runtime.swap_state {
        SwapState::AwaitingIme { request_id, .. } => request_id,
        _ => unreachable!(),
    };
    runtime.confirm_ime_cancel(request);
    assert_eq!(runtime.active_page_key(), "desktop");
    assert!(!runtime.input_frozen());
    // The re-parked target equals the live variant, so the confirmation drops
    // the park instead of committing: the mounted document and its widget
    // state (confirmed text, no caret churn) survive untouched.
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    let node = runtime.document.as_ref().unwrap().tree.nodes[key]
        .schema
        .clone();
    match runtime.widget_states.get_or_init(&node, &runtime.state) {
        Some(crate::widget_state::WidgetState::TextInput(text)) => {
            assert_eq!(text.text(), "abIMEz");
        }
        _ => panic!("missing text input state"),
    }
}

// Non-responsive documents must ignore the constructor's viewport argument
// entirely: state seeds and page selection stay identical to
// `new_from_document` regardless of the size passed.
#[test]
fn constructor_viewport_argument_is_ignored_for_non_responsive_docs() {
    let source: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","children":[
          {"type":"frame","id":"root","width":100,"height":100}]}"#,
    )
    .unwrap();
    let sized = Runtime::new_from_document_with_viewport(source.clone(), (402.0, 874.0)).unwrap();
    let defaulted = Runtime::new_from_document(source).unwrap();
    assert_eq!(sized.active_page_key(), defaulted.active_page_key());
    assert_eq!(
        sized.state.viewport_snapshot(),
        defaulted.state.viewport_snapshot(),
        "non-responsive construction must keep the 800x600 default viewport"
    );
}

// Regression: parked materialization must read responsive `$storage` through
// the storage cache, exactly like live evaluation. The signal map is not
// authoritative — hydrated values exist only in the cache, and a wiped key
// can linger in the map after the cache dropped it.
#[test]
fn parked_materialization_reads_hydrated_storage_not_stale_map_entries() {
    let source: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","responsive":true,"children":[
          {"type":"frame","id":"home-d","screen":"/","width":800,"height":600,
           "children":[{"type":"rectangle","id":"a","width":1,"height":5}]},
          {"type":"frame","id":"home-m","screen":"/","breakpoint":{"maxWidth":480},
           "width":320,"height":600,"children":[
             {"type":"rectangle","id":"hydrated","width":1,"height":5,
              "bindings":{"width":"$storage.w"}},
             {"type":"rectangle","id":"wiped","width":7,"height":5,
              "bindings":{"width":"$storage.gone"}}]}]}"#,
    )
    .unwrap();
    let (projected, _) = jian_ops_schema::screen_projection::project_screens(&source);
    let (normalized, variants) = projected.unwrap();
    let desktop = normalized
        .pages
        .as_ref()
        .unwrap()
        .iter()
        .find(|page| page.id == "home-d")
        .unwrap()
        .clone();
    let mut mounted = normalized.clone();
    mounted.pages = Some(vec![desktop]);
    let mut runtime = Runtime::new_from_document(mounted).unwrap();
    runtime.configure_variant_source(normalized, "/", variants);

    // Hydrated-only value: present in the cache, absent from the signal map.
    runtime
        .state
        .storage_cache
        .set_local("w", serde_json::json!(240.0));
    // Wiped value: the map keeps a stale entry after the cache dropped it.
    runtime.state.storage_set("gone", serde_json::json!(555.0));
    runtime.state.storage_cache.remove("gone");

    assert!(runtime.switch_variant("home-m@0-480").unwrap());
    let doc = runtime.document.as_ref().unwrap();
    let hydrated = runtime.layout.node_rect(doc.tree.get("hydrated").unwrap());
    assert_eq!(
        hydrated.unwrap().size.width,
        240.0,
        "hydrated storage values must materialize in the parked build"
    );
    let wiped = runtime.layout.node_rect(doc.tree.get("wiped").unwrap());
    assert_eq!(
        wiped.unwrap().size.width,
        7.0,
        "a wiped storage key must not be resurrected from the stale signal map"
    );
}
