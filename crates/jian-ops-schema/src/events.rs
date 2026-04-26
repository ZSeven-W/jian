use crate::expression::Expression;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A single Action is a 1-key object: `{ "<action_name>": <body> }`.
///
/// Examples (all are valid JSON `Action`s):
/// - `{ "set": { "$state.count": "$state.count + 1" } }`
/// - `{ "fetch": { "url": "/api/x", "into": "$state.u" } }`
/// - `{ "push": "/detail/42" }`
///
/// The body shape per action is NOT validated here — see `jian-core::action`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(transparent)]
pub struct Action(pub BTreeMap<String, serde_json::Value>);

impl Action {
    /// Returns (action_name, body) if this Action has exactly one key.
    pub fn single(&self) -> Option<(&str, &serde_json::Value)> {
        let mut iter = self.0.iter();
        let first = iter.next()?;
        if iter.next().is_some() {
            return None;
        }
        Some((first.0.as_str(), first.1))
    }
}

pub type ActionList = Vec<Action>;

/// All supported event hook keys. Note: input events (`onChange`, `onSubmit`, `onFocus`,
/// `onBlur`) apply only to input-kind nodes. `on_key` is keyboard, `on_reach_end`
/// is list-scroll-end, etc.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct EventHandlers {
    // Gesture-originated
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_tap: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_double_tap: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_long_press: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_pan_start: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_pan_update: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_pan_end: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_scale_start: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_scale_update: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_scale_end: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_rotate_start: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_rotate_update: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_rotate_end: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_hover_enter: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_hover_leave: Option<ActionList>,

    // Input-node events
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_change: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_submit: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_focus: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_blur: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_key: Option<ActionList>,

    // Scroll / list
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_scroll: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_reach_end: Option<ActionList>,
}

/// `bindings` is a map from property-name (with optional `bind:` prefix for two-way)
/// to a Tier-1 expression. E.g. `{ "content": "`Count: ${$state.count}`" }` or
/// `{ "bind:value": "$state.email" }`.
pub type Bindings = BTreeMap<String, Expression>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_set_action() {
        let json = r#"{"set":{"$state.count":"$state.count + 1"}}"#;
        let a: Action = serde_json::from_str(json).unwrap();
        let (name, body) = a.single().unwrap();
        assert_eq!(name, "set");
        assert!(body.is_object());
    }

    #[test]
    fn push_action_with_string_body() {
        let json = r#"{"push":"/detail/42"}"#;
        let a: Action = serde_json::from_str(json).unwrap();
        let (name, body) = a.single().unwrap();
        assert_eq!(name, "push");
        assert_eq!(body.as_str(), Some("/detail/42"));
    }

    #[test]
    fn event_handlers_partial() {
        let json = r#"{
          "onTap": [{"set":{"$state.count":"$state.count+1"}}],
          "onLongPress": [{"open_menu":"context"}]
        }"#;
        let e: EventHandlers = serde_json::from_str(json).unwrap();
        assert_eq!(e.on_tap.unwrap().len(), 1);
        assert_eq!(e.on_long_press.unwrap().len(), 1);
    }

    #[test]
    fn bindings_with_two_way() {
        let json = r#"{
          "content": "\"Count: \" + $state.count",
          "bind:value": "$state.email"
        }"#;
        let b: Bindings = serde_json::from_str(json).unwrap();
        assert_eq!(b.len(), 2);
        assert!(b.contains_key("bind:value"));
    }
}
