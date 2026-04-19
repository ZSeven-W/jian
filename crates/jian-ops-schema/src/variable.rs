use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum VariableKind {
    Color,
    Number,
    Boolean,
    String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum VariableScalar {
    Bool(bool),
    Num(f64),
    Str(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
pub struct ThemedValue {
    pub value: VariableScalar,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum VariableValue {
    Scalar(VariableScalar),
    Themed(Vec<ThemedValue>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct VariableDefinition {
    #[serde(rename = "type")]
    pub kind: VariableKind,
    pub value: VariableValue,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_variable_roundtrip() {
        let json = r##"{"type":"color","value":"#ff0000"}"##;
        let v: VariableDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(v.kind, VariableKind::Color);
        match v.value {
            VariableValue::Scalar(VariableScalar::Str(ref s)) => assert_eq!(s, "#ff0000"),
            _ => panic!(),
        }
    }

    #[test]
    fn themed_variable_roundtrip() {
        let json = r##"{"type":"color","value":[{"value":"#000000","theme":{"mode":"light"}},{"value":"#ffffff","theme":{"mode":"dark"}}]}"##;
        let v: VariableDefinition = serde_json::from_str(json).unwrap();
        match v.value {
            VariableValue::Themed(ref arr) => assert_eq!(arr.len(), 2),
            _ => panic!(),
        }
        let s = serde_json::to_string(&v).unwrap();
        let v2: VariableDefinition = serde_json::from_str(&s).unwrap();
        assert_eq!(v, v2);
    }
}
