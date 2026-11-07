//! Document-level motion declarations shared by every node kind.

use crate::events::ExtraJson;
use crate::node::PenNode;
use crate::PenDocument;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Properties known to the structured animation registry. Layout properties
/// remain known to the action layer but are rejected by the P1 runtime/lint.
pub const ANIMATION_REGISTRY_PROPERTIES: &[&str] = &[
    "opacity",
    "x",
    "y",
    "translateX",
    "translateY",
    "rotation",
    "scaleX",
    "scaleY",
    "fill",
    "stroke",
    "cornerRadius",
    "width",
    "height",
];

/// P1 properties that are safe to animate without relayout.
pub const P1_MOTION_PROPERTIES: &[&str] = &[
    "opacity",
    "translateX",
    "translateY",
    "scaleX",
    "scaleY",
    "rotation",
    "fill",
    "stroke",
    "cornerRadius",
];

/// CSS/Material-style easing names accepted by document motion declarations.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub enum Easing {
    #[serde(rename = "linear")]
    #[cfg_attr(feature = "export-ts", ts(rename = "linear"))]
    Linear,
    #[serde(rename = "ease")]
    #[cfg_attr(feature = "export-ts", ts(rename = "ease"))]
    Ease,
    #[serde(rename = "ease_in", alias = "easeIn", alias = "ease-in")]
    #[cfg_attr(feature = "export-ts", ts(rename = "easeIn"))]
    EaseIn,
    #[serde(rename = "ease_out", alias = "easeOut", alias = "ease-out")]
    #[cfg_attr(feature = "export-ts", ts(rename = "easeOut"))]
    EaseOut,
    #[serde(rename = "ease_in_out", alias = "easeInOut", alias = "ease-in-out")]
    #[cfg_attr(feature = "export-ts", ts(rename = "easeInOut"))]
    EaseInOut,
    #[default]
    #[serde(rename = "standard")]
    #[cfg_attr(feature = "export-ts", ts(rename = "standard"))]
    Standard,
    #[serde(rename = "emphasized")]
    #[cfg_attr(feature = "export-ts", ts(rename = "emphasized"))]
    Emphasized,
    #[serde(rename = "emphasizedDecelerate", alias = "emphasized_decelerate")]
    #[cfg_attr(feature = "export-ts", ts(rename = "emphasizedDecelerate"))]
    EmphasizedDecelerate,
    #[serde(rename = "emphasizedAccelerate", alias = "emphasized_accelerate")]
    #[cfg_attr(feature = "export-ts", ts(rename = "emphasizedAccelerate"))]
    EmphasizedAccelerate,
    #[serde(rename = "cubicBezier")]
    #[cfg_attr(feature = "export-ts", ts(rename = "cubicBezier"))]
    CubicBezier(f32, f32, f32, f32),
}

/// The document's authored motion preference. A host may reduce further.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "lowercase")]
pub enum MotionPreference {
    Full,
    Reduced,
}

impl MotionPreference {
    pub fn more_reduced(self, other: Self) -> Self {
        if self == Self::Reduced || other == Self::Reduced {
            Self::Reduced
        } else {
            Self::Full
        }
    }
}

/// A node-level transition applied to runtime paint changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    #[serde(default = "default_transition_duration")]
    pub duration_ms: u64,
    #[serde(default)]
    pub easing: Easing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<String>>,
    #[serde(default, flatten)]
    pub extra: ExtraJson,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            duration_ms: 200,
            easing: Easing::Standard,
            properties: None,
            extra: ExtraJson::default(),
        }
    }
}

fn default_transition_duration() -> u64 {
    200
}

/// A keyframe stop map. Values use the same JSON value representation as the
/// action layer so colors and numbers keep their existing wire shape.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Keyframe {
    pub offset: f32,
    pub values: BTreeMap<String, Value>,
    #[serde(default, flatten)]
    pub extra: ExtraJson,
}

impl<'de> Deserialize<'de> for Keyframe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        keyframe_from_value(value).map_err(D::Error::custom)
    }
}

/// What starts a node animation: the screen mounting, the node scrolling into
/// view, or an interaction (`play_motion`) naming the animation's `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub enum MotionTrigger {
    #[serde(rename = "mount")]
    #[cfg_attr(feature = "export-ts", ts(rename = "mount"))]
    Mount,
    #[serde(rename = "inView")]
    #[cfg_attr(feature = "export-ts", ts(rename = "inView"))]
    InView,
    #[serde(rename = "manual")]
    #[cfg_attr(feature = "export-ts", ts(rename = "manual"))]
    Manual,
}

/// Fill policy for a node declaration. P1 intentionally keeps only the CSS
/// policies needed by mount/inView recipes.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub enum NodeAnimationFillMode {
    #[default]
    #[serde(rename = "forwards")]
    #[cfg_attr(feature = "export-ts", ts(rename = "forwards"))]
    Forwards,
    #[serde(rename = "none")]
    #[cfg_attr(feature = "export-ts", ts(rename = "none"))]
    None,
}

/// One time-based node animation declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct NodeAnimation {
    pub trigger: MotionTrigger,
    /// Names the animation for `play_motion`. Required for `manual`, unique
    /// within one node, and shared across nodes to form one sequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(deserialize_with = "deserialize_keyframes")]
    pub keyframes: Vec<Keyframe>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub delay_ms: u64,
    #[serde(default)]
    pub easing: Easing,
    #[serde(default, skip_serializing_if = "Iterations::is_default")]
    #[cfg_attr(feature = "export-ts", ts(type = "number | \"infinite\""))]
    pub iterations: Iterations,
    #[serde(default, skip_serializing_if = "is_forwards")]
    pub fill_mode: NodeAnimationFillMode,
    #[serde(default = "default_once", skip_serializing_if = "is_true")]
    pub once: bool,
    #[serde(default, flatten)]
    pub extra: ExtraJson,
}

fn default_once() -> bool {
    true
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_forwards(value: &NodeAnimationFillMode) -> bool {
    *value == NodeAnimationFillMode::Forwards
}

/// How many times an animation plays. `"infinite"` repeats until the node
/// unmounts or an interaction stops it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Iterations {
    Count(u32),
    Infinite,
}

impl Default for Iterations {
    fn default() -> Self {
        Self::Count(1)
    }
}

impl Iterations {
    pub fn is_infinite(self) -> bool {
        self == Self::Infinite
    }

    /// The pass count, with `Infinite` mapped to `cap`.
    pub fn count_or(self, cap: u32) -> u32 {
        match self {
            Self::Count(count) => count,
            Self::Infinite => cap,
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Serialize for Iterations {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Count(count) => serializer.serialize_u32(*count),
            Self::Infinite => serializer.serialize_str(INFINITE_ITERATIONS),
        }
    }
}

const INFINITE_ITERATIONS: &str = "infinite";

impl<'de> Deserialize<'de> for Iterations {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IterationsVisitor;

        impl serde::de::Visitor<'_> for IterationsVisitor {
            type Value = Iterations;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a non-negative integer or \"infinite\"")
            }

            fn visit_u64<E: Error>(self, value: u64) -> Result<Iterations, E> {
                u32::try_from(value)
                    .map(Iterations::Count)
                    .map_err(|_| E::custom(format!("iterations {value} is out of range")))
            }

            fn visit_i64<E: Error>(self, value: i64) -> Result<Iterations, E> {
                u64::try_from(value)
                    .map_err(|_| E::custom(format!("iterations {value} must not be negative")))
                    .and_then(|value| self.visit_u64(value))
            }

            fn visit_str<E: Error>(self, value: &str) -> Result<Iterations, E> {
                if value == INFINITE_ITERATIONS {
                    Ok(Iterations::Infinite)
                } else {
                    Err(E::custom(format!(
                        "iterations must be a number or \"infinite\", got {value:?}"
                    )))
                }
            }
        }

        deserializer.deserialize_any(IterationsVisitor)
    }
}

impl schemars::JsonSchema for Iterations {
    fn schema_name() -> String {
        "Iterations".to_owned()
    }

    fn json_schema(generator: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Schema, SchemaObject, SubschemaValidation};
        let count = generator.subschema_for::<u32>();
        let infinite = Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            enum_values: Some(vec![Value::String(INFINITE_ITERATIONS.to_owned())]),
            ..Default::default()
        });
        Schema::Object(SchemaObject {
            subschemas: Some(Box::new(SubschemaValidation {
                any_of: Some(vec![count, infinite]),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

/// Accept a keyframe array or the two-stop `{from, to}` shorthand.
fn deserialize_keyframes<'de, D>(deserializer: D) -> Result<Vec<Keyframe>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    keyframes_from_value(value).map_err(D::Error::custom)
}

fn keyframes_from_value(value: Value) -> Result<Vec<Keyframe>, String> {
    match value {
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (index, item) in items.into_iter().enumerate() {
                match keyframe_from_value(item) {
                    Ok(keyframe) => out.push(keyframe),
                    Err(error) => return Err(format!("{error} at keyframe {index}")),
                }
            }
            Ok(out)
        }
        Value::Object(map) => from_to_keyframes(map),
        _ => Err("keyframes must be an array or a {from, to} object".to_owned()),
    }
}

fn from_to_keyframes(mut map: serde_json::Map<String, Value>) -> Result<Vec<Keyframe>, String> {
    let from = map
        .remove("from")
        .ok_or_else(|| "keyframes object missing field `from`".to_owned())?;
    let to = map
        .remove("to")
        .ok_or_else(|| "keyframes object missing field `to`".to_owned())?;
    Ok(vec![
        Keyframe {
            offset: 0.0,
            values: property_map(from, "from")?,
            extra: ExtraJson::default(),
        },
        Keyframe {
            offset: 1.0,
            values: property_map(to, "to")?,
            extra: ExtraJson::default(),
        },
    ])
}

fn property_map(value: Value, field: &str) -> Result<BTreeMap<String, Value>, String> {
    let Value::Object(map) = value else {
        return Err(format!("`{field}` must be an object of property values"));
    };
    Ok(map.into_iter().collect())
}

/// Canonical `{offset, values}` or flat `{offset, prop: value, ...}`.
fn keyframe_from_value(value: Value) -> Result<Keyframe, String> {
    let Value::Object(mut map) = value else {
        return Err("keyframe must be an object".to_owned());
    };
    let offset = match map.remove("offset") {
        Some(offset) => serde_json::from_value::<f32>(offset)
            .map_err(|error| format!("invalid `offset`: {error}"))?,
        None => return Err("missing field `offset`".to_owned()),
    };
    if let Some(values) = map.remove("values") {
        let values =
            serde_json::from_value(values).map_err(|error| format!("invalid `values`: {error}"))?;
        Ok(Keyframe {
            offset,
            values,
            extra: ExtraJson(map.into_iter().collect()),
        })
    } else {
        Ok(Keyframe {
            offset,
            values: map.into_iter().collect(),
            extra: ExtraJson::default(),
        })
    }
}

/// Validate every motion declaration before a document is exposed to a host.
pub(crate) fn validate_document(document: &PenDocument) -> Result<(), (String, String)> {
    validate_nodes(&document.children, "$.children")?;
    if let Some(pages) = &document.pages {
        for (index, page) in pages.iter().enumerate() {
            validate_nodes(&page.children, &format!("$.pages[{index}].children"))?;
        }
    }
    Ok(())
}

fn validate_nodes(nodes: &[PenNode], path: &str) -> Result<(), (String, String)> {
    for (index, node) in nodes.iter().enumerate() {
        let node_path = format!("{path}[{index}]");
        if let Some(animations) = node.motion_declarations().1 {
            let mut ids = std::collections::BTreeSet::new();
            for (animation_index, animation) in animations.iter().enumerate() {
                let animation_path = format!("{node_path}.animations[{animation_index}]");
                validate_keyframes(&animation.keyframes, &format!("{animation_path}.keyframes"))?;
                validate_animation_id(animation, &animation_path, &mut ids)?;
            }
        }
        if let Some(children) = node.children_ref() {
            validate_nodes(children, &format!("{node_path}.children"))?;
        }
    }
    Ok(())
}

fn validate_animation_id(
    animation: &NodeAnimation,
    path: &str,
    ids: &mut std::collections::BTreeSet<String>,
) -> Result<(), (String, String)> {
    let Some(id) = animation.id.as_deref() else {
        if animation.trigger == MotionTrigger::Manual {
            return Err((
                format!("{path}.id"),
                "a manual animation needs an id for play_motion".to_owned(),
            ));
        }
        return Ok(());
    };
    let id = id.trim();
    if id.is_empty() {
        return Err((
            format!("{path}.id"),
            "animation id must not be blank".to_owned(),
        ));
    }
    if !ids.insert(id.to_owned()) {
        return Err((
            format!("{path}.id"),
            format!("animation id {id} is used twice on this node"),
        ));
    }
    Ok(())
}

fn validate_keyframes(keyframes: &[Keyframe], path: &str) -> Result<(), (String, String)> {
    let Some(first) = keyframes.first() else {
        return Err((
            path.to_owned(),
            "keyframes must include offsets 0 and 1".to_owned(),
        ));
    };
    if first.offset != 0.0 {
        return Err((
            path.to_owned(),
            "first keyframe offset must be 0".to_owned(),
        ));
    }
    let Some(last) = keyframes.last() else {
        unreachable!("first keyframe was present");
    };
    if last.offset != 1.0 {
        return Err((path.to_owned(), "last keyframe offset must be 1".to_owned()));
    }
    let mut previous = -1.0_f32;
    for (index, keyframe) in keyframes.iter().enumerate() {
        if !keyframe.offset.is_finite() || !(0.0..=1.0).contains(&keyframe.offset) {
            return Err((
                format!("{path}[{index}].offset"),
                "offset must be finite and between 0 and 1".to_owned(),
            ));
        }
        if keyframe.offset < previous {
            return Err((
                format!("{path}[{index}].offset"),
                "keyframe offsets must be sorted".to_owned(),
            ));
        }
        previous = keyframe.offset;
        for property in keyframe.values.keys() {
            if !ANIMATION_REGISTRY_PROPERTIES.contains(&property.as_str()) {
                return Err((
                    format!("{path}[{index}].values.{property}"),
                    format!("unknown animation property {property}"),
                ));
            }
        }
    }
    Ok(())
}
