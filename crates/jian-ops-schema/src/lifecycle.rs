use crate::events::ActionList;
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
}
