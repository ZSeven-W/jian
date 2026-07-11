use super::base::PenNodeBase;
use crate::sizing::SizingBehavior;
use crate::style::{PenEffect, PenFill, PenStroke};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub struct PenPathHandle {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum PenPathPointType {
    Corner,
    Mirrored,
    Independent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "lowercase")]
pub enum PathFillRule {
    Nonzero,
    Evenodd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PenPathAnchor {
    pub x: f64,
    pub y: f64,
    pub handle_in: Option<PenPathHandle>,
    pub handle_out: Option<PenPathHandle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point_type: Option<PenPathPointType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PathNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchors: Option<Vec<PenPathAnchor>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "fillRule")]
    pub fill_rule: Option<PathFillRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<SizingBehavior>,
    #[serde(flatten)]
    pub limits: crate::sizing::SizeLimits,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Vec<PenFill>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<PenStroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<PenEffect>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<crate::state::StateSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<crate::events::Bindings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<crate::events::EventHandlers>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<crate::lifecycle::NodeLifecycleHooks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantics: Option<crate::semantics::SemanticsMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gestures: Option<crate::gestures::GestureOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<crate::navigation::NavigationRoute>,
}

#[cfg(test)]
mod tests {
    use super::PathNode;

    #[test]
    fn fill_rule_round_trips_and_stays_absent_by_default() {
        let with_rule = r#"{"id":"ring","fillRule":"evenodd"}"#;
        let path: PathNode = serde_json::from_str(with_rule).expect("parse even-odd path");
        assert_eq!(serde_json::to_string(&path).unwrap(), with_rule);

        let absent = r#"{"id":"plain"}"#;
        let path: PathNode = serde_json::from_str(absent).expect("parse path without fill rule");
        assert_eq!(serde_json::to_string(&path).unwrap(), absent);
    }
}
