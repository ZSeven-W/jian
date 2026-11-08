//! The scene-side binding table (`layout_scene_binding::apply_value`, used by
//! OpenPencil's Preview overlay and motion overrides) and jian-core's
//! schema-side table (`binding::application::apply_binding_value`, used by
//! render collection) must mean the same thing by a bound value.
//!
//! `classify` lists every `BindingTarget`, so a new variant does not compile
//! until someone decides how the two tables should agree on it.

use jian_core::binding::{apply_binding_value, BindingTarget};
use jian_core::value::RuntimeValue;
use jian_scene::layout_scene::{NodeKind, SceneNode, SceneWidget};
use jian_scene::layout_scene_binding::apply_value;
use jian_scene::layout_scene_motion::scene_color;
use jian_widgets::Rect;
use serde_json::{json, Map, Value};

#[derive(Debug, PartialEq)]
enum Parity {
    /// Both tables move the node's rect the same way.
    Geometry,
    /// Same number; the scene stores rotation in radians.
    Number,
    /// Same solid colour on the first fill / the stroke.
    Colour,
    Visibility,
    /// Same text for strings, integers and booleans (see the pinned
    /// divergences below for null and integral floats).
    Text,
    /// Same widget state field.
    Widget,
    /// Paint-only offsets: the scene shifts the subtree, the schema side
    /// deliberately leaves geometry alone for the host overlay to apply.
    SceneOnly,
    /// The schema side rewrites `ref` / `value`; the scene side toggles the
    /// painted children. They act on different layers, so only the widget
    /// arm is comparable.
    Structural,
}

fn classify(target: BindingTarget) -> Parity {
    use BindingTarget::*;
    match target {
        X | Y | Width | Height | ScaleX | ScaleY => Parity::Geometry,
        Opacity | CornerRadius | Rotation => Parity::Number,
        Fill | TextColor | Stroke => Parity::Colour,
        Visible => Parity::Visibility,
        Content => Parity::Text,
        Value | Checked | SelectedValue => Parity::Widget,
        TranslateX | TranslateY => Parity::SceneOnly,
        Variant | ActiveState => Parity::Structural,
    }
}

fn value(value: Value) -> RuntimeValue {
    RuntimeValue(value)
}

fn scene_node(widget: Option<&str>) -> SceneNode {
    let mut node = SceneNode::leaf("n", NodeKind::Rect);
    node.bounds = Rect::xywh(10.0, 20.0, 100.0, 40.0);
    node.widget = widget.map(|kind| SceneWidget {
        kind: kind.to_owned(),
        ..SceneWidget::default()
    });
    node
}

fn schema(kind: &str) -> Map<String, Value> {
    json!({ "type": kind, "x": 10, "y": 20, "width": 100, "height": 40 })
        .as_object()
        .cloned()
        .expect("object")
}

#[test]
fn geometry_targets_move_the_rect_identically() {
    for (target, bound) in [
        (BindingTarget::X, json!(30)),
        (BindingTarget::Y, json!(-5.5)),
        (BindingTarget::Width, json!(60)),
        (BindingTarget::Height, json!(12)),
        (BindingTarget::ScaleX, json!(0.5)),
        (BindingTarget::ScaleY, json!(2)),
    ] {
        assert_eq!(classify(target), Parity::Geometry);
        let application = apply_binding_value(
            &mut schema("rectangle"),
            target,
            &value(bound.clone()),
            true,
        );
        let expected =
            application.apply_to_rect(jian_core::geometry::rect(10.0, 20.0, 100.0, 40.0));
        let mut node = scene_node(None);
        apply_value(&mut node, target, &value(bound));
        assert_eq!(
            (node.bounds.origin.x, node.bounds.origin.y),
            (expected.origin.x, expected.origin.y),
            "{target:?}"
        );
        assert_eq!(
            (node.bounds.size.x, node.bounds.size.y),
            (expected.size.width, expected.size.height),
            "{target:?}"
        );
    }
}

#[test]
fn paint_targets_store_the_same_values() {
    let mut object = schema("rectangle");
    let mut node = scene_node(None);
    for (target, bound) in [
        (BindingTarget::Opacity, json!(0.4)),
        (BindingTarget::CornerRadius, json!(6)),
        (BindingTarget::Rotation, json!(90)),
        (BindingTarget::Fill, json!("#336699")),
        (BindingTarget::Stroke, json!("#00ff0080")),
        (BindingTarget::Visible, json!(false)),
    ] {
        assert!(matches!(
            classify(target),
            Parity::Number | Parity::Colour | Parity::Visibility
        ));
        apply_binding_value(&mut object, target, &value(bound.clone()), true);
        apply_value(&mut node, target, &value(bound));
    }
    let number = |key: &str| object[key].as_f64().expect("number") as f32;
    assert_eq!(node.opacity, number("opacity"));
    assert_eq!(node.corner_radius, number("cornerRadius"));
    assert!((node.rotation.to_degrees() - number("rotation")).abs() < 1e-4);
    assert_eq!(
        scene_color(node.fill.expect("fill")),
        object["fill"][0]["color"]
    );
    let stroke = node.stroke.expect("stroke");
    assert_eq!(
        scene_color(stroke.color),
        object["stroke"]["fill"][0]["color"]
    );
    assert_eq!(f64::from(stroke.width), object["stroke"]["thickness"]);
    assert_eq!(node.hidden, object["visible"] == json!(false));
}

#[test]
fn text_agrees_for_strings_integers_and_booleans() {
    for bound in [json!("hello"), json!(42), json!(true), json!(-3)] {
        let mut object = schema("text");
        apply_binding_value(
            &mut object,
            BindingTarget::Content,
            &value(bound.clone()),
            true,
        );
        let mut node = scene_node(None);
        apply_value(&mut node, BindingTarget::Content, &value(bound.clone()));
        assert_eq!(node.text.as_deref(), object["content"].as_str(), "{bound}");
    }
}

/// Known differences carried over unchanged from the two original tables.
/// Changing either side changes what Preview or render collection shows, so
/// it needs a decision; update this test together with that change.
#[test]
fn pinned_text_divergences() {
    let content = |bound: Value| {
        let mut object = schema("text");
        object.insert("content".into(), json!("before"));
        apply_binding_value(&mut object, BindingTarget::Content, &value(bound), true);
        object["content"].as_str().map(str::to_owned)
    };
    let scene_text = |bound: Value| {
        let mut node = scene_node(None);
        apply_value(&mut node, BindingTarget::Content, &value(bound));
        node.text
    };
    // Null: the scene shows empty text, the schema keeps the authored text.
    assert_eq!(scene_text(Value::Null).as_deref(), Some(""));
    assert_eq!(content(Value::Null).as_deref(), Some("before"));
    // Integral floats: JSON form on the scene, Rust's shortest form in the
    // schema.
    assert_eq!(scene_text(json!(2.0)).as_deref(), Some("2.0"));
    assert_eq!(content(json!(2.0)).as_deref(), Some("2"));
}

#[test]
fn widget_targets_write_the_same_state() {
    let cases = [
        (BindingTarget::Value, "slider", json!(30), "value"),
        (BindingTarget::Checked, "checkbox", json!(true), "checked"),
        (BindingTarget::SelectedValue, "select", json!("b"), "value"),
        (BindingTarget::ActiveState, "switch", json!(true), "checked"),
        (BindingTarget::ActiveState, "tabs", json!("b"), "value"),
    ];
    for (target, kind, bound, field) in cases {
        assert!(matches!(
            classify(target),
            Parity::Widget | Parity::Structural
        ));
        let mut object = schema(kind);
        apply_binding_value(&mut object, target, &value(bound.clone()), true);
        let mut node = scene_node(Some(kind));
        apply_value(&mut node, target, &value(bound));
        let widget = node.widget.expect("widget");
        let scene = match kind {
            "slider" => json!(widget.value_num),
            "checkbox" | "switch" => json!(widget.checked),
            _ => json!(widget.value_str),
        };
        // Compare numbers by value: the scene keeps f32, the schema JSON
        // keeps the bound integer.
        let schema = object[field]
            .as_f64()
            .map_or_else(|| object[field].clone(), |number| json!(number as f32));
        assert_eq!(scene, schema, "{target:?} on {kind}");
    }
}

#[test]
fn translation_stays_off_schema_geometry() {
    for target in [BindingTarget::TranslateX, BindingTarget::TranslateY] {
        assert_eq!(classify(target), Parity::SceneOnly);
        let mut object = schema("rectangle");
        let before = object.clone();
        let application = apply_binding_value(&mut object, target, &value(json!(8)), true);
        assert_eq!(object, before);
        assert_eq!(application, Default::default());
        let mut node = scene_node(None);
        apply_value(&mut node, target, &value(json!(8)));
        assert_ne!(node.bounds, Rect::xywh(10.0, 20.0, 100.0, 40.0));
    }
}
