use crate::events::ActionList;
use crate::expression::Expression;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticRole {
    Button,
    Link,
    Image,
    Text,
    Heading,
    Input,
    List,
    ListItem,
    Header,
    Nav,
    Main,
    Dialog,
    Alert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LiveRegion {
    Off,
    Polite,
    Assertive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SemanticAction {
    pub name: String,
    pub label: String,
    pub handler: ActionList,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SemanticsMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<SemanticRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_region: Option<LiveRegion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<Expression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<SemanticAction>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_role() {
        let json = r#"{"role":"button","label":"Submit form"}"#;
        let s: SemanticsMeta = serde_json::from_str(json).unwrap();
        assert!(matches!(s.role, Some(SemanticRole::Button)));
        assert_eq!(s.label.as_deref(), Some("Submit form"));
    }

    #[test]
    fn semantic_with_actions() {
        let json = r#"{
          "role":"dialog",
          "actions":[{"name":"dismiss","label":"Close","handler":[{"pop":null}]}]
        }"#;
        let s: SemanticsMeta = serde_json::from_str(json).unwrap();
        let acts = s.actions.unwrap();
        assert_eq!(acts[0].name, "dismiss");
    }
}
