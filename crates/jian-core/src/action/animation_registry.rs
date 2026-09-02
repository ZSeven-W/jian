//! Canonical registry of properties the structured animate action may target.

use crate::binding::{BindingTarget, InvalidationKind};
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

pub const SHADER_UNIFORM_PREFIX: &str = "shader.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationValueType {
    Number,
    Color,
    Length,
    Angle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationInterpolate {
    Linear,
    Discrete,
    ColorSrgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationApply {
    Binding(BindingTarget),
    ShaderUniform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimatableProperty {
    pub name: String,
    pub value_type: AnimationValueType,
    pub interpolate: AnimationInterpolate,
    pub invalidation_class: InvalidationKind,
    pub apply: AnimationApply,
    pub capability: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnimationRegistryError {
    #[error("animatable property name is empty")]
    EmptyName,
    #[error("animatable property '{name}' is already registered")]
    Duplicate { name: String },
    #[error("continuous Relayout animation is forbidden for '{name}'")]
    ContinuousRelayout { name: String },
    #[error("invalid invalidation class for animatable property '{name}'")]
    InvalidInvalidation { name: String },
    #[error("shader uniform property '{name}' must use ShaderUniform apply")]
    InvalidShaderApply { name: String },
}

#[derive(Debug, Default)]
pub struct AnimatablePropertyRegistry {
    entries: RwLock<BTreeMap<String, AnimatableProperty>>,
}

impl Clone for AnimatablePropertyRegistry {
    fn clone(&self) -> Self {
        Self {
            entries: RwLock::new(self.read_entries().clone()),
        }
    }
}

impl AnimatablePropertyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtins() -> Self {
        let registry = Self::new();
        for property in builtin_properties() {
            registry
                .register(property)
                .expect("builtin animation property contract is valid");
        }
        registry
    }

    pub fn register(&self, property: AnimatableProperty) -> Result<(), AnimationRegistryError> {
        if property.name.is_empty() {
            return Err(AnimationRegistryError::EmptyName);
        }
        let mut entries = self.write_entries();
        if entries.contains_key(&property.name) {
            return Err(AnimationRegistryError::Duplicate {
                name: property.name,
            });
        }
        if matches!(
            property.invalidation_class,
            InvalidationKind::None | InvalidationKind::Navigation
        ) {
            return Err(AnimationRegistryError::InvalidInvalidation {
                name: property.name,
            });
        }
        if property.invalidation_class == InvalidationKind::Relayout
            && property.interpolate != AnimationInterpolate::Discrete
        {
            return Err(AnimationRegistryError::ContinuousRelayout {
                name: property.name,
            });
        }
        if property.name.starts_with(SHADER_UNIFORM_PREFIX)
            && property.apply != AnimationApply::ShaderUniform
        {
            return Err(AnimationRegistryError::InvalidShaderApply {
                name: property.name,
            });
        }
        entries.insert(property.name.clone(), property);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<AnimatableProperty> {
        self.read_entries().get(name).cloned()
    }

    pub fn entries(&self) -> impl Iterator<Item = AnimatableProperty> {
        self.read_entries()
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn is_reserved_shader_uniform(&self, name: &str) -> bool {
        name.strip_prefix(SHADER_UNIFORM_PREFIX)
            .is_some_and(|uniform| !uniform.is_empty())
    }

    fn read_entries(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, AnimatableProperty>> {
        self.entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write_entries(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, BTreeMap<String, AnimatableProperty>> {
        self.entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub fn animatable_property_registry() -> &'static AnimatablePropertyRegistry {
    static REGISTRY: OnceLock<AnimatablePropertyRegistry> = OnceLock::new();
    REGISTRY.get_or_init(AnimatablePropertyRegistry::with_builtins)
}

fn builtin_properties() -> Vec<AnimatableProperty> {
    use AnimationInterpolate::{ColorSrgb, Discrete, Linear};
    use AnimationValueType::{Angle, Color, Length, Number};
    use BindingTarget::{
        CornerRadius, Fill, Height, Opacity, Rotation, ScaleX, ScaleY, Stroke, TranslateX,
        TranslateY, Width, X, Y,
    };
    [
        (
            "opacity",
            Number,
            Linear,
            InvalidationKind::PaintOnly,
            Opacity,
        ),
        ("x", Length, Linear, InvalidationKind::HitTest, X),
        ("y", Length, Linear, InvalidationKind::HitTest, Y),
        (
            "translateX",
            Length,
            Linear,
            InvalidationKind::PaintOnly,
            TranslateX,
        ),
        (
            "translateY",
            Length,
            Linear,
            InvalidationKind::PaintOnly,
            TranslateY,
        ),
        (
            "rotation",
            Angle,
            Linear,
            InvalidationKind::HitTest,
            Rotation,
        ),
        ("scaleX", Number, Linear, InvalidationKind::HitTest, ScaleX),
        ("scaleY", Number, Linear, InvalidationKind::HitTest, ScaleY),
        ("fill", Color, ColorSrgb, InvalidationKind::PaintOnly, Fill),
        (
            "stroke",
            Color,
            ColorSrgb,
            InvalidationKind::PaintOnly,
            Stroke,
        ),
        (
            "cornerRadius",
            Length,
            Linear,
            InvalidationKind::PaintOnly,
            CornerRadius,
        ),
        ("width", Length, Discrete, InvalidationKind::Relayout, Width),
        (
            "height",
            Length,
            Discrete,
            InvalidationKind::Relayout,
            Height,
        ),
    ]
    .into_iter()
    .map(
        |(name, value_type, interpolate, invalidation_class, target)| AnimatableProperty {
            name: name.to_owned(),
            value_type,
            interpolate,
            invalidation_class,
            apply: AnimationApply::Binding(target),
            capability: None,
        },
    )
    .collect()
}
