use super::*;
use crate::motion::TimedTransition;

const VIEWPORT: (f32, f32) = (390.0, 844.0);

fn layer(screen: TransitionScreen, dx: f32, dy: f32, alpha: f32) -> TransitionLayer {
    TransitionLayer {
        screen,
        dx,
        dy,
        alpha,
    }
}

fn assert_layers(actual: [TransitionLayer; 2], expected: [TransitionLayer; 2]) {
    for (actual, expected) in actual.iter().zip(expected.iter()) {
        assert_eq!(actual.screen, expected.screen, "{actual:?} vs {expected:?}");
        for (a, e) in [
            (actual.dx, expected.dx),
            (actual.dy, expected.dy),
            (actual.alpha, expected.alpha),
        ] {
            assert!((a - e).abs() < 1e-3, "{actual:?} vs {expected:?}");
        }
    }
}

fn spec(kind: ScreenTransitionKind) -> ScreenTransitionSpec {
    ScreenTransitionSpec {
        kind,
        ..ScreenTransitionSpec::default()
    }
}

fn golden() -> serde_json::Value {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/motion/transitions.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("parse")
}

#[test]
fn the_default_spec_reproduces_the_recorded_transitions() {
    let (width, _) = VIEWPORT;
    for transition in golden()["transitions"].as_array().expect("transitions") {
        let navigation = match transition["kind"].as_str().expect("kind") {
            "push" => NavigationKind::Push,
            "pop" => NavigationKind::Pop,
            _ => NavigationKind::Replace,
        };
        let resolved = ResolvedScreenTransition::resolve(None, navigation);
        assert_eq!(
            resolved.duration_ms,
            transition["durationMs"].as_u64().expect("duration")
        );
        let clock = TimedTransition::new(0, resolved.duration_ms);
        for sample in transition["samples"].as_array().expect("samples") {
            let t = sample["t"].as_u64().expect("t");
            let linear = sample["linear"].as_f64().expect("linear") as f32;
            let eased = sample["eased"].as_f64().expect("eased") as f32;
            let layers = resolved.layers_at(clock.linear_t(t), VIEWPORT);
            let expected = match navigation {
                NavigationKind::Push => [
                    layer(TransitionScreen::Outgoing, 0.0, 0.0, 1.0),
                    layer(TransitionScreen::Entering, width * (1.0 - eased), 0.0, 1.0),
                ],
                NavigationKind::Pop => [
                    layer(TransitionScreen::Entering, 0.0, 0.0, 1.0),
                    layer(TransitionScreen::Outgoing, width * eased, 0.0, 1.0),
                ],
                NavigationKind::Replace => [
                    layer(TransitionScreen::Outgoing, 0.0, 0.0, 1.0 - linear),
                    layer(TransitionScreen::Entering, 0.0, 0.0, linear),
                ],
            };
            assert_layers(layers, expected);
        }
    }
}

#[test]
fn every_kind_starts_off_screen_or_clear_and_ends_in_place() {
    let (w, h) = VIEWPORT;
    let entering = TransitionScreen::Entering;
    let outgoing = TransitionScreen::Outgoing;
    let cases = [
        (ScreenTransitionKind::SlideLeft, (w, 0.0)),
        (ScreenTransitionKind::SlideRight, (-w, 0.0)),
        (ScreenTransitionKind::SlideUp, (0.0, h)),
        (ScreenTransitionKind::SlideDown, (0.0, -h)),
    ];
    for (kind, (dx, dy)) in cases {
        let push = ResolvedScreenTransition::resolve(Some(&spec(kind)), NavigationKind::Push);
        assert_eq!(push.duration_ms, DEFAULT_SLIDE_MS, "{kind:?}");
        assert_layers(
            push.layers_at(0.0, VIEWPORT),
            [layer(outgoing, 0.0, 0.0, 1.0), layer(entering, dx, dy, 1.0)],
        );
        assert_layers(
            push.layers_at(1.0, VIEWPORT),
            [
                layer(outgoing, 0.0, 0.0, 1.0),
                layer(entering, 0.0, 0.0, 1.0),
            ],
        );
        // Going back plays the slide in reverse: the screen leaves the way
        // it came in.
        let pop = ResolvedScreenTransition::resolve(Some(&spec(kind)), NavigationKind::Pop);
        assert_layers(
            pop.layers_at(0.0, VIEWPORT),
            [
                layer(entering, 0.0, 0.0, 1.0),
                layer(outgoing, 0.0, 0.0, 1.0),
            ],
        );
        assert_layers(
            pop.layers_at(1.0, VIEWPORT),
            [layer(entering, 0.0, 0.0, 1.0), layer(outgoing, dx, dy, 1.0)],
        );
    }

    let fade = ResolvedScreenTransition::resolve(
        Some(&spec(ScreenTransitionKind::Fade)),
        NavigationKind::Push,
    );
    assert_eq!(fade.duration_ms, DEFAULT_FADE_MS);
    assert_layers(
        fade.layers_at(0.0, VIEWPORT),
        [
            layer(outgoing, 0.0, 0.0, 1.0),
            layer(entering, 0.0, 0.0, 0.0),
        ],
    );
    assert_layers(
        fade.layers_at(1.0, VIEWPORT),
        [
            layer(outgoing, 0.0, 0.0, 0.0),
            layer(entering, 0.0, 0.0, 1.0),
        ],
    );
}

#[test]
fn none_switches_at_once() {
    let cut = ResolvedScreenTransition::resolve(
        Some(&ScreenTransitionSpec {
            kind: ScreenTransitionKind::None,
            duration_ms: Some(500),
            easing: None,
        }),
        NavigationKind::Push,
    );
    assert_eq!(cut.duration_ms, 0);
    assert!(!TimedTransition::new(0, cut.duration_ms).is_active(0));
    assert_layers(
        cut.layers_at(0.0, VIEWPORT),
        [
            layer(TransitionScreen::Outgoing, 0.0, 0.0, 0.0),
            layer(TransitionScreen::Entering, 0.0, 0.0, 1.0),
        ],
    );
}

#[test]
fn authored_duration_and_easing_replace_the_kind_defaults() {
    let authored = ScreenTransitionSpec {
        kind: ScreenTransitionKind::SlideUp,
        duration_ms: Some(300),
        easing: Some(SchemaEasing::Linear),
    };
    let resolved = ResolvedScreenTransition::resolve(Some(&authored), NavigationKind::Replace);
    assert_eq!(resolved.duration_ms, 300);
    assert!((resolved.progress(0.25) - 0.25).abs() < 1e-6);
    // Replace plays a slide forward, like push.
    assert_layers(
        resolved.layers_at(0.25, VIEWPORT),
        [
            layer(TransitionScreen::Outgoing, 0.0, 0.0, 1.0),
            layer(TransitionScreen::Entering, 0.0, VIEWPORT.1 * 0.75, 1.0),
        ],
    );
}

#[test]
fn the_spec_reads_and_writes_the_action_json_shape() {
    let parsed: ScreenTransitionSpec =
        serde_json::from_value(serde_json::json!({"kind": "slideUp", "durationMs": 300}))
            .expect("spec");
    assert_eq!(parsed.kind, ScreenTransitionKind::SlideUp);
    assert_eq!(parsed.duration_ms, Some(300));
    assert_eq!(
        serde_json::to_value(&parsed).expect("json"),
        serde_json::json!({"kind": "slideUp", "durationMs": 300})
    );
    let empty: ScreenTransitionSpec = serde_json::from_value(serde_json::json!({})).expect("spec");
    assert_eq!(empty, ScreenTransitionSpec::default());
    assert!(
        serde_json::from_value::<ScreenTransitionSpec>(serde_json::json!({"kind": "spin"}))
            .is_err()
    );
}
