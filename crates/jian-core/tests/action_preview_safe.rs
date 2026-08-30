//! R5 safe Preview actions and the $state input alias.

use jian_core::action::services::effect_sink::{EffectOutcome, EffectRequest, EffectSink};
use jian_core::action::services::{
    ScrollAlignment, UiMutationOutcome, UiMutationRequest, UiMutationSink, UiMutationWork,
};
use jian_core::action::{
    execute_list_async, preview_action_descriptors, ExecOutcome, PreviewActionPolicy,
};
use jian_core::Runtime;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct RecordingEffectSink {
    requests: RefCell<Vec<EffectRequest>>,
}

#[derive(Default)]
struct RecordingUiMutationSink {
    requests: RefCell<Vec<UiMutationRequest>>,
}

impl UiMutationSink for RecordingUiMutationSink {
    fn apply(&self, request: &UiMutationRequest) -> UiMutationOutcome {
        self.requests.borrow_mut().push(request.clone());
        UiMutationOutcome::Applied(UiMutationWork::REDRAW_AND_HIT_TEST)
    }
}

impl EffectSink for RecordingEffectSink {
    fn request(
        &self,
        _ctx: &jian_core::action::context::EffectRequestContext,
        request: &EffectRequest,
    ) -> EffectOutcome {
        self.requests.borrow_mut().push(request.clone());
        EffectOutcome::Accepted
    }
}

fn run(runtime: &Runtime, list: Value) -> ExecOutcome {
    let registry = runtime.actions.clone();
    let context = runtime.make_action_ctx();
    let registry = registry.borrow();
    futures::executor::block_on(execute_list_async(&registry, &list, &context))
}

#[test]
fn toggle_flips_one_writable_bool_path() {
    let runtime = Runtime::new();
    runtime.state.app_set("enabled", json!(false));

    let first = run(&runtime, json!([{ "toggle": "$state.enabled" }]));
    assert!(first.result.is_ok(), "first toggle: {:?}", first.result);
    assert_eq!(
        runtime.state.app_get("enabled").and_then(|v| v.as_bool()),
        Some(true)
    );

    let second = run(&runtime, json!([{ "toggle": "$state.enabled" }]));
    assert!(second.result.is_ok(), "second toggle: {:?}", second.result);
    assert_eq!(
        runtime.state.app_get("enabled").and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[test]
fn state_alias_reads_writes_resets_and_deletes_app_state() {
    let runtime = Runtime::new();
    runtime.state.app_set("count", json!(1));

    let written = run(
        &runtime,
        json!([{ "set": { "$state.count": "$state.count + 1" } }]),
    );
    assert!(written.result.is_ok(), "alias write: {:?}", written.result);
    assert_eq!(
        runtime.state.app_get("count").and_then(|v| v.as_i64()),
        Some(2),
        "$state reads from and writes to the canonical app scope"
    );

    let deleted = run(&runtime, json!([{ "delete": "$state.count" }]));
    assert!(deleted.result.is_ok(), "alias delete: {:?}", deleted.result);
    assert_eq!(
        runtime.state.app_get("count").map(|v| v.0),
        Some(Value::Null)
    );

    runtime.state.app_set("first", json!(1));
    runtime.state.app_set("second", json!(2));
    let reset = run(&runtime, json!([{ "reset": "$state" }]));
    assert!(reset.result.is_ok(), "alias reset: {:?}", reset.result);
    assert!(runtime.state.app_get("first").is_none());
    assert!(runtime.state.app_get("second").is_none());
}

#[test]
fn dismiss_keyboard_emits_the_typed_effect_request() {
    let sink = Rc::new(RecordingEffectSink::default());
    let mut runtime = Runtime::new();
    let sink_service: Rc<dyn EffectSink> = sink.clone();
    runtime.set_effect_sink(sink_service);

    let outcome = run(&runtime, json!([{ "dismiss_keyboard": {} }]));
    assert!(
        outcome.result.is_ok(),
        "dismiss keyboard: {:?}",
        outcome.result
    );
    assert_eq!(
        sink.requests.borrow().as_slice(),
        &[EffectRequest::DismissKeyboard]
    );
}

#[test]
fn visibility_and_scroll_emit_exact_typed_mutations() {
    let sink = Rc::new(RecordingUiMutationSink::default());
    let mut runtime = Runtime::new();
    let sink_service: Rc<dyn UiMutationSink> = sink.clone();
    runtime.set_ui_mutation_sink(sink_service);

    let outcome = run(
        &runtime,
        json!([
            { "show": "panel" },
            { "hide": { "target": "panel" } },
            { "toggle_visibility": "panel" },
            { "scroll_to": { "target": "panel", "alignment": "center" } }
        ]),
    );
    assert!(outcome.result.is_ok(), "UI mutations: {:?}", outcome.result);
    assert_eq!(
        sink.requests.borrow().as_slice(),
        &[
            UiMutationRequest::SetVisibility {
                node_id: "panel".into(),
                visible: true,
            },
            UiMutationRequest::SetVisibility {
                node_id: "panel".into(),
                visible: false,
            },
            UiMutationRequest::ToggleVisibility {
                node_id: "panel".into(),
            },
            UiMutationRequest::ScrollTo {
                target_id: "panel".into(),
                alignment: ScrollAlignment::Center,
            },
        ]
    );
}

#[test]
fn safe_action_inputs_are_validated_without_weak_fallbacks() {
    let runtime = Runtime::new();
    let registry = runtime.actions.borrow();
    for invalid in [
        json!({ "toggle": "$app.flags.enabled" }),
        json!({ "show": "  " }),
        json!({ "hide": {} }),
        json!({ "toggle_visibility": 7 }),
        json!({ "scroll_to": { "target": "", "alignment": "center" } }),
        json!({ "scroll_to": { "target": "panel", "alignment": "middle" } }),
    ] {
        assert!(
            registry.parse_single(&invalid).is_err(),
            "invalid action must be rejected: {invalid}"
        );
    }

    drop(registry);
    runtime.state.app_set("not_bool", json!(1));
    let outcome = run(&runtime, json!([{ "toggle": "$app.not_bool" }]));
    assert!(outcome.result.is_err(), "toggle rejects a non-bool target");
    assert_eq!(
        runtime
            .state
            .app_get("not_bool")
            .and_then(|value| value.as_i64()),
        Some(1),
        "failed toggle leaves the original value intact"
    );
}

#[test]
fn complete_preview_authorable_vocabulary_is_ordered() {
    let expected = [
        "set",
        "toggle",
        "delete",
        "reset",
        "if",
        "delay",
        "parallel",
        "push",
        "replace",
        "pop",
        "show",
        "hide",
        "toggle_visibility",
        "focus",
        "blur",
        "scroll_to",
        "animate",
        "toast",
        "alert",
        "confirm",
        "open_url",
        "copy",
        "share",
        "haptic",
        "dismiss_keyboard",
    ];
    assert_eq!(PreviewActionPolicy::ALLOWED, &expected);
    let descriptors = preview_action_descriptors();
    let authorable: Vec<&str> = descriptors
        .iter()
        .filter(|descriptor| descriptor.preview_authorable)
        .map(|descriptor| descriptor.name)
        .collect();
    assert_eq!(authorable, expected);
    assert_eq!(descriptors[0].category, "state");
    assert_eq!(
        descriptors
            .iter()
            .find(|descriptor| descriptor.name == "dismiss_keyboard")
            .and_then(|descriptor| descriptor.required_capability),
        Some("dismiss_keyboard")
    );

    for unsafe_name in [
        "abort",
        "for_each",
        "race",
        "paste",
        "storage_set",
        "storage_clear",
        "storage_wipe",
        "fetch",
        "ws_connect",
        "ws_send",
        "ws_close",
        "vibrate",
        "notify",
        "call",
    ] {
        assert!(
            descriptors
                .iter()
                .any(|descriptor| descriptor.name == unsafe_name && !descriptor.preview_authorable),
            "{unsafe_name} is compatibility-only"
        );
    }
}

#[test]
fn every_authorable_action_except_future_animate_is_constructible() {
    let runtime = Runtime::new();
    let registry = runtime.actions.borrow();
    let actions = [
        ("set", json!({ "set": { "$app.x": "1" } })),
        ("toggle", json!({ "toggle": "$state.enabled" })),
        ("delete", json!({ "delete": "$app.x" })),
        ("reset", json!({ "reset": "$app" })),
        ("if", json!({ "if": { "expr": "true", "then": [] } })),
        ("delay", json!({ "delay": { "ms": 1 } })),
        ("parallel", json!({ "parallel": [] })),
        ("push", json!({ "push": "'route'" })),
        ("replace", json!({ "replace": "'route'" })),
        ("pop", json!({ "pop": null })),
        ("show", json!({ "show": "panel" })),
        ("hide", json!({ "hide": "panel" })),
        ("toggle_visibility", json!({ "toggle_visibility": "panel" })),
        ("focus", json!({ "focus": { "nodeId": "panel" } })),
        ("blur", json!({ "blur": {} })),
        (
            "scroll_to",
            json!({ "scroll_to": { "target": "panel", "alignment": "center" } }),
        ),
        ("toast", json!({ "toast": "'saved'" })),
        (
            "alert",
            json!({ "alert": { "title": "'T'", "message": "'M'" } }),
        ),
        (
            "confirm",
            json!({ "confirm": { "title": "'T'", "message": "'M'" } }),
        ),
        (
            "open_url",
            json!({ "open_url": { "url": "'https://openpencil.dev'" } }),
        ),
        ("copy", json!({ "copy": "'text'" })),
        ("share", json!({ "share": { "text": "'text'" } })),
        ("haptic", json!({ "haptic": { "style": "light" } })),
        ("dismiss_keyboard", json!({ "dismiss_keyboard": {} })),
    ];
    assert_eq!(
        actions.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        PreviewActionPolicy::ALLOWED
            .iter()
            .copied()
            .filter(|name| *name != "animate")
            .collect::<Vec<_>>()
    );
    for (name, action) in actions {
        let parsed = registry
            .parse_single(&action)
            .unwrap_or_else(|error| panic!("{action} must be registered: {error}"));
        assert_eq!(parsed.name(), name);
    }
    assert!(
        registry.parse_single(&json!({ "animate": {} })).is_err(),
        "R7, not R5, owns the animate factory"
    );
    assert!(
        registry.parse_single(&json!({ "sequential": [] })).is_err(),
        "ActionList execution is already sequential; no duplicate factory"
    );
}

#[test]
fn compatibility_actions_stay_registered_but_not_authorable() {
    let runtime = Runtime::new();
    let registry = runtime.actions.borrow();
    for action in [
        json!({ "abort": {} }),
        json!({ "for_each": { "in": "[]", "as": "item", "do": [] } }),
        json!({ "race": [] }),
        json!({ "paste": "$app.text" }),
        json!({ "storage_set": { "key": "'value'" } }),
        json!({ "storage_clear": { "key": "item" } }),
        json!({ "storage_wipe": {} }),
        json!({ "fetch": { "url": "'https://example.com'" } }),
        json!({ "ws_connect": { "id": "socket", "url": "'wss://example.com'" } }),
        json!({ "ws_send": { "id": "socket", "data": "'x'" } }),
        json!({ "ws_close": { "id": "socket" } }),
        json!({ "vibrate": {} }),
        json!({ "notify": {} }),
        json!({ "call": { "module": "'m'", "function": "'f'" } }),
    ] {
        registry
            .parse_single(&action)
            .unwrap_or_else(|error| panic!("{action} must be registered: {error}"));
    }
}
