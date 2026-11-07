//! Golden vectors recorded from OpenPencil's original preview motion runtime
//! (see `tests/golden/motion/README.md`). The moved code must reproduce them.

use serde_json::Value;
use std::path::PathBuf;

/// Tolerance for f32 values that round-trip through JSON.
pub const EPSILON: f64 = 1e-6;

pub fn golden(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/motion")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

#[test]
fn golden_vectors_have_the_recorded_shape() {
    let curves = golden("curves.json");
    let curves = curves["curves"].as_array().expect("curves");
    assert_eq!(curves.len(), 12);
    assert!(curves
        .iter()
        .all(|curve| curve["values"].as_array().map(Vec::len) == Some(1001)));

    let tracks = golden("tracks.json");
    assert_eq!(tracks["cases"].as_array().map(Vec::len), Some(30));

    let transitions = golden("transitions.json");
    let kinds: Vec<_> = transitions["transitions"]
        .as_array()
        .expect("transitions")
        .iter()
        .map(|transition| transition["kind"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(kinds, ["push", "pop", "replace"]);
}

#[test]
fn ease_matches_the_recorded_curves() {
    let curves = golden("curves.json");
    for curve in curves["curves"].as_array().expect("curves") {
        let schema: jian_ops_schema::motion::Easing =
            serde_json::from_value(curve["easing"].clone()).expect("schema easing");
        let easing = jian_core::action::services::Easing::from(&schema);
        for (index, expected) in curve["values"]
            .as_array()
            .expect("values")
            .iter()
            .enumerate()
        {
            let progress = index as f32 / 1000.0;
            let actual = f64::from(jian_core::motion::ease(easing, progress));
            let expected = expected.as_f64().expect("number");
            assert!(
                (actual - expected).abs() <= EPSILON,
                "{:?} at {progress}: {actual} != {expected}",
                curve["easing"]
            );
        }
    }
}

fn request_from_case(case: &Value) -> jian_core::action::services::AnimationRequest {
    use jian_core::action::animation_registry::animatable_property_registry;
    use jian_core::action::services::{
        AnimationDirection, AnimationFillMode, AnimationProperty, AnimationRequest, Easing,
    };
    let request = &case["request"];
    let name = request["property"].as_str().expect("property");
    let descriptor = animatable_property_registry()
        .get(name)
        .expect("registered property");
    let schema_easing: jian_ops_schema::motion::Easing =
        serde_json::from_value(request["easing"].clone()).expect("easing");
    let stops = request["stops"].as_array().map(|stops| {
        stops
            .iter()
            .map(|stop| (stop[0].as_f64().expect("offset") as f32, stop[1].clone()))
            .collect()
    });
    AnimationRequest {
        target: "node".into(),
        property: AnimationProperty::from_registered(name, descriptor.apply),
        from: Some(request["from"].clone()),
        to: request["to"].clone(),
        stops,
        duration_ms: request["durationMs"].as_u64().expect("duration"),
        delay_ms: request["delayMs"].as_u64().expect("delay"),
        easing: Easing::from(&schema_easing),
        iterations: request["iterations"].as_u64().expect("iterations") as u32,
        direction: match request["direction"].as_str().expect("direction") {
            "normal" => AnimationDirection::Normal,
            "reverse" => AnimationDirection::Reverse,
            "alternate" => AnimationDirection::Alternate,
            "alternateReverse" => AnimationDirection::AlternateReverse,
            other => panic!("direction {other}"),
        },
        fill_mode: match request["fillMode"].as_str().expect("fill mode") {
            "none" => AnimationFillMode::None,
            "forwards" => AnimationFillMode::Forwards,
            "backwards" => AnimationFillMode::Backwards,
            "both" => AnimationFillMode::Both,
            other => panic!("fill mode {other}"),
        },
        requested_at_ms: 0,
    }
}

fn assert_value_matches(context: &str, actual: &Option<Value>, expected: &Value) {
    match (actual, expected) {
        (None, Value::Null) => {}
        (Some(Value::Number(a)), Value::Number(e)) => {
            let (a, e) = (a.as_f64().expect("f64"), e.as_f64().expect("f64"));
            assert!((a - e).abs() <= EPSILON, "{context}: {a} != {e}");
        }
        (Some(actual), expected) => assert_eq!(actual, expected, "{context}"),
        (None, expected) => panic!("{context}: no value, expected {expected}"),
    }
}

#[test]
fn timeline_sampling_matches_the_recorded_tracks() {
    let tracks = golden("tracks.json");
    for case in tracks["cases"].as_array().expect("cases") {
        let request = request_from_case(case);
        let name = case["name"].as_str().expect("name");
        for sample in case["samples"].as_array().expect("samples") {
            let t = sample["t"].as_u64().expect("t");
            let (value, finished) = jian_core::motion::sample_request(&request, t);
            assert_value_matches(&format!("{name} at {t} ms"), &value, &sample["value"]);
            assert_eq!(
                finished,
                sample["finished"].as_bool().expect("finished"),
                "{name} at {t} ms"
            );
        }
    }
}
