use crate::expression::Expression;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScrollBehavior {
    Auto,
    Contain,
    None,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GestureOverrides {
    /// When true, this node and its subtree bypass the Arena and receive raw pointer events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_pointer: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<Expression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_behavior: Option<ScrollBehavior>,
    /// Override drag threshold in logical pixels (default 8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drag_threshold: Option<f64>,
    /// Override long-press duration in ms (default 500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_press_duration: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_pointer_override() {
        let g: GestureOverrides = serde_json::from_str(r#"{"rawPointer":true}"#).unwrap();
        assert_eq!(g.raw_pointer, Some(true));
    }

    #[test]
    fn scroll_and_thresholds() {
        let json = r#"{"scrollBehavior":"contain","dragThreshold":16,"longPressDuration":300}"#;
        let g: GestureOverrides = serde_json::from_str(json).unwrap();
        assert!(matches!(g.scroll_behavior, Some(ScrollBehavior::Contain)));
        assert_eq!(g.drag_threshold, Some(16.0));
        assert_eq!(g.long_press_duration, Some(300));
    }
}
