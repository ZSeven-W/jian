use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Opacity can be a number or a `$variable` reference string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum NumberOrExpression {
    Number(f64),
    Expression(String),
}

/// Boolean that may also be an expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum BoolOrExpression {
    Bool(bool),
    Expression(String),
}

/// Shared fields across all node types.
/// Note: concrete nodes use `#[serde(flatten)]` to embed `PenNodeBase`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PenNodeBase {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<NumberOrExpression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<BoolOrExpression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_x: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_y: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<BTreeMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_base_roundtrip() {
        let json = r#"{"id":"node-1"}"#;
        let b: PenNodeBase = serde_json::from_str(json).unwrap();
        assert_eq!(b.id, "node-1");
        assert_eq!(serde_json::to_string(&b).unwrap(), json);
    }

    #[test]
    fn opacity_as_number() {
        let j = r#"{"id":"n","opacity":0.5}"#;
        let b: PenNodeBase = serde_json::from_str(j).unwrap();
        match b.opacity {
            Some(NumberOrExpression::Number(x)) => assert_eq!(x, 0.5),
            _ => panic!(),
        }
    }

    #[test]
    fn opacity_as_expression() {
        let j = r#"{"id":"n","opacity":"$alpha-muted"}"#;
        let b: PenNodeBase = serde_json::from_str(j).unwrap();
        match b.opacity {
            Some(NumberOrExpression::Expression(ref s)) => assert_eq!(s, "$alpha-muted"),
            _ => panic!(),
        }
    }
}
