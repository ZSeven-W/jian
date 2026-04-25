use crate::events::ActionList;
use crate::expression::Expression;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
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
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum LiveRegion {
    Off,
    Polite,
    Assertive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct SemanticAction {
    pub name: String,
    pub label: String,
    pub handler: ActionList,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct SemanticsMeta {
    // ── A11y (v1.0) ────────────────────────────────────────────────
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

    // ── AI Action Surface (v1.0 additive — 2026-04-24) ─────────────
    /// Author-stable override for the auto-derived AI action name.
    /// When set, the resulting action name is `<scope>.<aiName>`
    /// without the auto `_<hash4>` suffix and survives slug recomputes
    /// across builds. See `2026-04-24-ai-action-surface.md` §3.3-3.4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_name: Option<String>,

    /// Tool description shown to external AI agents. Overrides the
    /// auto-generated default; lets authors steer what a model "sees"
    /// without changing visible UI text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_description: Option<String>,

    /// `true` permanently hides the node's derived action from the AI
    /// surface (StaticHidden). Defaults to `false`. ConfirmGated /
    /// StateGated availability are decided dynamically and do **not**
    /// require this flag — see ai-action-surface.md §4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_hidden: Option<bool>,

    /// Historical `aiName` values still accepted by `execute_action`
    /// for transparent migration after a rename. Aliases are honoured
    /// at execute time (with `audit reason_code: "alias_used"`) but
    /// not surfaced by `list_available_actions`. See §9.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_aliases: Option<Vec<String>>,
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

    #[test]
    fn ai_action_surface_fields_round_trip() {
        let json = r#"{
          "aiName":"submit_form",
          "aiDescription":"Submit the registration form",
          "aiHidden":false,
          "aiAliases":["sign_up","register"]
        }"#;
        let s: SemanticsMeta = serde_json::from_str(json).unwrap();
        assert_eq!(s.ai_name.as_deref(), Some("submit_form"));
        assert_eq!(
            s.ai_description.as_deref(),
            Some("Submit the registration form")
        );
        assert_eq!(s.ai_hidden, Some(false));
        assert_eq!(
            s.ai_aliases.as_deref(),
            Some(&["sign_up".to_owned(), "register".to_owned()][..])
        );
        let ser = serde_json::to_string(&s).unwrap();
        assert!(ser.contains("\"aiName\""));
        assert!(ser.contains("\"aiAliases\""));
    }

    #[test]
    fn ai_fields_are_optional() {
        // Old `.op` files with bare A11y semantics keep deserialising —
        // additive contract preserved.
        let json = r#"{"role":"button","label":"OK"}"#;
        let s: SemanticsMeta = serde_json::from_str(json).unwrap();
        assert!(s.ai_name.is_none());
        assert!(s.ai_description.is_none());
        assert!(s.ai_hidden.is_none());
        assert!(s.ai_aliases.is_none());
    }
}
