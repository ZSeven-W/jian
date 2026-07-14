#[test]
fn pump_retains_every_completed_action_outcome() {
    let mut runtime = Runtime::new();
    runtime.enable_action_reporting();
    for message in ["first", "second"] {
        runtime.task_queue.spawn_future(
            std::future::ready(ExecOutcome {
                result: Err(crate::action::ActionError::Custom(message.to_owned())),
                warnings: vec![crate::expression::Diagnostic {
                    kind: crate::expression::DiagKind::RuntimeWarning,
                    message: format!("warning-{message}"),
                    span: crate::expression::Span::zero(),
                }],
            }),
            runtime.document_generation,
            Some(message.to_owned()),
        );
    }

    runtime.pump(0);
    let outcomes = runtime.take_action_outcomes();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].outcome.warnings[0].message, "warning-first");
    assert_eq!(outcomes[1].outcome.warnings[0].message, "warning-second");
    assert_eq!(outcomes[0].source.as_deref(), Some("first"));
    assert_eq!(outcomes[1].source.as_deref(), Some("second"));
    assert!(matches!(
        outcomes[0].outcome.result,
        Err(crate::action::ActionError::Custom(ref message)) if message == "first"
    ));
    assert!(runtime.take_action_outcomes().is_empty(), "drain is exact");
}

#[test]
fn synchronous_dispatch_parse_error_is_queued_for_host_reporting() {
    let schema: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","children":[
              {"type":"rectangle","id":"button","width":40,"height":40,
               "events":{"onTap":[{"not_registered":null}]}}
            ]}"#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.enable_action_reporting();
    runtime.build_layout((100.0, 100.0)).unwrap();
    runtime.rebuild_spatial();
    for phase in [
        crate::gesture::PointerPhase::Down,
        crate::gesture::PointerPhase::Up,
    ] {
        runtime.dispatch_pointer(PointerEvent::simple(
            0,
            phase,
            crate::geometry::point(10.0, 10.0),
        ));
    }

    let outcomes = runtime.take_action_outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(
        outcomes[0].outcome.result,
        Err(crate::action::ActionError::UnknownAction(ref action))
            if action == "not_registered"
    ));
    assert_eq!(outcomes[0].source.as_deref(), Some("onTap"));
}

#[test]
fn top_level_authored_offsets_drive_hit_testing_and_focused_rect() {
    let schema: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","state":{"count":{"type":"int","default":0}},"children":[
              {"type":"rectangle","id":"button","x":80,"y":40,"width":80,"height":80,
               "events":{"onTap":[{"set":{"$app.count":"$app.count + 1"}}]}},
              {"type":"text_input","id":"field","x":20,"y":130,"width":100,"height":30,"value":""}
            ]}"#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.build_layout((400.0, 200.0)).unwrap();
    runtime.rebuild_spatial();

    let button = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("button")
        .unwrap();
    assert_eq!(
        runtime.layout.node_rect(button),
        Some(crate::geometry::rect(80.0, 40.0, 80.0, 80.0)),
        "the production layout must retain authored geometry for a document root"
    );

    for phase in [
        crate::gesture::PointerPhase::Down,
        crate::gesture::PointerPhase::Up,
    ] {
        runtime.dispatch_pointer(PointerEvent::simple(
            1,
            phase,
            crate::geometry::point(100.0, 60.0),
        ));
    }
    assert_eq!(runtime.state.app_get("count").unwrap().as_i64(), Some(1));

    let field = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    runtime.focus_request(field).unwrap();
    assert_eq!(
        runtime.focused_node_rect(),
        Some(crate::geometry::rect(20.0, 130.0, 100.0, 30.0))
    );
}

#[test]
fn failed_relayout_keeps_live_layout_spatial_and_dispatch_consistent() {
    let schema: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","responsive":true,
            "state":{"hit":{"type":"int","default":0}},
            "children":[{"type":"frame","id":"root","width":100,"height":100,"children":[
              {"type":"rectangle","id":"button","width":30,"height":30,
               "events":{"onTap":[{"set":{"$app.hit":"1"}}]}}]}]}"#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.build_layout((100.0, 100.0)).unwrap();
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("button")
        .unwrap();
    let old_rect = runtime.layout.node_rect(key).unwrap();

    runtime.set_viewport_size((200.0, 100.0));
    runtime.layout.inject_staged_build_failure();
    assert!(runtime.relayout().is_err());
    assert_eq!(runtime.layout.node_rect(key), Some(old_rect));

    runtime.dispatch_pointer(PointerEvent::simple(
        1,
        crate::gesture::PointerPhase::Down,
        crate::geometry::point(5.0, 5.0),
    ));
    runtime.dispatch_pointer(PointerEvent::simple(
        1,
        crate::gesture::PointerPhase::Up,
        crate::geometry::point(5.0, 5.0),
    ));
    assert_eq!(runtime.state.app_get("hit").unwrap().as_i64(), Some(1));
}

#[test]
fn failed_atomic_reload_keeps_previous_document_state_layout_and_tasks() {
    let mut runtime = Runtime::new();
    runtime
            .load_str(
                r#"{"version":"1.2","state":{"kept":{"type":"int","default":7}},"children":[{"type":"rectangle","id":"old","x":12,"y":8,"width":30,"height":20}]}"#,
            )
            .unwrap();
    runtime.build_layout((100.0, 80.0)).unwrap();
    let old_key = runtime.document.as_ref().unwrap().tree.get("old").unwrap();
    let old_rect = runtime.node_scene_rect(old_key).unwrap();
    runtime.task_queue.spawn_future(
        std::future::pending::<ExecOutcome>(),
        runtime.document_generation,
        Some("kept-task".to_owned()),
    );

    runtime.layout.inject_staged_build_failure();
    let error = runtime
            .load_str_and_relayout(
                r#"{"version":"1.2","state":{"replacement":{"type":"int","default":1}},"children":[{"type":"rectangle","id":"new","width":50,"height":50}]}"#,
            )
            .unwrap_err();

    assert!(matches!(error, CoreError::Layout(_)));
    assert!(runtime.document.as_ref().unwrap().tree.get("new").is_none());
    assert_eq!(runtime.state.app_get("kept").unwrap().as_i64(), Some(7));
    assert!(runtime.state.app_get("replacement").is_none());
    assert_eq!(runtime.node_scene_rect(old_key), Some(old_rect));
    assert!(!runtime.task_queue.is_empty());
}

#[test]
fn atomic_reload_geometry_uses_exact_retained_layout_scopes() {
    let before = r#"{
          "version":"1.2","responsive":true,
          "app":{"name":"t","version":"1","id":"t","capabilities":["storage"]},
          "routes":{"entry":"/detail","routes":{"/detail":{"pageId":"main"}}},
          "state":{"offset":{"type":"int","default":1}},
          "pages":[{"id":"main","name":"Main","state":{"w":{"type":"int","default":10}},
            "children":[{"type":"rectangle","id":"box","width":1,"height":10,
              "state":{"extra":{"type":"int","default":2}}}]}],"children":[]}"#;
    let mut runtime = Runtime::new();
    runtime.load_str(before).unwrap();
    runtime.build_layout((200.0, 100.0)).unwrap();
    runtime.state.app_set("offset", serde_json::json!(5));
    runtime.state.page_set("main", "w", serde_json::json!(40));
    runtime
        .state
        .self_set("main", "box", "extra", serde_json::json!(3));
    runtime.state.storage_set("bump", serde_json::json!(7));
    runtime.nav = Rc::new(crate::screens::ScreenRouter::new(
        "/detail",
        ["/detail".to_owned()],
    ));

    runtime
            .load_str_and_relayout(
                r#"{
                  "version":"1.2","responsive":true,
                  "app":{"name":"t","version":"1","id":"t","capabilities":["storage"]},
                  "routes":{"entry":"/detail","routes":{"/detail":{"pageId":"main"}}},
                  "state":{"offset":{"type":"int","default":2}},
                  "pages":[{"id":"main","name":"Main","state":{"w":{"type":"int","default":12}},
                    "children":[{"type":"rectangle","id":"box","width":1,"height":10,
                      "state":{"extra":{"type":"int","default":4}},
                      "bindings":{"width":"$page.w + $self.extra + $viewport.width / 10 + ($route.path == '/detail' ? 20 : 0) + $storage.bump + $app.offset"}}]}],
                  "children":[]}"#,
            )
            .unwrap();

    let box_key = runtime.document.as_ref().unwrap().tree.get("box").unwrap();
    assert_eq!(runtime.layout.node_rect(box_key).unwrap().size.width, 95.0);
    assert_eq!(
        runtime.state.page_get("main", "w").unwrap().as_i64(),
        Some(40)
    );
    assert_eq!(
        runtime
            .state
            .self_get("main", "box", "extra")
            .unwrap()
            .as_i64(),
        Some(3)
    );
}

#[test]
fn atomic_reload_revokes_storage_before_staging_geometry() {
    let mut runtime = Runtime::new();
    runtime
        .load_str(
            r#"{"version":"1.2","responsive":true,
                "app":{"name":"t","version":"1","id":"t","capabilities":["storage"]},
                "children":[{"type":"rectangle","id":"old","width":10,"height":10}]}"#,
        )
        .unwrap();
    runtime.build_layout((100.0, 100.0)).unwrap();
    runtime.state.storage_set("bump", serde_json::json!(70));

    runtime
        .load_str_and_relayout(
            r#"{"version":"1.2","responsive":true,
                "app":{"name":"t","version":"1","id":"t"},
                "children":[{"type":"rectangle","id":"box","width":1,"height":10,
                "bindings":{"width":"$storage.bump == null ? 11 : $storage.bump"}}]}"#,
        )
        .unwrap();

    let box_key = runtime.document.as_ref().unwrap().tree.get("box").unwrap();
    assert_eq!(runtime.layout.node_rect(box_key).unwrap().size.width, 11.0);
    assert!(runtime.state.storage_snapshot().is_empty());
    assert_eq!(
        runtime.state.storage_cache.snapshot(),
        serde_json::json!({})
    );
}

#[test]
fn atomic_nonresponsive_reload_conforms_top_level_self_state() {
    let mut runtime = Runtime::new();
    runtime
        .load_str(
            r#"{"version":"1.2","children":[{"type":"rectangle","id":"card",
                "width":10,"height":10,"state":{"kept":{"type":"int","default":7}}}]}"#,
        )
        .unwrap();
    runtime.build_layout((100.0, 100.0)).unwrap();
    runtime
        .state
        .self_set("", "card", "kept", serde_json::json!(9));

    runtime
        .load_str_and_relayout(
            r#"{"version":"1.2","children":[{"type":"rectangle","id":"card",
                "width":20,"height":10,"state":{"kept":{"type":"int","default":8},
                "added":{"type":"int","default":4}}}]}"#,
        )
        .unwrap();

    assert_eq!(
        runtime.state.self_get("", "card", "kept").unwrap().as_i64(),
        Some(9)
    );
    assert_eq!(
        runtime
            .state
            .self_get("", "card", "added")
            .unwrap()
            .as_i64(),
        Some(4)
    );
}

#[test]
fn atomic_reload_reseeds_same_discriminant_widget_kind_changes() {
    let mut runtime = Runtime::new();
    runtime
        .load_str(
            r#"{"version":"1.2","children":[
                {"type":"text_input","id":"text","value":"old","width":80,"height":20},
                {"type":"switch","id":"toggle","checked":false,"width":40,"height":20}]}"#,
        )
        .unwrap();
    runtime.build_layout((200.0, 100.0)).unwrap();
    for id in ["text", "toggle"] {
        let key = runtime.document.as_ref().unwrap().tree.get(id).unwrap();
        let schema = runtime.document.as_ref().unwrap().tree.nodes[key]
            .schema
            .clone();
        runtime.widget_states.get_or_init(&schema, &runtime.state);
    }
    if let Some(crate::widget_state::WidgetState::TextInput(text)) =
        runtime.widget_states.get_mut("text")
    {
        text.set_text("durable");
    }
    if let Some(crate::widget_state::WidgetState::Toggle { on }) =
        runtime.widget_states.get_mut("toggle")
    {
        *on = true;
    }

    runtime
        .load_str_and_relayout(
            r#"{"version":"1.2","children":[
                {"type":"text_area","id":"text","value":"fresh-area","width":80,"height":20},
                {"type":"checkbox","id":"toggle","checked":false,"width":40,"height":20}]}"#,
        )
        .unwrap();

    assert!(matches!(
        runtime.widget_states.get("text"),
        Some(crate::widget_state::WidgetState::TextInput(text)) if text.text() == "fresh-area"
    ));
    assert!(matches!(
        runtime.widget_states.get("toggle"),
        Some(crate::widget_state::WidgetState::Toggle { on: false })
    ));
}

#[test]
fn atomic_reload_uses_ordered_numeric_clamp_for_reversed_ranges() {
    let mut runtime = Runtime::new();
    runtime
            .load_str(
                r#"{"version":"1.2","children":[
                {"type":"slider","id":"slider","value":5,"min":0,"max":10,"width":80,"height":20},
                {"type":"number_input","id":"number","value":5,"min":0,"max":10,"width":80,"height":20}]}"#,
            )
            .unwrap();
    runtime.build_layout((200.0, 100.0)).unwrap();
    for id in ["slider", "number"] {
        let key = runtime.document.as_ref().unwrap().tree.get(id).unwrap();
        let schema = runtime.document.as_ref().unwrap().tree.nodes[key]
            .schema
            .clone();
        runtime.widget_states.get_or_init(&schema, &runtime.state);
    }

    runtime
            .load_str_and_relayout(
                r#"{"version":"1.2","children":[
                {"type":"slider","id":"slider","min":100,"max":10,"width":80,"height":20},
                {"type":"number_input","id":"number","value":100,"min":100,"max":10,"width":80,"height":20}]}"#,
            )
            .unwrap();

    assert!(matches!(
        runtime.widget_states.get("slider"),
        Some(crate::widget_state::WidgetState::Slider { value, .. }) if (*value - 100.0).abs() < f64::EPSILON
    ));
    assert!(matches!(
        runtime.widget_states.get("number"),
        Some(crate::widget_state::WidgetState::TextInput(text)) if text.text() == "100"
    ));
}

#[test]
fn host_driven_relayout_failure_is_queued_as_layout_error_not_warning() {
    let mut runtime = Runtime::new();
    runtime
            .load_str(
                r#"{"version":"1.2","responsive":true,"children":[{"type":"frame","id":"root","width":"fill_container","height":"fill_container"}]}"#,
            )
            .unwrap();
    runtime.build_layout((100.0, 80.0)).unwrap();
    let warning_count = runtime.load_warnings().len();

    runtime.layout.inject_staged_build_failure();
    runtime.set_viewport_size((120.0, 80.0));

    assert_eq!(runtime.load_warnings().len(), warning_count);
    let errors = runtime.take_layout_errors();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("viewport relayout failed"));
    assert!(runtime.take_layout_errors().is_empty());
}

#[test]
fn pump_interleaves_event_chains_without_reordering_each_chain() {
    let schema: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","state":{"a":{"type":"int","default":0},"b":{"type":"int","default":0}},
            "children":[{"type":"frame","id":"root","layout":"horizontal","width":100,"height":30,"children":[
              {"type":"rectangle","id":"slow","width":40,"height":30,"events":{"onTap":[{"delay":{"ms":100}},{"set":{"$app.a":"1"}}]}},
              {"type":"rectangle","id":"fast","width":40,"height":30,"events":{"onTap":[{"delay":{"ms":50}},{"set":{"$app.b":"1"}}]}}
            ]}]}"#,
        )
        .unwrap();
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.build_layout((100.0, 30.0)).unwrap();
    for (pointer, x) in [(1, 5.0), (2, 45.0)] {
        runtime.dispatch_pointer(PointerEvent::simple_at(
            pointer,
            crate::gesture::PointerPhase::Down,
            crate::geometry::point(x, 5.0),
            0,
        ));
        runtime.dispatch_pointer(PointerEvent::simple_at(
            pointer,
            crate::gesture::PointerPhase::Up,
            crate::geometry::point(x, 5.0),
            0,
        ));
    }
    assert_eq!(runtime.state.app_get("a").unwrap().as_i64(), Some(0));
    assert_eq!(runtime.state.app_get("b").unwrap().as_i64(), Some(0));

    runtime.pump(50);
    assert_eq!(runtime.state.app_get("a").unwrap().as_i64(), Some(0));
    assert_eq!(runtime.state.app_get("b").unwrap().as_i64(), Some(1));
    runtime.pump(100);
    assert_eq!(runtime.state.app_get("a").unwrap().as_i64(), Some(1));
}
