//! R7 structured animate action parser and registry integration.

use jian_core::action::services::{
    AnimationDirection, AnimationFillMode, AnimationOutcome, AnimationProperty, AnimationRequest,
    AnimationSink, Easing,
};
use jian_core::action::{
    animatable_property_registry, default_registry, execute_list_async, AnimatableProperty,
    AnimatablePropertyRegistry, AnimationApply, AnimationInterpolate, AnimationRegistryError,
    AnimationValueType,
};
use jian_core::binding::{BindingTarget, InvalidationKind};
use jian_core::Runtime;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct RecordingSink {
    requests: RefCell<Vec<AnimationRequest>>,
}

impl AnimationSink for RecordingSink {
    fn request(&self, request: &AnimationRequest) -> AnimationOutcome {
        self.requests.borrow_mut().push(request.clone());
        AnimationOutcome::Accepted
    }
}

#[test]
fn animate_is_constructible_from_the_complete_structured_body() {
    let registry = default_registry();
    let action = json!({
        "animate": {
            "target": "card",
            "property": "opacity",
            "from": 0.0,
            "to": 1.0,
            "durationMs": 300,
            "delayMs": 50,
            "easing": "ease_in_out",
            "iterations": 2,
            "direction": "alternate",
            "fillMode": "forwards"
        }
    });
    let parsed = registry
        .borrow()
        .parse_single(&action)
        .expect("R7 registers animate");
    assert_eq!(parsed.name(), "animate");
}

#[test]
fn builtins_are_registry_entries_not_an_animate_hardcoded_list() {
    let registry = animatable_property_registry();
    // The registry is an OPEN process-global (custom entries may be
    // registered at any time — sibling tests do), so assert the builtin
    // set as an ordered subset rather than the whole table.
    let names: Vec<String> = registry
        .entries()
        .map(|entry| entry.name)
        .filter(|name| !name.starts_with("test."))
        .collect();
    assert_eq!(
        names,
        [
            "cornerRadius",
            "fill",
            "height",
            "opacity",
            "rotation",
            "scaleX",
            "scaleY",
            "stroke",
            "translateX",
            "translateY",
            "width",
            "x",
            "y",
        ]
    );
    let opacity = registry.get("opacity").expect("opacity builtin");
    assert_eq!(opacity.value_type, AnimationValueType::Number);
    assert_eq!(opacity.interpolate, AnimationInterpolate::Linear);
    assert_eq!(opacity.invalidation_class, InvalidationKind::PaintOnly);
    assert_eq!(
        opacity.apply,
        AnimationApply::Binding(BindingTarget::Opacity)
    );
    assert!(opacity.capability.is_none());
    assert!(registry.is_reserved_shader_uniform("shader.glow"));
    assert!(!registry.is_reserved_shader_uniform("shader."));
}

#[test]
fn translate_x_registry_entry_is_paint_only() {
    let translate_x = animatable_property_registry()
        .get("translateX")
        .expect("translateX builtin");
    assert_eq!(translate_x.value_type, AnimationValueType::Length);
    assert_eq!(translate_x.interpolate, AnimationInterpolate::Linear);
    assert_eq!(translate_x.invalidation_class, InvalidationKind::PaintOnly);
    assert_eq!(
        translate_x.apply,
        AnimationApply::Binding(BindingTarget::TranslateX)
    );
}

#[test]
fn animate_accepts_translate_x_property() {
    let registry = default_registry();
    let parsed = registry
        .borrow()
        .parse_single(&json!({ "animate": {
            "target": "card",
            "property": "translateX",
            "from": -12,
            "to": 24,
            "durationMs": 300
        }}))
        .expect("paint-only translateX animation should validate");
    assert_eq!(parsed.name(), "animate");
}

#[test]
fn continuous_relayout_registration_is_rejected_but_discrete_is_allowed() {
    let registry = AnimatablePropertyRegistry::new();
    let linear = AnimatableProperty {
        name: "badWidth".to_owned(),
        value_type: AnimationValueType::Length,
        interpolate: AnimationInterpolate::Linear,
        invalidation_class: InvalidationKind::Relayout,
        apply: AnimationApply::Binding(BindingTarget::Width),
        capability: None,
    };
    assert_eq!(
        registry.register(linear),
        Err(AnimationRegistryError::ContinuousRelayout {
            name: "badWidth".to_owned()
        })
    );
    let discrete = AnimatableProperty {
        name: "layoutWidth".to_owned(),
        value_type: AnimationValueType::Length,
        interpolate: AnimationInterpolate::Discrete,
        invalidation_class: InvalidationKind::Relayout,
        apply: AnimationApply::Binding(BindingTarget::Width),
        capability: None,
    };
    assert!(registry.register(discrete).is_ok());
    let shader = AnimatableProperty {
        name: "shader.glow".to_owned(),
        value_type: AnimationValueType::Number,
        interpolate: AnimationInterpolate::Linear,
        invalidation_class: InvalidationKind::PaintOnly,
        apply: AnimationApply::ShaderUniform,
        capability: None,
    };
    assert!(registry.register(shader).is_ok());
    assert!(registry.get("shader.glow").is_some());
}

#[test]
fn action_emits_the_exact_typed_request() {
    let sink = Rc::new(RecordingSink::default());
    let sink_service: Rc<dyn AnimationSink> = sink.clone();
    let mut runtime = Runtime::new();
    runtime.set_now_ms(25);
    runtime.set_animation_sink(sink_service);
    let list = json!([{
        "animate": {
            "target": "card",
            "property": "rotation",
            "from": 10,
            "to": 90,
            "durationMs": 300,
            "delayMs": 50,
            "easing": "ease_in_out",
            "iterations": 2,
            "direction": "alternate_reverse",
            "fillMode": "both"
        }
    }]);
    let registry = runtime.actions.borrow();
    let context = runtime.make_action_ctx();
    let outcome = futures::executor::block_on(execute_list_async(&registry, &list, &context));
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    assert_eq!(
        sink.requests.borrow().as_slice(),
        &[AnimationRequest {
            target: "card".to_owned(),
            property: AnimationProperty::Rotation,
            from: Some(json!(10)),
            to: json!(90),
            duration_ms: 300,
            delay_ms: 50,
            easing: Easing::EaseInOut,
            iterations: 2,
            direction: AnimationDirection::AlternateReverse,
            fill_mode: AnimationFillMode::Both,
            requested_at_ms: 25,
        }]
    );
}

#[test]
fn non_preview_runtime_gets_a_diagnostic_null_sink() {
    let runtime = Runtime::new();
    let list = json!([{ "animate": {
        "target":"card","property":"opacity","to":1,"durationMs":100
    }}]);
    let registry = runtime.actions.borrow();
    let context = runtime.make_action_ctx();
    let outcome = futures::executor::block_on(execute_list_async(&registry, &list, &context));
    assert!(outcome.result.is_ok());
    assert!(
        outcome
            .warnings
            .iter()
            .any(|warning| warning.message.contains("animation sink unavailable")),
        "NullAnimationSink must diagnose instead of silently succeeding"
    );
}

#[test]
fn parser_rejects_invalid_bounds_values_and_easing() {
    let registry = default_registry();
    let invalid = [
        json!({ "animate": {
            "target":"x","property":"opacity","to":1,
            "durationMs":100,"iterations":0
        }}),
        json!({ "animate": {
            "target":"x","property":"opacity","to":1,
            "durationMs":100,"iterations":1001
        }}),
        json!({ "animate": {
            "target":"x","property":"opacity","to":"NaN",
            "durationMs":100
        }}),
        json!({ "animate": {
            "target":"x","property":"opacity","to":1,
            "durationMs":100,"easing":"spring"
        }}),
        json!({ "animate": {
            "target":"x","property":"opacity","to":1,
            "durationMs":0
        }}),
        json!({ "animate": {
            "target":"x","property":"opacity","to":1,
            "durationMs":100,"direction":"sideways"
        }}),
        json!({ "animate": {
            "target":"x","property":"opacity","to":1,
            "durationMs":100,"fillMode":"auto"
        }}),
        json!({ "animate": {
            "target":"x","property":"opacity","to":1,
            "durationMs":18446744073709551615u64,"iterations":2
        }}),
    ];
    for action in invalid {
        assert!(
            registry.borrow().parse_single(&action).is_err(),
            "invalid animate body must be rejected: {action}"
        );
    }
}

#[test]
fn unknown_property_has_the_typed_error_and_width_is_discrete() {
    let registry = default_registry();
    let unknown = match registry.borrow().parse_single(&json!({ "animate": {
        "target":"x","property":"future.blur","to":1,"durationMs":100
    }})) {
        Err(error) => error,
        Ok(_) => panic!("unknown property must be rejected"),
    };
    assert!(matches!(
        unknown,
        jian_core::action::ActionError::UnknownAnimatableProperty { property }
            if property == "future.blur"
    ));

    let width = registry
        .borrow()
        .parse_single(&json!({ "animate": {
            "target":"x","property":"width","to":200,"durationMs":100
        }}))
        .expect("discrete width transition");
    assert_eq!(width.name(), "animate");
}

#[test]
fn schema_round_trip_preserves_unknown_animation_property_text() {
    let source = r#"{
        "version":"1.1","formatVersion":"1.1","id":"future",
        "app":{"name":"future","version":"1","id":"future"},
        "children":[{"type":"rectangle","id":"x","events":{"onTap":[
            {"animate":{"target":"x","property":"shader.futureGlow","to":1,"durationMs":100}}
        ]}}]
    }"#;
    let document = jian_ops_schema::load_str(source)
        .expect("schema accepts body")
        .value;
    let encoded = serde_json::to_string(&document).expect("round trip");
    assert!(encoded.contains("\"property\":\"shader.futureGlow\""));
}

/// A registry entry's `capability` is a fail-closed gate: the same
/// request reaches the sink only when the document declares the
/// capability, and an entry naming an unknown capability fails at parse
/// so the author sees it rather than a silent runtime skip.
#[test]
fn capability_gated_property_is_fail_closed() {
    animatable_property_registry()
        .register(AnimatableProperty {
            name: "test.hapticPulse".to_owned(),
            value_type: AnimationValueType::Number,
            interpolate: AnimationInterpolate::Linear,
            invalidation_class: InvalidationKind::PaintOnly,
            apply: AnimationApply::Binding(BindingTarget::Opacity),
            capability: Some("haptic".to_owned()),
        })
        .expect("test property registers once");

    let doc = |caps: &str| {
        format!(
            r##"{{
                "version": "1.1", "formatVersion": "1.1", "id": "x",
                "app": {{ "name": "x", "version": "1", "id": "x",
                          "capabilities": [{caps}] }},
                "children": []
            }}"##
        )
    };
    let list = json!([{
        "animate": {
            "target": "card",
            "property": "test.hapticPulse",
            "to": 1.0,
            "durationMs": 100
        }
    }]);

    // Undeclared: the request never reaches the sink.
    let sink = Rc::new(RecordingSink::default());
    let mut runtime = Runtime::new();
    runtime.load_str(&doc("")).expect("load doc");
    runtime.set_animation_sink(sink.clone() as Rc<dyn AnimationSink>);
    let outcome = {
        let registry = runtime.actions.borrow();
        let context = runtime.make_action_ctx();
        futures::executor::block_on(execute_list_async(&registry, &list, &context))
    };
    assert!(outcome.result.is_ok(), "gated, not fatal");
    assert!(
        sink.requests.borrow().is_empty(),
        "an undeclared capability must keep the request away from the sink"
    );
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.message.contains("capability")),
        "the author gets a diagnostic, not silence: {:?}",
        outcome.warnings
    );

    // Declared: the same request lands.
    let sink = Rc::new(RecordingSink::default());
    let mut runtime = Runtime::new();
    runtime.load_str(&doc("\"haptic\"")).expect("load doc");
    runtime.set_animation_sink(sink.clone() as Rc<dyn AnimationSink>);
    let outcome = {
        let registry = runtime.actions.borrow();
        let context = runtime.make_action_ctx();
        futures::executor::block_on(execute_list_async(&registry, &list, &context))
    };
    assert!(outcome.result.is_ok());
    assert_eq!(
        sink.requests.borrow().len(),
        1,
        "the declared capability lets the request through"
    );
}

/// An entry whose capability name the runtime cannot resolve fails at
/// parse — fail closed at the earliest visible point.
#[test]
fn unknown_capability_name_fails_at_parse() {
    animatable_property_registry()
        .register(AnimatableProperty {
            name: "test.mystery".to_owned(),
            value_type: AnimationValueType::Number,
            interpolate: AnimationInterpolate::Linear,
            invalidation_class: InvalidationKind::PaintOnly,
            apply: AnimationApply::Binding(BindingTarget::Opacity),
            capability: Some("teleport".to_owned()),
        })
        .expect("test property registers once");
    let registry = default_registry();
    let parsed = registry.borrow().parse_single(&json!({
        "animate": {
            "target": "card",
            "property": "test.mystery",
            "to": 1.0,
            "durationMs": 100
        }
    }));
    assert!(
        parsed.is_err(),
        "an unresolvable capability name must fail at parse"
    );
}
