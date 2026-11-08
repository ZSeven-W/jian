use super::super::curve::ease;
use super::*;
use crate::scene::Color;
use serde_json::json;

fn animation(easing: Value, keyframes: Value) -> NodeAnimation {
    serde_json::from_value(json!({
        "trigger": "mount", "durationMs": 400, "easing": easing, "keyframes": keyframes,
    }))
    .expect("animation")
}

/// Evaluate a compiled animation the way CSS does: find the interval, solve
/// its cubic-bezier precisely, interpolate the interval's end values.
fn css_value(compiled: &CompiledAnimation, name: &str, u: f32) -> Value {
    let segment = compiled
        .segments
        .iter()
        .find(|segment| u <= segment.t1)
        .unwrap_or_else(|| compiled.segments.last().expect("segments"));
    let span = (segment.t1 - segment.t0).max(f32::EPSILON);
    let local = ((u - segment.t0) / span).clamp(0.0, 1.0);
    let piece = CubicPiece {
        x0: 0.0,
        x1: 1.0,
        y0: 0.0,
        y1: 1.0,
        control: segment.bezier,
    };
    let progress = piece.eval(local);
    let (from, to) = (&segment.from[name], &segment.to[name]);
    match (from, to) {
        (Value::String(_), Value::String(_)) => {
            super::super::interpolate::interpolate_color(from, to, progress)
        }
        _ => {
            let (a, b) = (from.as_f64().unwrap_or(0.0), to.as_f64().unwrap_or(0.0));
            json!(a + (b - a) * f64::from(progress))
        }
    }
}

fn assert_compiles_faithfully(name: &str, declaration: &NodeAnimation) -> CompiledAnimation {
    let compiled = compile(declaration);
    assert!(!compiled.degraded, "{name} degraded");
    let tracks = tracks(declaration);
    let easing = Easing::from(&declaration.easing);
    for track in &tracks {
        let numbers: Vec<f64> = track.stops.iter().filter_map(|(_, v)| v.as_f64()).collect();
        let range = numbers.iter().cloned().fold(f64::MIN, f64::max)
            - numbers.iter().cloned().fold(f64::MAX, f64::min);
        for step in 0..=1000 {
            let u = step as f32 / 1000.0;
            let runtime = interpolate_stops(&track.stops, track.kind, ease(easing, u));
            let css = css_value(&compiled, &track.name, u);
            match (&runtime, &css) {
                (Value::String(a), Value::String(b)) => {
                    let (a, b) = (
                        Color::from_hex(a).expect("hex"),
                        Color::from_hex(b).expect("hex"),
                    );
                    for (x, y) in [
                        (a.r(), b.r()),
                        (a.g(), b.g()),
                        (a.b(), b.b()),
                        (a.a(), b.a()),
                    ] {
                        assert!(
                            x.abs_diff(y) <= 3,
                            "{name}.{} at {u}: {runtime} vs {css}",
                            track.name
                        );
                    }
                }
                _ => {
                    let (a, b) = (runtime.as_f64().unwrap(), css.as_f64().unwrap());
                    assert!(
                        (a - b).abs() <= range.max(1e-6) * 0.005,
                        "{name}.{} at {u}: runtime {a} vs css {b}",
                        track.name
                    );
                }
            }
        }
    }
    compiled
}

#[test]
fn every_named_and_custom_curve_compiles_within_half_a_percent() {
    let three_stops = json!([
        {"offset": 0, "values": {"translateX": 0}},
        {"offset": 0.5, "values": {"translateX": 100}},
        {"offset": 1, "values": {"translateX": 40}}
    ]);
    for easing in [
        json!("linear"),
        json!("ease"),
        json!("easeIn"),
        json!("easeOut"),
        json!("easeInOut"),
        json!("standard"),
        json!("emphasized"),
        json!("emphasizedDecelerate"),
        json!("emphasizedAccelerate"),
        json!({"cubicBezier": [0.25, 0.1, 0.25, 1.0]}),
        json!({"cubicBezier": [0.34, 1.56, 0.64, 1.0]}),
        json!({"cubicBezier": [0.6, -0.28, 0.735, 0.045]}),
    ] {
        let name = easing.to_string();
        assert_compiles_faithfully(&name, &animation(easing, three_stops.clone()));
    }
}

#[test]
fn two_keyframes_with_a_named_curve_compile_to_its_own_bezier() {
    // CSS's own `ease` is a different curve; the compiled timing reproduces
    // the runtime's polynomial instead.
    let compiled = assert_compiles_faithfully(
        "ease opacity",
        &animation(
            json!("ease"),
            json!({"from": {"opacity": 0}, "to": {"opacity": 1}}),
        ),
    );
    assert_eq!(compiled.segments.len(), 1);
    let expected = [1.0 / 3.0, 0.0, 2.0 / 3.0, 1.0];
    for (actual, expected) in compiled.segments[0].bezier.iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "{:?}",
            compiled.segments[0].bezier
        );
    }
}

#[test]
fn segments_start_and_end_on_the_authored_values() {
    // The runtime's coarse bezier solve starts this slide at 15.9997; the
    // export starts at the authored 16.
    let compiled = assert_compiles_faithfully(
        "decelerate slide",
        &animation(
            json!("emphasizedDecelerate"),
            json!({"from": {"translateY": 16}, "to": {"translateY": 0}}),
        ),
    );
    let first = compiled.segments.first().expect("segments");
    let last = compiled.segments.last().expect("segments");
    assert_eq!(first.from["translateY"].as_f64(), Some(16.0));
    assert_eq!(last.to["translateY"].as_f64(), Some(0.0));
}

#[test]
fn colours_sparse_properties_and_many_stops_compile() {
    assert_compiles_faithfully(
        "colour",
        &animation(
            json!("ease"),
            json!({"from": {"fill": "#ff0000"}, "to": {"fill": "#0000ff"}}),
        ),
    );
    assert_compiles_faithfully(
        "five stops",
        &animation(
            json!("easeInOut"),
            json!([
                {"offset": 0, "values": {"opacity": 0}},
                {"offset": 0.25, "values": {"opacity": 1}},
                {"offset": 0.5, "values": {"opacity": 0.2}},
                {"offset": 0.75, "values": {"opacity": 0.8}},
                {"offset": 1, "values": {"opacity": 0.5}}
            ]),
        ),
    );
    assert_compiles_faithfully(
        "sparse",
        &animation(
            json!("emphasizedDecelerate"),
            json!([
                {"offset": 0, "values": {"opacity": 0, "scaleX": 0.9}},
                {"offset": 0.4, "values": {"opacity": 1}},
                {"offset": 1, "values": {"scaleX": 1}}
            ]),
        ),
    );
}

#[test]
fn an_overshooting_curve_holds_while_the_runtime_clamps() {
    let compiled = assert_compiles_faithfully(
        "overshoot",
        &animation(
            json!({"cubicBezier": [0.34, 1.56, 0.64, 1.0]}),
            json!({"from": {"translateX": 0}, "to": {"translateX": 100}}),
        ),
    );
    assert!(compiled.segments.iter().any(|segment| {
        segment.from == segment.to && segment.from["translateX"].as_f64() == Some(100.0)
    }));
}

#[test]
fn too_many_cuts_fall_back_to_sampled_linear_pieces() {
    let keyframes: Vec<Value> = (0..=80)
        .map(|index| {
            json!({"offset": index as f32 / 80.0, "values": {"opacity": (index % 2) as f32}})
        })
        .collect();
    let compiled = compile(&animation(json!("standard"), Value::Array(keyframes)));
    assert!(compiled.degraded);
    assert_eq!(compiled.segments.len(), DEGRADED_SEGMENTS);
}
