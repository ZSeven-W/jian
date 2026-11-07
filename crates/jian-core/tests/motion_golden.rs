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
