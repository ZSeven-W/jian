//! Value interpolation between motion keyframes, moved unchanged from
//! OpenPencil's preview runtime. Numbers interpolate linearly, colours per
//! sRGB channel with rounding, and discrete properties switch at the end.

use crate::action::animation_registry::{animatable_property_registry, AnimationInterpolate};
use crate::action::services::AnimationProperty;
use crate::scene::Color;

/// How a property interpolates: the registry entry, with the internal
/// switch toggle progress always linear and unknown properties discrete.
pub fn interpolate_kind(property: &AnimationProperty) -> AnimationInterpolate {
    match property {
        AnimationProperty::ToggleProgress => AnimationInterpolate::Linear,
        other => animatable_property_registry()
            .get(other.name())
            .map(|entry| entry.interpolate)
            .unwrap_or(AnimationInterpolate::Discrete),
    }
}

/// Sample ordered `(progress, value)` stops at `progress`. Before the first
/// stop holds the first value; after the last stop holds the last.
pub fn interpolate_stops(
    stops: &[(f32, serde_json::Value)],
    interpolate: AnimationInterpolate,
    progress: f32,
) -> serde_json::Value {
    let Some(first) = stops.first() else {
        return serde_json::Value::Null;
    };
    if progress <= first.0 {
        return first.1.clone();
    }
    for pair in stops.windows(2) {
        let [from, to] = pair else { unreachable!() };
        if progress <= to.0 {
            let span = to.0 - from.0;
            let local = if span <= f32::EPSILON {
                1.0
            } else {
                (progress - from.0) / span
            };
            return interpolate_pair(&from.1, &to.1, interpolate, local);
        }
    }
    stops.last().expect("checked first").1.clone()
}

pub fn interpolate_pair(
    from: &serde_json::Value,
    to: &serde_json::Value,
    interpolate: AnimationInterpolate,
    progress: f32,
) -> serde_json::Value {
    match interpolate {
        AnimationInterpolate::Linear => interpolate_number(from, to, progress),
        AnimationInterpolate::ColorSrgb => interpolate_color(from, to, progress),
        AnimationInterpolate::Discrete => {
            if progress >= 1.0 {
                to.clone()
            } else {
                from.clone()
            }
        }
    }
}

pub fn interpolate_number(
    from: &serde_json::Value,
    to: &serde_json::Value,
    progress: f32,
) -> serde_json::Value {
    let from = from.as_f64().unwrap_or_default();
    let to = to.as_f64().unwrap_or_default();
    serde_json::Number::from_f64(from + (to - from) * f64::from(progress))
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

pub fn interpolate_color(
    from: &serde_json::Value,
    to: &serde_json::Value,
    progress: f32,
) -> serde_json::Value {
    let Some(from) = from.as_str().and_then(Color::from_hex) else {
        return from.clone();
    };
    let Some(to) = to.as_str().and_then(Color::from_hex) else {
        return serde_json::Value::String(color_hex(from));
    };
    let channel = |from: u8, to: u8| {
        (f32::from(from) + (f32::from(to) - f32::from(from)) * progress)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    serde_json::Value::String(color_hex(Color::rgba(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
        channel(from.a(), to.a()),
    )))
}

/// `#rrggbb` for opaque colours, `#rrggbbaa` otherwise.
pub fn color_hex(color: Color) -> String {
    if color.a() == 255 {
        format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
    } else {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            color.r(),
            color.g(),
            color.b(),
            color.a()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stops_hold_their_ends_and_interpolate_between() {
        let stops = [(0.0, json!(0.0)), (0.5, json!(100.0)), (1.0, json!(40.0))];
        let linear = AnimationInterpolate::Linear;
        assert_eq!(interpolate_stops(&stops, linear, -1.0), json!(0.0));
        assert_eq!(interpolate_stops(&stops, linear, 0.25), json!(50.0));
        assert_eq!(interpolate_stops(&stops, linear, 0.75), json!(70.0));
        assert_eq!(interpolate_stops(&stops, linear, 2.0), json!(40.0));
        assert_eq!(interpolate_stops(&[], linear, 0.5), serde_json::Value::Null);
    }

    #[test]
    fn colours_round_per_channel_and_keep_alpha_form() {
        assert_eq!(
            interpolate_color(&json!("#ff0000"), &json!("#0000ff"), 0.5),
            json!("#800080")
        );
        assert_eq!(
            interpolate_color(&json!("#ff000080"), &json!("#ff0000ff"), 0.0),
            json!("#ff000080")
        );
        assert_eq!(
            interpolate_color(&json!("nope"), &json!("#000000"), 0.5),
            json!("nope")
        );
    }

    #[test]
    fn discrete_values_switch_at_the_end() {
        let discrete = AnimationInterpolate::Discrete;
        assert_eq!(
            interpolate_pair(&json!(1), &json!(2), discrete, 0.99),
            json!(1)
        );
        assert_eq!(
            interpolate_pair(&json!(1), &json!(2), discrete, 1.0),
            json!(2)
        );
    }
}
