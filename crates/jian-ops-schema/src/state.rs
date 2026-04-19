use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveType {
    Int,
    Float,
    Number,
    String,
    Bool,
    Array,
    Object,
    Date,
}

/// Recursive state type description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum StateType {
    Primitive(PrimitiveType),
    OneOf {
        #[serde(rename = "oneOf")]
        options: Vec<StateType>,
    },
    Array {
        array: Box<StateType>,
    },
    Object {
        object: BTreeMap<String, StateType>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct StateEntry {
    #[serde(rename = "type")]
    pub kind: StateType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persist: Option<bool>,
}

pub type StateSchema = BTreeMap<String, StateEntry>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_int() {
        let json = r#"{"count":{"type":"int","default":0}}"#;
        let s: StateSchema = serde_json::from_str(json).unwrap();
        let entry = s.get("count").unwrap();
        assert!(matches!(
            entry.kind,
            StateType::Primitive(PrimitiveType::Int)
        ));
        assert_eq!(entry.default, Some(serde_json::json!(0)));
    }

    #[test]
    fn persist_flag() {
        let json = r#"{"token":{"type":"string","default":"","persist":true}}"#;
        let s: StateSchema = serde_json::from_str(json).unwrap();
        assert_eq!(s.get("token").unwrap().persist, Some(true));
    }

    #[test]
    fn oneof_type() {
        let json = r#"{"val":{"type":{"oneOf":["int","string"]},"default":null}}"#;
        let s: StateSchema = serde_json::from_str(json).unwrap();
        match &s.get("val").unwrap().kind {
            StateType::OneOf { options } => assert_eq!(options.len(), 2),
            _ => panic!(),
        }
    }

    #[test]
    fn typed_array() {
        let json = r#"{"items":{"type":{"array":"string"},"default":[]}}"#;
        let s: StateSchema = serde_json::from_str(json).unwrap();
        match &s.get("items").unwrap().kind {
            StateType::Array { array } => {
                assert!(matches!(
                    **array,
                    StateType::Primitive(PrimitiveType::String)
                ))
            }
            _ => panic!(),
        }
    }

    #[test]
    fn nested_object_type() {
        let json = r#"{
          "user": {
            "type": {"object": {"id":"string","active":"bool"}},
            "default": null
          }
        }"#;
        let s: StateSchema = serde_json::from_str(json).unwrap();
        match &s.get("user").unwrap().kind {
            StateType::Object { object } => {
                assert_eq!(object.len(), 2);
                assert!(matches!(
                    object.get("id").unwrap(),
                    StateType::Primitive(PrimitiveType::String)
                ));
            }
            _ => panic!(),
        }
    }
}
