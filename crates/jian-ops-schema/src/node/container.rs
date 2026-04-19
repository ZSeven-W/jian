use crate::sizing::SizingBehavior;
use crate::style::{PenEffect, PenFill, PenStroke};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    None,
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum JustifyContent {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum AlignItems {
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum Padding {
    Uniform(f64),
    XY([f64; 2]),
    LtrB([f64; 4]),
    Expression(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum CornerRadius {
    Uniform(f64),
    PerCorner([f64; 4]),
}

/// Container props — shared by Frame/Group/Rectangle.
/// `children` is NOT included here because PenNode children are recursively
/// defined in node/mod.rs to avoid circular module dependency. Each concrete
/// node type that has children declares it explicitly via `children: Option<Vec<PenNode>>`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ContainerProps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<crate::node::base::NumberOrExpression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<Padding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justify_content: Option<JustifyContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align_items: Option<AlignItems>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_content: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<CornerRadius>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Vec<PenFill>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<PenStroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<PenEffect>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_container_roundtrip() {
        let json = "{}";
        let c: ContainerProps = serde_json::from_str(json).unwrap();
        assert_eq!(c, ContainerProps::default());
        assert_eq!(serde_json::to_string(&c).unwrap(), json);
    }

    #[test]
    fn padding_uniform() {
        let j = r#"{"padding":8}"#;
        let c: ContainerProps = serde_json::from_str(j).unwrap();
        assert!(matches!(c.padding, Some(Padding::Uniform(8.0))));
    }

    #[test]
    fn padding_per_side() {
        let j = r#"{"padding":[1,2,3,4]}"#;
        let c: ContainerProps = serde_json::from_str(j).unwrap();
        assert!(matches!(
            c.padding,
            Some(Padding::LtrB([1.0, 2.0, 3.0, 4.0]))
        ));
    }

    #[test]
    fn layout_horizontal() {
        let j = r#"{"layout":"horizontal","gap":12,"padding":[0,0],"justifyContent":"space_between","alignItems":"center"}"#;
        let c: ContainerProps = serde_json::from_str(j).unwrap();
        assert!(matches!(c.layout, Some(LayoutMode::Horizontal)));
        assert!(matches!(
            c.justify_content,
            Some(JustifyContent::SpaceBetween)
        ));
        assert!(matches!(c.align_items, Some(AlignItems::Center)));
    }
}
