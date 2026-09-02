//! Platform-neutral delivery for structured animation requests.

use crate::action::animation_registry::{animatable_property_registry, AnimationApply};
use crate::binding::{BindingTarget, InvalidationKind};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnimationProperty {
    Opacity,
    X,
    Y,
    TranslateX,
    TranslateY,
    Rotation,
    ScaleX,
    ScaleY,
    Fill,
    Stroke,
    CornerRadius,
    Width,
    Height,
    ShaderUniform(String),
    Custom(String),
}

impl AnimationProperty {
    pub fn from_registered(name: &str, apply: AnimationApply) -> Self {
        match name {
            "opacity" => Self::Opacity,
            "x" => Self::X,
            "y" => Self::Y,
            "translateX" => Self::TranslateX,
            "translateY" => Self::TranslateY,
            "rotation" => Self::Rotation,
            "scaleX" => Self::ScaleX,
            "scaleY" => Self::ScaleY,
            "fill" => Self::Fill,
            "stroke" => Self::Stroke,
            "cornerRadius" => Self::CornerRadius,
            "width" => Self::Width,
            "height" => Self::Height,
            _ if apply == AnimationApply::ShaderUniform => Self::ShaderUniform(name.to_owned()),
            _ => Self::Custom(name.to_owned()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Opacity => "opacity",
            Self::X => "x",
            Self::Y => "y",
            Self::TranslateX => "translateX",
            Self::TranslateY => "translateY",
            Self::Rotation => "rotation",
            Self::ScaleX => "scaleX",
            Self::ScaleY => "scaleY",
            Self::Fill => "fill",
            Self::Stroke => "stroke",
            Self::CornerRadius => "cornerRadius",
            Self::Width => "width",
            Self::Height => "height",
            Self::ShaderUniform(name) | Self::Custom(name) => name,
        }
    }

    pub fn binding_target(&self) -> Option<BindingTarget> {
        animatable_property_registry()
            .get(self.name())
            .and_then(|entry| match entry.apply {
                AnimationApply::Binding(target) => Some(target),
                AnimationApply::ShaderUniform => None,
            })
    }

    pub fn invalidation(&self) -> Option<InvalidationKind> {
        animatable_property_registry()
            .get(self.name())
            .map(|entry| entry.invalidation_class)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Easing {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationDirection {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationFillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimationRequest {
    pub target: String,
    pub property: AnimationProperty,
    pub from: Option<Value>,
    pub to: Value,
    pub duration_ms: u64,
    pub delay_ms: u64,
    pub easing: Easing,
    pub iterations: u32,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub requested_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnimationOutcome {
    Accepted,
    Unsupported,
    Rejected(String),
}

pub trait AnimationSink {
    fn request(&self, request: &AnimationRequest) -> AnimationOutcome;
}

pub struct NullAnimationSink;

impl AnimationSink for NullAnimationSink {
    fn request(&self, _request: &AnimationRequest) -> AnimationOutcome {
        AnimationOutcome::Unsupported
    }
}
