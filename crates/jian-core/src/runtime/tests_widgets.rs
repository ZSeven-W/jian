#[test]
fn text_input_keyboard_and_text_routing() {
    use crate::gesture::pointer::Modifiers;
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{"version":"1.1","formatVersion":"1.1","children":[
                  {"type":"frame","id":"root","children":[
                    {"type":"text_input","id":"a"},
                    {"type":"text_input","id":"b"}]}]}"#,
        )
        .unwrap(),
    )
    .unwrap();
    // Focus the first input, type, then backspace one char.
    rt.focus_next().unwrap();
    assert!(rt.dispatch_text_input("hi").unwrap());
    rt.dispatch_keyboard("Backspace", "Backspace", false, Modifiers::empty());
    assert_eq!(widget_text(&mut rt, "a"), "h");
    // Tab to the second input; typing there leaves the first alone.
    rt.focus_next().unwrap();
    assert!(rt.dispatch_text_input("x").unwrap());
    assert_eq!(widget_text(&mut rt, "b"), "x");
    assert_eq!(widget_text(&mut rt, "a"), "h");
}

fn widget_text(rt: &mut Runtime, id: &str) -> String {
    match rt.widget_states.get_mut(id) {
        Some(crate::widget_state::WidgetState::TextInput(st)) => st.text().to_owned(),
        _ => panic!("expected text state for {id}"),
    }
}

#[test]
fn bind_value_syncs_text_input_into_state_graph() {
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"email":{"type":"string","default":""}},
                  "children":[
                    {"type":"frame","id":"root","children":[
                      {"type":"text_input","id":"e",
                       "bindings":{"bind:value":"$state.email"}}]}]}"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.focus_next().unwrap();
    assert!(rt.dispatch_text_input("a@b").unwrap());
    let got = rt
        .state
        .app_get("email")
        .and_then(|v| v.as_str().map(str::to_owned));
    assert_eq!(got.as_deref(), Some("a@b"));
    // Backspace updates the bound value too.
    rt.dispatch_keyboard(
        "Backspace",
        "Backspace",
        false,
        crate::gesture::pointer::Modifiers::empty(),
    );
    let got = rt
        .state
        .app_get("email")
        .and_then(|v| v.as_str().map(str::to_owned));
    assert_eq!(got.as_deref(), Some("a@"));
}

#[test]
fn number_input_bind_value_syncs_as_json_number() {
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"n":{"type":"float","default":0}},
                  "children":[
                    {"type":"frame","id":"root","children":[
                      {"type":"number_input","id":"ni",
                       "bindings":{"bind:value":"$state.n"}}]}]}"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.focus_next().unwrap();
    assert!(rt.dispatch_text_input("42").unwrap());
    // Bound as a number, not the string "42".
    assert_eq!(rt.state.app_get("n").and_then(|v| v.as_f64()), Some(42.0));
}

#[test]
fn switch_and_slider_keyboard_sync_to_state_graph() {
    use crate::gesture::pointer::Modifiers;
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"on":{"type":"bool","default":false},
                           "vol":{"type":"float","default":0}},
                  "children":[
                    {"type":"frame","id":"root","children":[
                      {"type":"switch","id":"sw","bindings":{"bind:value":"$state.on"}},
                      {"type":"slider","id":"sl","min":0,"max":10,"step":2,
                       "bindings":{"bind:value":"$state.vol"}}]}]}"#,
        )
        .unwrap(),
    )
    .unwrap();
    // Switch: Space flips it on.
    rt.focus_next().unwrap();
    rt.dispatch_keyboard(" ", " ", false, Modifiers::empty());
    assert_eq!(rt.state.app_get("on").and_then(|v| v.as_bool()), Some(true));
    // Slider: two ArrowRight steps of 2 → 4.
    rt.focus_next().unwrap();
    rt.dispatch_keyboard("ArrowRight", "ArrowRight", false, Modifiers::empty());
    rt.dispatch_keyboard("ArrowRight", "ArrowRight", false, Modifiers::empty());
    assert_eq!(rt.state.app_get("vol").and_then(|v| v.as_f64()), Some(4.0));
}

#[test]
fn select_arrow_keys_cycle_options_into_state_graph() {
    use crate::gesture::pointer::Modifiers;
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            // `choice` is deliberately NOT declared in the document
            // state schema: bound keys are created on first write
            // (sync_widget_binding). A declared key would exist at
            // mount and its persisted value would override the
            // authored `value:"a"` seed (bind:value read-back).
            r#"{"version":"1.1","formatVersion":"1.1",
                  "children":[
                    {"type":"frame","id":"root","children":[
                      {"type":"select","id":"se","value":"a",
                       "options":[{"value":"a","label":"A"},{"value":"b","label":"B"},
                                  {"value":"c","label":"C"}],
                       "bindings":{"bind:value":"$state.choice"}}]}]}"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.focus_next().unwrap();
    rt.dispatch_keyboard("ArrowDown", "ArrowDown", false, Modifiers::empty()); // a → b
    rt.dispatch_keyboard("ArrowDown", "ArrowDown", false, Modifiers::empty()); // b → c
    assert_eq!(
        rt.state
            .app_get("choice")
            .and_then(|v| v.as_str().map(str::to_owned))
            .as_deref(),
        Some("c")
    );
    rt.dispatch_keyboard("ArrowDown", "ArrowDown", false, Modifiers::empty()); // c → a (wrap)
    assert_eq!(
        rt.state
            .app_get("choice")
            .and_then(|v| v.as_str().map(str::to_owned))
            .as_deref(),
        Some("a")
    );
}

#[test]
fn tap_toggles_switch_and_syncs_state_graph() {
    use crate::geometry::point;
    use crate::gesture::pointer::{PointerEvent, PointerPhase};
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"on":{"type":"bool","default":false}},
                  "children":[
                    {"type":"frame","id":"root","width":200,"height":80,"children":[
                      {"type":"switch","id":"sw","x":10,"y":10,"width":44,"height":24,
                       "bindings":{"bind:value":"$state.on"}}]}]}"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    let key = rt.document.as_ref().unwrap().tree.get("sw").unwrap();
    let r = rt.layout.node_rect(key).expect("switch laid out");
    let center = point(
        r.min_x() + r.size.width / 2.0,
        r.min_y() + r.size.height / 2.0,
    );
    // A full tap = Down then Up on the switch.
    rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Down, center));
    rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Up, center));
    assert_eq!(rt.state.app_get("on").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn drag_slider_to_track_end_drives_bound_state_toward_max() {
    use crate::geometry::point;
    use crate::gesture::pointer::{PointerEvent, PointerPhase};
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{"version":"1.1","formatVersion":"1.1",
                  "state":{"vol":{"type":"float","default":0}},
                  "children":[
                    {"type":"frame","id":"root","width":300,"height":80,"children":[
                      {"type":"slider","id":"sl","x":10,"y":30,"width":200,"height":20,
                       "min":0,"max":100,"step":1,
                       "bindings":{"bind:value":"$state.vol"}}]}]}"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    let key = rt.document.as_ref().unwrap().tree.get("sl").unwrap();
    let r = rt.layout.node_rect(key).expect("slider laid out");
    // Down near the left (arms the drag), then Move to past the right
    // edge: the value should clamp to max.
    let left = point(r.min_x() + 2.0, r.min_y() + r.size.height / 2.0);
    let far_right = point(
        r.min_x() + r.size.width + 50.0,
        r.min_y() + r.size.height / 2.0,
    );
    rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Down, left));
    rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Move, far_right));
    rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Up, far_right));
    assert_eq!(
        rt.state.app_get("vol").and_then(|v| v.as_f64()),
        Some(100.0),
        "dragging to the track's right end should drive the value to max"
    );
}

#[test]
fn legacy_role_input_promotes_and_is_editable_via_runtime() {
    use jian_ops_schema::compat::{load_str_with, LoadOptions};
    // End-to-end: an old `frame role="input"` (with a bind:value) is
    // promoted on load, focusable by type, accepts typed text, and
    // syncs into the state graph — exercising Phase A promote +
    // Phase B focus/routing/bind-sync in one path.
    let legacy = r#"{"version":"1.1","formatVersion":"1.1",
          "state":{"q":{"type":"string","default":""}},
          "children":[
            {"type":"frame","id":"root","children":[
              {"type":"frame","id":"f","role":"input",
               "bindings":{"bind:value":"$state.q"}}]}]}"#;
    let loaded = load_str_with(
        legacy,
        LoadOptions {
            promote_legacy_widgets: true,
        },
    )
    .unwrap();
    let mut rt = Runtime::new_from_document(loaded.value).unwrap();
    rt.focus_next().unwrap();
    assert!(rt.dispatch_text_input("hey").unwrap());
    let got = rt
        .state
        .app_get("q")
        .and_then(|v| v.as_str().map(str::to_owned));
    assert_eq!(got.as_deref(), Some("hey"));
}

/// Two-finger pinch on a frame that declares `events.onScaleUpdate`
/// drives `$state.zoom` via `$event.scale`. Locks in the full
/// chain: PointerRouter cross-arena registration → ScaleRecognizer
/// geometry → SemanticEvent dispatch → event_payload → expression
/// resolves `$event.scale` → state graph write.
#[test]
fn two_finger_pinch_updates_state_zoom_via_event_scale() {
    use crate::geometry::point;
    use crate::gesture::pointer::PointerPhase;
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"0.8.0",
              "state":{ "zoom":{ "type":"float", "default":1.0 } },
              "children":[
                { "type":"frame","id":"canvas",
                  "width":800, "height":600,
                  "events":{
                    "onScaleUpdate": [
                      { "set": { "$app.zoom": "$event.scale" } }
                    ]
                  }
                }
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    // First finger Down at (200, 300), second at (400, 300):
    // distance 200, focal (300, 300).
    rt.dispatch_pointer(PointerEvent::simple(
        0,
        PointerPhase::Down,
        point(200.0, 300.0),
    ));
    rt.dispatch_pointer(PointerEvent::simple(
        1,
        PointerPhase::Down,
        point(400.0, 300.0),
    ));
    // Spread fingers to (100, 300) and (500, 300): distance 400 →
    // scale 2.0. Past 5% threshold → ScaleStart + ScaleUpdate fire.
    rt.dispatch_pointer(PointerEvent::simple(
        0,
        PointerPhase::Move,
        point(100.0, 300.0),
    ));
    rt.dispatch_pointer(PointerEvent::simple(
        1,
        PointerPhase::Move,
        point(500.0, 300.0),
    ));
    let zoom = rt
        .state
        .app_get("zoom")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    assert!(
        (zoom - 2.0).abs() < f32::EPSILON,
        "$event.scale should drive $app.zoom to 2.0, got {zoom}"
    );
}

/// Companion test for Rotate: `$state.rotation` driven from
/// `$event.radians`. Same pipeline as pinch, different recognizer.
#[test]
fn two_finger_rotate_updates_state_via_event_radians() {
    use crate::geometry::point;
    use crate::gesture::pointer::PointerPhase;
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"0.8.0",
              "state":{ "rotation":{ "type":"float", "default":0.0 } },
              "children":[
                { "type":"frame","id":"canvas",
                  "width":800, "height":600,
                  "events":{
                    "onRotateUpdate": [
                      { "set": { "$app.rotation": "$event.radians" } }
                    ]
                  }
                }
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    // Two fingers along the x-axis (angle 0).
    rt.dispatch_pointer(PointerEvent::simple(
        0,
        PointerPhase::Down,
        point(300.0, 300.0),
    ));
    rt.dispatch_pointer(PointerEvent::simple(
        1,
        PointerPhase::Down,
        point(500.0, 300.0),
    ));
    // Rotate finger 1 down to (500, 400): line from (300,300) →
    // (500,400) has angle atan2(100, 200) ≈ 0.4636 rad. > 5° threshold.
    rt.dispatch_pointer(PointerEvent::simple(
        1,
        PointerPhase::Move,
        point(500.0, 400.0),
    ));
    // Now fully to (500, 500): angle ≈ 0.7854 rad (45°). Update fires.
    rt.dispatch_pointer(PointerEvent::simple(
        1,
        PointerPhase::Move,
        point(500.0, 500.0),
    ));
    let rad = rt
        .state
        .app_get("rotation")
        .and_then(|v| v.as_f64())
        .unwrap_or(-1.0) as f32;
    assert!(
        rad > 0.7 && rad < 0.85,
        "$event.radians should drive $state.rotation near 0.785 (45°), got {rad}"
    );
}

#[test]
fn full_pipeline_smoke() {
    let mut rt = Runtime::new();
    rt.load_str(
        r#"{
          "version":"0.8.0",
          "children":[{"type":"rectangle","id":"r","width":200,"height":100}]
        }"#,
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    assert_eq!(rt.spatial.len(), 1);
}

/// Hot-reload preserves app-scope state values. A user editing the
/// .op while `$state.count == 5` should still see `5` after save.
#[test]
fn replace_document_preserves_app_state() {
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"0.8.0",
              "state":{"count":{"type":"int","default":0}},
              "children":[]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.state.app_set("count", serde_json::json!(5));
    assert_eq!(rt.state.app_get("count").unwrap().as_i64(), Some(5));

    let new_schema: PenDocument = serde_json::from_str(
        r#"{
          "version":"0.8.0",
          "state":{
            "count":{"type":"int","default":0},
            "username":{"type":"string","default":""}
          },
          "children":[]
        }"#,
    )
    .unwrap();
    rt.replace_document(new_schema).unwrap();

    // Pre-existing key kept its live value.
    assert_eq!(rt.state.app_get("count").unwrap().as_i64(), Some(5));
    // Newly declared key got its schema default.
    assert_eq!(rt.state.app_get("username").unwrap().as_str(), Some(""));
}

#[test]
fn reload_replaces_nonconforming_live_state_with_staged_default() {
    let old: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","state":{"value":{"type":"string","default":"old"}},"children":[]}"#,
    )
    .unwrap();
    let mut runtime = Runtime::new_from_document(old).unwrap();
    runtime.state.app_set("value", serde_json::json!("live"));
    let new: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","state":{"value":{"type":"int","default":7}},"children":[]}"#,
    )
    .unwrap();
    runtime.replace_document(new).unwrap();
    assert_eq!(runtime.state.app_get("value").unwrap().as_i64(), Some(7));
    assert!(runtime
        .load_warnings()
        .iter()
        .any(|warning| warning.contains("no longer conforms")));
}

#[test]
fn loader_failure_leaves_tasks_sessions_hydration_and_generation_untouched() {
    use crate::action::context::WsHandle;
    use crate::action::services::WebSocketSession;
    use async_trait::async_trait;
    struct Session;
    #[async_trait(?Send)]
    impl WebSocketSession for Session {
        async fn send(&self, _: String) -> Result<(), String> {
            Ok(())
        }
        async fn close(&self) -> Result<(), String> {
            Ok(())
        }
        async fn receive(&self) -> Vec<String> {
            Vec::new()
        }
    }
    let schema: PenDocument = serde_json::from_str(r#"{"version":"1.2","children":[]}"#).unwrap();
    let mut runtime = Runtime::new_from_document(schema.clone()).unwrap();
    runtime.ws_sessions.borrow_mut().insert(
        "live".into(),
        WsHandle {
            session: Rc::new(Session),
            on_message: None,
            generation: runtime.document_generation,
        },
    );
    runtime.task_queue.spawn_future(
        std::future::pending::<ExecOutcome>(),
        runtime.document_generation,
        Some("pending".into()),
    );
    let _ = runtime.state.storage_cache.read("theme");
    let generation = runtime.document_generation;
    runtime.fail_next_loader = true;
    assert!(runtime.replace_document(schema).is_err());
    assert_eq!(runtime.document_generation, generation);
    assert!(runtime.ws_sessions.borrow().contains_key("live"));
    assert!(!runtime.task_queue.is_empty());
    assert!(runtime.state.storage_cache.is_hydrating("theme"));
}

#[test]
fn successful_reload_restores_route_snapshot_against_new_valid_paths() {
    use crate::action::services::RouteState;
    struct RecordingRouter {
        restored: RefCell<Option<(RouteState, Vec<String>)>>,
    }
    impl RouterSvc for RecordingRouter {
        fn current(&self) -> RouteState {
            RouteState {
                path: "/stats".into(),
                params: [("id".into(), "7".into())].into(),
                query: [("tab".into(), "all".into())].into(),
                stack: vec!["/".into(), "/stats".into()],
            }
        }
        fn push(&self, _: &str) {}
        fn replace(&self, _: &str) {}
        fn pop(&self) {}
        fn reset(&self, _: &str) {}
        fn restore(&self, state: RouteState, valid: &[String]) {
            *self.restored.borrow_mut() = Some((state, valid.to_vec()));
        }
    }
    let old: PenDocument = serde_json::from_str(r#"{"version":"1.2","children":[]}"#).unwrap();
    let mut runtime = Runtime::new_from_document(old).unwrap();
    let router = Rc::new(RecordingRouter {
        restored: RefCell::new(None),
    });
    runtime.nav = router.clone();
    let new: PenDocument = serde_json::from_str(r#"{
          "version":"1.2","routes":{"entry":"/","routes":{"/":{"pageId":"home"},"/stats":{"pageId":"stats"}}},
          "pages":[{"id":"home","name":"Home","children":[]},{"id":"stats","name":"Stats","children":[]}],"children":[]}"#).unwrap();
    runtime.replace_document(new).unwrap();
    let restored = router.restored.borrow();
    let (state, valid) = restored.as_ref().expect("restore called");
    assert_eq!(state.path, "/stats");
    assert!(valid.contains(&"/".to_owned()) && valid.contains(&"/stats".to_owned()));
}

/// Capability gate rebuilds from the new schema, so adding `network`
/// in the .op edit becomes effective without a process restart.
#[test]
fn replace_document_refreshes_capability_gate() {
    use crate::capability::Capability;
    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"0.8.0",
              "id":"test",
              "app":{
                "name":"t","version":"0.1.0","id":"com.test.t",
                "capabilities":[]
              },
              "children":[]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!rt.capabilities.check(Capability::Network, "fetch", 0));

    let with_net: PenDocument = serde_json::from_str(
        r#"{
          "version":"0.8.0",
          "id":"test",
          "app":{
            "name":"t","version":"0.1.0","id":"com.test.t",
            "capabilities":["network"]
          },
          "children":[]
        }"#,
    )
    .unwrap();
    rt.replace_document(with_net).unwrap();
    assert!(rt.capabilities.check(Capability::Network, "fetch", 0));
}

#[test]
fn pump_websockets_drains_on_message_into_state() {
    use crate::action::context::WsHandle;
    use crate::action::services::WebSocketSession;
    use async_trait::async_trait;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct ScriptedSession {
        inbox: Rc<RefCell<Vec<String>>>,
    }
    #[async_trait(?Send)]
    impl WebSocketSession for ScriptedSession {
        async fn send(&self, _: String) -> Result<(), String> {
            Ok(())
        }
        async fn close(&self) -> Result<(), String> {
            Ok(())
        }
        async fn receive(&self) -> Vec<String> {
            std::mem::take(&mut *self.inbox.borrow_mut())
        }
    }

    let mut rt = Runtime::new();
    rt.load_str(
        r#"{
              "version":"0.8.0",
              "state":{ "last":{ "type":"string", "default":"" } },
              "children":[]
            }"#,
    )
    .unwrap();
    rt.build_layout((100.0, 100.0)).unwrap();

    // Inject a fake session with one queued message + an
    // on_message handler that copies $event.data into $app.last.
    // (Runtime path-prefix is `$app` for app-scope writes; the
    // public `$state.*` shorthand is resolved earlier in the
    // expression parser.)
    let inbox = Rc::new(RefCell::new(vec!["hello".to_owned()]));
    let session: Rc<dyn WebSocketSession> = Rc::new(ScriptedSession {
        inbox: inbox.clone(),
    });
    rt.ws_sessions.borrow_mut().insert(
        "chat".to_owned(),
        WsHandle {
            session,
            on_message: Some(serde_json::json!([
                { "set": { "$app.last": "$event.data" } }
            ])),
            generation: rt.document_generation,
        },
    );

    let fired = rt.pump_websockets();
    assert_eq!(fired, 1, "one queued message should fire one handler");
    // The set action runs end-to-end: registry parse, `$event.data`
    // resolution, executor write, and scheduler flush.
    assert_eq!(
        rt.state.app_get("last").unwrap().as_str(),
        Some("hello"),
        "$app.last should receive the WebSocket payload"
    );
    // Inbox now empty — second pump fires nothing.
    assert_eq!(rt.pump_websockets(), 0);
}

#[test]
fn pump_websockets_reports_synchronous_handler_parse_errors() {
    use crate::action::context::WsHandle;
    use crate::action::services::WebSocketSession;
    use async_trait::async_trait;

    struct OneMessage;
    #[async_trait(?Send)]
    impl WebSocketSession for OneMessage {
        async fn send(&self, _: String) -> Result<(), String> {
            Ok(())
        }
        async fn close(&self) -> Result<(), String> {
            Ok(())
        }
        async fn receive(&self) -> Vec<String> {
            vec!["hello".to_owned()]
        }
    }

    let mut runtime = Runtime::new();
    runtime
        .load_str(r#"{"version":"1.2","children":[]}"#)
        .unwrap();
    runtime.enable_action_reporting();
    runtime.ws_sessions.borrow_mut().insert(
        "chat".to_owned(),
        WsHandle {
            session: Rc::new(OneMessage),
            on_message: Some(serde_json::json!([{"not_registered": null}])),
            generation: runtime.document_generation,
        },
    );

    assert_eq!(runtime.pump_websockets(), 0);
    let outcomes = runtime.take_action_outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(
        outcomes[0].outcome.result,
        Err(crate::action::ActionError::UnknownAction(ref name))
            if name == "not_registered"
    ));
    assert_eq!(outcomes[0].source.as_deref(), Some("websocket:chat"));
}
