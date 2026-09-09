//! Document-level motion declarations shared by every node kind.

use crate::events::ExtraJson;
use crate::node::PenNode;
use crate::PenDocument;
use serde::{Deserialize, Serialize};
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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

impl Default for Easing {
    fn default() -> Self {
        Self::Standard
    }
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Keyframe {
    pub offset: f32,
    pub values: BTreeMap<String, Value>,
    #[serde(default, flatten)]
    pub extra: ExtraJson,
}

/// The two P1 lifecycle triggers.
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
}

/// Fill policy for a node declaration. P1 intentionally keeps only the CSS
/// policies needed by mount/inView recipes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub enum NodeAnimationFillMode {
    #[serde(rename = "forwards")]
    #[cfg_attr(feature = "export-ts", ts(rename = "forwards"))]
    Forwards,
    #[serde(rename = "none")]
    #[cfg_attr(feature = "export-ts", ts(rename = "none"))]
    None,
}

impl Default for NodeAnimationFillMode {
    fn default() -> Self {
        Self::Forwards
    }
}

/// One time-based node animation declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct NodeAnimation {
    pub trigger: MotionTrigger,
    pub keyframes: Vec<Keyframe>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub delay_ms: u64,
    #[serde(default)]
    pub easing: Easing,
    #[serde(default = "default_iterations", skip_serializing_if = "is_one")]
    pub iterations: u32,
    #[serde(default, skip_serializing_if = "is_forwards")]
    pub fill_mode: NodeAnimationFillMode,
    #[serde(default = "default_once", skip_serializing_if = "is_true")]
    pub once: bool,
    #[serde(default, flatten)]
    pub extra: ExtraJson,
}

fn default_iterations() -> u32 {
    1
}

fn default_once() -> bool {
    true
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_one(value: &u32) -> bool {
    *value == 1
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_forwards(value: &NodeAnimationFillMode) -> bool {
    *value == NodeAnimationFillMode::Forwards
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
            for (animation_index, animation) in animations.iter().enumerate() {
                validate_keyframes(
                    &animation.keyframes,
                    &format!("{node_path}.animations[{animation_index}].keyframes"),
                )?;
            }
        }
        if let Some(children) = node.children_ref() {
            validate_nodes(children, &format!("{node_path}.children"))?;
        }
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
