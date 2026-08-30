//! Structured animate action backed by the canonical property registry.

use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::animation_registry::{animatable_property_registry, AnimationValueType};
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::action::services::{
    AnimationDirection, AnimationFillMode, AnimationOutcome, AnimationProperty, AnimationRequest,
    Easing,
};
use async_trait::async_trait;
use serde_json::Value;

const MAX_ITERATIONS: u32 = 1000;

struct Animate {
    request: AnimationRequest,
}

#[async_trait(?Send)]
impl ActionImpl for Animate {
    fn name(&self) -> &'static str {
        "animate"
    }

    async fn execute(&self, context: &ActionContext) -> ActionResult {
        let mut request = self.request.clone();
        request.requested_at_ms = context.now_ms();
        match context.animation_sink.request(&request) {
            AnimationOutcome::Accepted => {}
            AnimationOutcome::Unsupported => warn(context, "animate: animation sink unavailable"),
            AnimationOutcome::Rejected(detail) => {
                warn(context, &format!("animate: {detail}"));
            }
        }
        Ok(())
    }
}

fn warn(context: &ActionContext, message: &str) {
    context.warn(crate::expression::Diagnostic {
        kind: crate::expression::DiagKind::RuntimeWarning,
        message: message.to_owned(),
        span: crate::expression::Span::zero(),
    });
}

pub fn factory_animate(body: &Value) -> Result<BoxedAction, ActionError> {
    let object = body.as_object().ok_or(ActionError::FieldType {
        name: "animate",
        field: "body",
        message: "must be an object".into(),
    })?;
    let target = required_string(object, "target")?;
    if target.trim().is_empty() {
        return Err(field_error("target", "must be a non-empty node id"));
    }
    let property_name = required_string(object, "property")?;
    let descriptor = animatable_property_registry()
        .get(property_name)
        .ok_or_else(|| ActionError::UnknownAnimatableProperty {
            property: property_name.to_owned(),
        })?;
    let property = AnimationProperty::from_registered(property_name, descriptor.apply);
    let to = object.get("to").ok_or(ActionError::MissingField {
        name: "animate",
        field: "to",
    })?;
    validate_value("to", to, descriptor.value_type)?;
    let from = object.get("from").cloned();
    if let Some(from) = &from {
        validate_value("from", from, descriptor.value_type)?;
    }
    let duration_ms = required_u64(object, "durationMs")?;
    if duration_ms == 0 {
        return Err(field_error("durationMs", "must be greater than zero"));
    }
    let delay_ms = optional_u64(object, "delayMs", 0)?;
    let iterations_raw = optional_u64(object, "iterations", 1)?;
    let iterations = u32::try_from(iterations_raw)
        .ok()
        .filter(|iterations| (1..=MAX_ITERATIONS).contains(iterations))
        .ok_or_else(|| {
            field_error(
                "iterations",
                "must be an integer between 1 and 1000 inclusive",
            )
        })?;
    let _total_end = duration_ms
        .checked_mul(u64::from(iterations))
        .and_then(|duration| delay_ms.checked_add(duration))
        .ok_or_else(|| field_error("durationMs", "delay plus iterations overflows u64"))?;
    let easing = parse_easing(optional_string(object, "easing", "linear")?)?;
    let direction = parse_direction(optional_string(object, "direction", "normal")?)?;
    let fill_mode = parse_fill_mode(optional_string(object, "fillMode", "none")?)?;
    Ok(Box::new(Animate {
        request: AnimationRequest {
            target: target.trim().to_owned(),
            property,
            from,
            to: to.clone(),
            duration_ms,
            delay_ms,
            easing,
            iterations,
            direction,
            fill_mode,
            requested_at_ms: 0,
        },
    }))
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, ActionError> {
    let value = object.get(field).ok_or(ActionError::MissingField {
        name: "animate",
        field,
    })?;
    value
        .as_str()
        .ok_or_else(|| field_error(field, "must be a string"))
}

fn optional_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &'static str,
    default: &'a str,
) -> Result<&'a str, ActionError> {
    object
        .get(field)
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| field_error(field, "must be a string"))
        })
        .unwrap_or(Ok(default))
}

fn required_u64(
    object: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<u64, ActionError> {
    let value = object.get(field).ok_or(ActionError::MissingField {
        name: "animate",
        field,
    })?;
    value
        .as_u64()
        .ok_or_else(|| field_error(field, "must be a non-negative integer"))
}

fn optional_u64(
    object: &serde_json::Map<String, Value>,
    field: &'static str,
    default: u64,
) -> Result<u64, ActionError> {
    object
        .get(field)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| field_error(field, "must be a non-negative integer"))
        })
        .unwrap_or(Ok(default))
}

fn validate_value(
    field: &'static str,
    value: &Value,
    value_type: AnimationValueType,
) -> Result<(), ActionError> {
    match value_type {
        AnimationValueType::Number | AnimationValueType::Length | AnimationValueType::Angle => {
            let Some(number) = value.as_f64() else {
                return Err(field_error(field, "must be a finite number"));
            };
            if !number.is_finite() {
                return Err(field_error(field, "must be a finite number"));
            }
        }
        AnimationValueType::Color => {
            let color = value
                .as_str()
                .and_then(crate::scene::Color::from_hex)
                .ok_or_else(|| field_error(field, "must be a hex color"))?;
            let _ = color;
        }
    }
    Ok(())
}

fn parse_easing(authored: &str) -> Result<Easing, ActionError> {
    match authored {
        "linear" => Ok(Easing::Linear),
        "ease" => Ok(Easing::Ease),
        "ease_in" | "ease-in" => Ok(Easing::EaseIn),
        "ease_out" | "ease-out" => Ok(Easing::EaseOut),
        "ease_in_out" | "ease-in-out" => Ok(Easing::EaseInOut),
        _ => Err(field_error("easing", "unknown easing")),
    }
}

fn parse_direction(authored: &str) -> Result<AnimationDirection, ActionError> {
    match authored {
        "normal" => Ok(AnimationDirection::Normal),
        "reverse" => Ok(AnimationDirection::Reverse),
        "alternate" => Ok(AnimationDirection::Alternate),
        "alternate_reverse" | "alternate-reverse" => Ok(AnimationDirection::AlternateReverse),
        _ => Err(field_error("direction", "unknown direction")),
    }
}

fn parse_fill_mode(authored: &str) -> Result<AnimationFillMode, ActionError> {
    match authored {
        "none" => Ok(AnimationFillMode::None),
        "forwards" => Ok(AnimationFillMode::Forwards),
        "backwards" => Ok(AnimationFillMode::Backwards),
        "both" => Ok(AnimationFillMode::Both),
        _ => Err(field_error("fillMode", "unknown fill mode")),
    }
}

fn field_error(field: &'static str, message: &str) -> ActionError {
    ActionError::FieldType {
        name: "animate",
        field,
        message: message.to_owned(),
    }
}
