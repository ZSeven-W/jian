use super::BindingTarget;
use crate::value::RuntimeValue;
use serde_json::Value;
/// Geometry overrides produced while applying one binding to a render node.
/// The same typed target table drives classification and value application.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BindingApplication {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub scale_x: Option<f32>,
    pub scale_y: Option<f32>,
}

impl BindingApplication {
    pub fn merge(&mut self, other: Self) {
        if other.x.is_some() {
            self.x = other.x;
        }
        if other.y.is_some() {
            self.y = other.y;
        }
        if other.width.is_some() {
            self.width = other.width;
        }
        if other.height.is_some() {
            self.height = other.height;
        }
        if other.scale_x.is_some() {
            self.scale_x = other.scale_x;
        }
        if other.scale_y.is_some() {
            self.scale_y = other.scale_y;
        }
    }

    pub fn apply_to_rect(self, rect: crate::geometry::Rect) -> crate::geometry::Rect {
        let base_width = self.width.unwrap_or(rect.size.width);
        let base_height = self.height.unwrap_or(rect.size.height);
        let base_x = self.x.unwrap_or(rect.origin.x);
        let base_y = self.y.unwrap_or(rect.origin.y);
        let width = base_width * self.scale_x.unwrap_or(1.0);
        let height = base_height * self.scale_y.unwrap_or(1.0);
        crate::geometry::rect(
            base_x + (base_width - width) / 2.0,
            base_y + (base_height - height) / 2.0,
            width,
            height,
        )
    }
}

/// Apply one typed binding value to a schema JSON view. Render collection and
/// Preview overlay both use this contract instead of maintaining property
/// matches with different supported sets.
pub fn apply_binding_value(
    object: &mut serde_json::Map<String, Value>,
    target: BindingTarget,
    value: &RuntimeValue,
    allow_rect_overrides: bool,
) -> BindingApplication {
    let mut application = BindingApplication::default();
    match target {
        BindingTarget::Content => {
            if let Some(content) = bound_scalar_to_string(value) {
                object.insert("content".into(), Value::String(content));
            }
        }
        BindingTarget::Value => {
            if let Some(value) = bound_scalar_to_json(value) {
                object.insert("value".into(), value);
            }
        }
        BindingTarget::Checked => {
            if let Some(checked) = value.as_bool() {
                object.insert("checked".into(), Value::Bool(checked));
            }
        }
        BindingTarget::SelectedValue => {
            if let Some(selected) = value.as_str() {
                object.insert("value".into(), Value::String(selected.to_owned()));
            }
        }
        BindingTarget::Visible => {
            if let Some(visible) = value.as_bool() {
                object.insert("visible".into(), Value::Bool(visible));
            }
        }
        BindingTarget::Opacity => {
            insert_number(object, "opacity", value);
        }
        BindingTarget::Fill | BindingTarget::TextColor => {
            if let Some(color) = value.as_str() {
                set_first_fill_color(object, color);
            }
        }
        BindingTarget::Stroke => {
            if let Some(color) = value.as_str() {
                set_stroke_color(object, color);
            }
        }
        BindingTarget::CornerRadius => {
            insert_number(object, "cornerRadius", value);
        }
        BindingTarget::X => {
            insert_number(object, "x", value);
            if allow_rect_overrides {
                application.x = number_from_runtime(value).map(|number| number as f32);
            }
        }
        BindingTarget::Y => {
            insert_number(object, "y", value);
            if allow_rect_overrides {
                application.y = number_from_runtime(value).map(|number| number as f32);
            }
        }
        // Paint-only translation is carried to the host overlay. It must not
        // materialize as schema geometry or contribute a rect override.
        BindingTarget::TranslateX | BindingTarget::TranslateY => {}
        BindingTarget::Width => {
            insert_number(object, "width", value);
            if allow_rect_overrides {
                application.width = number_from_runtime(value).map(|number| number as f32);
            }
        }
        BindingTarget::Height => {
            insert_number(object, "height", value);
            if allow_rect_overrides {
                application.height = number_from_runtime(value).map(|number| number as f32);
            }
        }
        BindingTarget::Rotation => {
            insert_number(object, "rotation", value);
        }
        BindingTarget::ScaleX => {
            if allow_rect_overrides {
                application.scale_x = number_from_runtime(value).map(|number| number as f32);
            }
        }
        BindingTarget::ScaleY => {
            if allow_rect_overrides {
                application.scale_y = number_from_runtime(value).map(|number| number as f32);
            }
        }
        BindingTarget::Variant => {
            if object.get("type").and_then(Value::as_str) == Some("ref") {
                if let Some(variant) = value.as_str() {
                    object.insert("ref".into(), Value::String(variant.to_owned()));
                }
            }
        }
        BindingTarget::ActiveState => {
            let kind = object.get("type").and_then(Value::as_str);
            match kind {
                Some("switch" | "checkbox") => {
                    if let Some(active) = value.as_bool() {
                        object.insert("checked".into(), Value::Bool(active));
                    }
                }
                _ => {
                    if let Some(active) = value.as_str() {
                        object.insert("value".into(), Value::String(active.to_owned()));
                    }
                }
            }
        }
    }
    application
}

pub(crate) fn bound_scalar_to_json(value: &RuntimeValue) -> Option<Value> {
    matches!(
        &value.0,
        Value::String(_) | Value::Number(_) | Value::Bool(_)
    )
    .then(|| value.0.clone())
}

pub(crate) fn number_from_runtime(value: &RuntimeValue) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|number| number as f64))
}

pub(crate) fn bound_scalar_to_string(value: &RuntimeValue) -> Option<String> {
    if value.is_null() {
        return None;
    }
    if let Some(string) = value.as_str() {
        return Some(string.to_owned());
    }
    if let Some(boolean) = value.as_bool() {
        return Some(boolean.to_string());
    }
    if let Some(integer) = value.as_i64() {
        return Some(integer.to_string());
    }
    if let Some(number) = value.as_f64() {
        return Some(number.to_string());
    }
    Some(String::new())
}

fn insert_number(
    object: &mut serde_json::Map<String, Value>,
    property: &str,
    value: &RuntimeValue,
) {
    if let Some(number) = number_from_runtime(value).and_then(serde_json::Number::from_f64) {
        object.insert(property.to_owned(), Value::Number(number));
    }
}

fn set_first_fill_color(object: &mut serde_json::Map<String, Value>, color: &str) {
    let entry = object
        .entry("fill".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(fills) = entry.as_array_mut() else {
        return;
    };
    if fills.is_empty() {
        fills.push(serde_json::json!({ "type": "solid", "color": color }));
        return;
    }
    let Some(first) = fills[0].as_object_mut() else {
        return;
    };
    match first.get("type").and_then(Value::as_str) {
        None | Some("solid") => {
            first.insert("type".into(), Value::String("solid".into()));
            first.insert("color".into(), Value::String(color.to_owned()));
        }
        _ => {}
    }
}

fn set_stroke_color(object: &mut serde_json::Map<String, Value>, color: &str) {
    let stroke = object.entry("stroke".to_owned()).or_insert_with(|| {
        serde_json::json!({
            "thickness": 1,
            "fill": [{ "type": "solid", "color": color }]
        })
    });
    let Some(stroke) = stroke.as_object_mut() else {
        return;
    };
    let fills = stroke
        .entry("fill".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(fills) = fills.as_array_mut() else {
        return;
    };
    if fills.is_empty() {
        fills.push(serde_json::json!({ "type": "solid", "color": color }));
    } else if let Some(first) = fills[0].as_object_mut() {
        first.insert("type".into(), Value::String("solid".into()));
        first.insert("color".into(), Value::String(color.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_application_writes_paint_and_centered_geometry() {
        let mut node = serde_json::json!({
            "type": "rectangle",
            "id": "card",
            "fill": [{ "type": "solid", "color": "#0000ff" }]
        });
        let object = node.as_object_mut().unwrap();
        let _ = apply_binding_value(
            object,
            BindingTarget::Fill,
            &RuntimeValue(serde_json::json!("#ff0000")),
            true,
        );
        let mut geometry = apply_binding_value(
            object,
            BindingTarget::Width,
            &RuntimeValue(serde_json::json!(100)),
            true,
        );
        geometry.merge(apply_binding_value(
            object,
            BindingTarget::ScaleX,
            &RuntimeValue(serde_json::json!(2)),
            true,
        ));
        assert_eq!(node["fill"][0]["color"], "#ff0000");
        assert_eq!(
            geometry.apply_to_rect(crate::geometry::rect(10.0, 20.0, 50.0, 40.0)),
            crate::geometry::rect(-40.0, 20.0, 200.0, 40.0)
        );
    }

    #[test]
    fn active_state_and_variant_have_real_structural_writes() {
        let mut tabs = serde_json::json!({"type":"tabs","id":"tabs","value":"a"});
        let _ = apply_binding_value(
            tabs.as_object_mut().unwrap(),
            BindingTarget::ActiveState,
            &RuntimeValue(serde_json::json!("b")),
            true,
        );
        assert_eq!(tabs["value"], "b");

        let mut reference = serde_json::json!({"type":"ref","id":"instance","ref":"a"});
        let _ = apply_binding_value(
            reference.as_object_mut().unwrap(),
            BindingTarget::Variant,
            &RuntimeValue(serde_json::json!("b")),
            true,
        );
        assert_eq!(reference["ref"], "b");
    }

    #[test]
    fn translate_targets_are_passed_through_without_layout_changes() {
        for target in [BindingTarget::TranslateX, BindingTarget::TranslateY] {
            let mut node = serde_json::json!({
                "type": "rectangle",
                "id": "card",
                "x": 10,
                "y": 20,
            });
            let application = apply_binding_value(
                node.as_object_mut().unwrap(),
                target,
                &RuntimeValue(serde_json::json!(32)),
                true,
            );
            assert_eq!(application, BindingApplication::default());
            assert_eq!(node["x"], 10);
            assert_eq!(node["y"], 20);
        }
    }
}
