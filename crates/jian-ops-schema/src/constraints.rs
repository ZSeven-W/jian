use serde::{Deserialize, Serialize};

/// Horizontal anchoring for an absolutely positioned node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum HConstraint {
    Left,
    Right,
    Center,
    LeftRight,
    Scale,
}

/// Vertical anchoring for an absolutely positioned node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum VConstraint {
    Top,
    Bottom,
    Center,
    TopBottom,
    Scale,
}

/// Figma-style per-axis anchoring for absolutely positioned nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub struct Constraints {
    pub h: HConstraint,
    pub v: VConstraint,
}

impl Default for Constraints {
    fn default() -> Self {
        Self {
            h: HConstraint::Left,
            v: VConstraint::Top,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_snake_case_and_defaults() {
        let constraints: Constraints =
            serde_json::from_str(r#"{"h":"left_right","v":"scale"}"#).unwrap();
        assert_eq!(constraints.h, HConstraint::LeftRight);
        assert_eq!(constraints.v, VConstraint::Scale);
        assert_eq!(
            Constraints::default(),
            Constraints {
                h: HConstraint::Left,
                v: VConstraint::Top,
            }
        );
    }

    #[test]
    fn base_without_constraints_serializes_identically() {
        use crate::node::base::PenNodeBase;

        let base: PenNodeBase = serde_json::from_str(r#"{"id":"n1"}"#).unwrap();
        assert!(base.constraints.is_none());
        let value = serde_json::to_value(&base).unwrap();
        assert!(value.get("constraints").is_none());
    }
}
