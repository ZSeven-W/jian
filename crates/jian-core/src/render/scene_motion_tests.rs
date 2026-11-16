use super::*;
use crate::action::services::AnimationProperty::*;
use crate::geometry::point;
use crate::render::scene_motion::PlaybackInteraction;
use crate::scene::Color;
use crate::Runtime;
use serde_json::json;

#[test]
fn motion_stream_keeps_nested_scopes_and_paints_p1_overrides() {
    let mut runtime = Runtime::new();
    runtime
        .load_str(
            r##"{"version":"1.1","children":[{
        "type":"frame","id":"root","width":100,"height":100,"children":[{
            "type":"rectangle","id":"child","x":20,"y":30,"width":20,"height":20,
            "fill":[{"type":"solid","color":"#223344"}],
            "stroke":{"thickness":2,"fill":[{"type":"solid","color":"#000000"}]}
        }]
    }]}"##,
        )
        .unwrap();
    runtime.build_layout((100., 100.)).unwrap();
    let doc = runtime.document.as_ref().unwrap();
    let interaction = PlaybackInteraction::default();
    let mut overrides = MotionOverrides::new();
    let baseline = collect(doc, &runtime.layout, &runtime.state, None, None);
    let empty = collect(
        doc,
        &runtime.layout,
        &runtime.state,
        None,
        Some(&SceneMotion {
            overrides: &overrides,
            interaction: &interaction,
        }),
    );
    assert_eq!(format!("{baseline:?}"), format!("{empty:?}"));

    for (id, property, value) in [
        ("root", TranslateX, json!(8)),
        ("root", TranslateY, json!(10)),
        ("root", Opacity, json!(0.5)),
        ("child", Rotation, json!(90)),
        ("child", ScaleX, json!(2)),
        ("child", ScaleY, json!(0.5)),
        ("child", Fill, json!("#ff0000")),
        ("child", Stroke, json!("#00ff00")),
        ("child", CornerRadius, json!(6)),
    ] {
        overrides.insert((id.to_owned(), property), value);
    }
    let animated = collect(
        doc,
        &runtime.layout,
        &runtime.state,
        None,
        Some(&SceneMotion {
            overrides: &overrides,
            interaction: &interaction,
        }),
    );
    let [ScenePaintCommand::PushTransform(root), ScenePaintCommand::PushOpacity(alpha), ScenePaintCommand::PushTransform(child), ScenePaintCommand::Draw(DrawOp::RoundedRect { rect, radii, paint }), ScenePaintCommand::Pop, ScenePaintCommand::PopLayer, ScenePaintCommand::Pop] =
        animated.as_slice()
    else {
        panic!("unexpected command order: {animated:?}");
    };
    assert_eq!(*alpha, 0.5);
    assert_eq!(radii.tl, 6.0);
    assert_eq!(paint.fill, Some(Color::rgb(255, 0, 0)));
    assert_eq!(paint.stroke.as_ref().unwrap().color, Color::rgb(0, 255, 0));
    assert_eq!(*rect, crate::geometry::rect(20., 30., 20., 20.));
    let transform = child.then(root);
    assert_eq!(transform.transform_point(point(30., 40.)), point(38., 50.));
    assert_eq!(
        transform.transform_point(point(40., 50.)).round(),
        point(33., 70.)
    );
}
