use crate::events::{ActionList, ExtraJson};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct AppLifecycleHooks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_launch: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_resume: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_background: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_terminate: Option<ActionList>,
    /// Event hooks disabled for this lifecycle scope
    /// (`["onUnmount", ...]`). Order is preserved exactly as
    /// authored; the schema layer never dedups or rewrites it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_events: Option<Vec<String>>,
    /// Explicit hook evaluation order (`["onMount", "onUnmount", ...]`).
    /// Order is preserved exactly as authored; the schema layer
    /// never dedups or rewrites it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_order: Option<Vec<String>>,
    /// Future/unknown hook keys (e.g. `onFutureVisibility`).
    /// Preserved verbatim on round-trip; the runtime never
    /// executes unknown hooks — an older runtime passes them
    /// through untouched. Exported to TypeScript as
    /// `{ [key in string]?: JsonValue }` alongside the known fields.
    #[serde(default, flatten)]
    pub extra: ExtraJson,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PageLifecycleHooks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_enter: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_leave: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_foreground: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_background: Option<ActionList>,
    /// Event hooks disabled for this lifecycle scope
    /// (`["onTerminate", ...]`). Order is preserved exactly as
    /// authored; the schema layer never dedups or rewrites it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_events: Option<Vec<String>>,
    /// Explicit hook evaluation order. Order is preserved exactly
    /// as authored; the schema layer never dedups or rewrites it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_order: Option<Vec<String>>,
    /// Future/unknown hook keys. Preserved verbatim on round-trip;
    /// the runtime never executes unknown hooks — an older runtime
    /// passes them through untouched. Exported to TypeScript as
    /// `{ [key in string]?: JsonValue }` alongside the known fields.
    #[serde(default, flatten)]
    pub extra: ExtraJson,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct NodeLifecycleHooks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_mount: Option<ActionList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_unmount: Option<ActionList>,
    /// Event hooks disabled for this lifecycle scope
    /// (`["onUnmount", ...]`). Order is preserved exactly as
    /// authored; the schema layer never dedups or rewrites it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_events: Option<Vec<String>>,
    /// Explicit hook evaluation order (`["onMount", "onUnmount", ...]`).
    /// Order is preserved exactly as authored; the schema layer
    /// never dedups or rewrites it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_order: Option<Vec<String>>,
    /// Future/unknown hook keys (e.g. `onFutureVisibility`).
    /// Preserved verbatim on round-trip; the runtime never
    /// executes unknown hooks — an older runtime passes them
    /// through untouched. Exported to TypeScript as
    /// `{ [key in string]?: JsonValue }` alongside the known fields.
    #[serde(default, flatten)]
    pub extra: ExtraJson,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_lifecycle_partial() {
        let json = r#"{"onLaunch":[{"toast":"App started"}]}"#;
        let l: AppLifecycleHooks = serde_json::from_str(json).unwrap();
        assert!(l.on_launch.is_some());
        assert!(l.on_resume.is_none());
    }

    #[test]
    fn page_lifecycle() {
        let json = r#"{"onEnter":[{"set":{"$state.count":0}}]}"#;
        let l: PageLifecycleHooks = serde_json::from_str(json).unwrap();
        assert!(l.on_enter.is_some());
    }

    #[test]
    fn node_lifecycle() {
        let json = r#"{"onMount":[{"focus":{"nodeId":"email-input"}}]}"#;
        let l: NodeLifecycleHooks = serde_json::from_str(json).unwrap();
        assert!(l.on_mount.is_some());
    }

    #[test]
    fn lifecycle_hooks_and_future_fields_round_trip() {
        let node_input = serde_json::json!({
            "onMount": [{"animate":{"target":"$self","to":{"opacity":1},"futureCurve":"spring-v2"}}],
            "disabledEvents": ["onUnmount"],
            "interactionOrder": ["onMount", "onUnmount"],
            "onFutureVisibility": [{"futureAction":{"value":1}}]
        });
        let decoded: NodeLifecycleHooks = serde_json::from_value(node_input.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), node_input);

        let app_input = serde_json::json!({
            "onLaunch": [{"set":{"$app.launched":"true"}}],
            "onResume": [{"toast":"`Resumed`"}],
            "disabledEvents": ["onTerminate"],
            "interactionOrder": ["onLaunch", "onResume"],
            "onFutureApp": [{"futureAction":{"value":2}}]
        });
        let decoded: AppLifecycleHooks = serde_json::from_value(app_input.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), app_input);

        let page_input = serde_json::json!({
            "onEnter": [{"set":{"$page.entered":"true"}}],
            "onLeave": [{"push":"/home"}],
            "disabledEvents": ["onForeground", "onBackground"],
            "interactionOrder": ["onEnter", "onLeave"],
            "onFuturePage": [{"futureAction":{"value":3}}]
        });
        let decoded: PageLifecycleHooks = serde_json::from_value(page_input.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), page_input);
    }

    #[test]
    fn lifecycle_vector_order_and_duplicates_are_preserved() {
        let json = r#"{"disabledEvents":["onMount","onUnmount","onMount"],"interactionOrder":["onUnmount","onMount","onUnmount"]}"#;
        let l: NodeLifecycleHooks = serde_json::from_str(json).unwrap();
        let order = vec![
            "onMount".to_owned(),
            "onUnmount".to_owned(),
            "onMount".to_owned(),
        ];
        let reverse = vec![
            "onUnmount".to_owned(),
            "onMount".to_owned(),
            "onUnmount".to_owned(),
        ];
        assert_eq!(l.disabled_events.as_deref(), Some(order.as_slice()));
        assert_eq!(l.interaction_order.as_deref(), Some(reverse.as_slice()));
        let output = serde_json::to_value(&l).unwrap();
        assert_eq!(
            output["interactionOrder"],
            serde_json::json!(["onUnmount", "onMount", "onUnmount"])
        );
    }

    #[test]
    fn legacy_lifecycle_serialize_unchanged() {
        // Old empty objects: no new fields may leak into the output.
        assert_eq!(
            serde_json::to_value(AppLifecycleHooks::default()).unwrap(),
            serde_json::json!({})
        );
        assert_eq!(
            serde_json::to_value(PageLifecycleHooks::default()).unwrap(),
            serde_json::json!({})
        );
        assert_eq!(
            serde_json::to_value(NodeLifecycleHooks::default()).unwrap(),
            serde_json::json!({})
        );
        // Old fixture shape: only the authored key survives.
        let node_input = serde_json::json!({"onMount": [{"focus":{"nodeId":"email-input"}}]});
        let decoded: NodeLifecycleHooks = serde_json::from_value(node_input.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), node_input);
        let app_input = serde_json::json!({"onLaunch": [{"toast":"App started"}]});
        let decoded: AppLifecycleHooks = serde_json::from_value(app_input.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), app_input);
    }
}
