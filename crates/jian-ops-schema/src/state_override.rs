//! Per-interaction-state style overrides for widget nodes.
//!
//! Spec 2026-06-13 §5: interaction visuals are auto-derived from the
//! node's authored style by default; authors may override individual
//! fields per state. `None` = keep the derived value.

use crate::style::{PenEffect, PenFill, PenStroke};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct StyleOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Vec<PenFill>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<PenStroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<PenEffect>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
}

/// Authored overrides for the four auto-derived interaction states.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct WidgetStates {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover: Option<StyleOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressed: Option<StyleOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<StyleOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<StyleOverride>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_states_serialize_to_empty_object() {
        let s = WidgetStates::default();
        assert_eq!(serde_json::to_string(&s).unwrap(), "{}");
    }

    #[test]
    fn partial_override_round_trips() {
        let json = r#"{"hover":{"opacity":0.9}}"#;
        let s: WidgetStates = serde_json::from_str(json).unwrap();
        assert_eq!(s.hover.as_ref().unwrap().opacity, Some(0.9));
        assert!(s.pressed.is_none());
        assert_eq!(serde_json::to_string(&s).unwrap(), json);
    }
}
