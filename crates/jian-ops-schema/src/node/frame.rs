use super::base::PenNodeBase;
use super::container::ContainerProps;
use serde::{Deserialize, Serialize};

/// Forward declaration of PenNode union — defined in `node/mod.rs`.
/// We accept `Vec<super::PenNode>` as children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct FrameNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(flatten)]
    pub container: ContainerProps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<super::PenNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_search_query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reusable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<crate::state::StateSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<crate::events::Bindings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<crate::events::EventHandlers>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<crate::lifecycle::NodeLifecycleHooks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantics: Option<crate::semantics::SemanticsMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gestures: Option<crate::gestures::GestureOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<crate::navigation::NavigationRoute>,
    /// Screen marker: this top-level frame is one screen of the app,
    /// mounted at the given route path ("/" = entry). Consumed only by
    /// the screen-projection pass; ignored elsewhere. Additive 1.x.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<String>,
    /// Breakpoint range for screen variants. Invalid ranges are stripped
    /// during responsive screen projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakpoint: Option<crate::breakpoint::BreakpointRange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct GroupNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(flatten)]
    pub container: ContainerProps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<super::PenNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<crate::state::StateSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<crate::events::Bindings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<crate::events::EventHandlers>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<crate::lifecycle::NodeLifecycleHooks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantics: Option<crate::semantics::SemanticsMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gestures: Option<crate::gestures::GestureOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<crate::navigation::NavigationRoute>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct RectangleNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(flatten)]
    pub container: ContainerProps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<super::PenNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<crate::state::StateSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<crate::events::Bindings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<crate::events::EventHandlers>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<crate::lifecycle::NodeLifecycleHooks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantics: Option<crate::semantics::SemanticsMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gestures: Option<crate::gestures::GestureOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<crate::navigation::NavigationRoute>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::breakpoint::BreakpointRange;

    #[test]
    fn breakpoint_parses_and_legacy_roundtrip_unchanged() {
        let breakpoint: BreakpointRange =
            serde_json::from_str(r#"{"minWidth":0,"maxWidth":480}"#).unwrap();
        assert!(breakpoint.validate().is_ok());
        let frame: FrameNode = serde_json::from_str(r#"{"id":"f","screen":"/home"}"#).unwrap();
        assert!(frame.breakpoint.is_none());
        assert!(serde_json::to_value(frame)
            .unwrap()
            .get("breakpoint")
            .is_none());
    }

    #[test]
    fn invalid_breakpoint_ranges_are_detected() {
        for json in [
            r#"{"minWidth":500,"maxWidth":480}"#,
            r#"{"minWidth":-1}"#,
            r#"{"maxWidth":null}"#,
            r#"{}"#,
        ] {
            let breakpoint: BreakpointRange = serde_json::from_str(json).unwrap();
            assert!(breakpoint.validate().is_err(), "{json}");
        }
    }

    #[test]
    fn frame_screen_marker_roundtrip() {
        let json = r#"{"id":"f1","screen":"/checkout"}"#;
        let f: FrameNode = serde_json::from_str(json).unwrap();
        assert_eq!(f.screen.as_deref(), Some("/checkout"));
        assert_eq!(serde_json::to_string(&f).unwrap(), json);
    }

    #[test]
    fn frame_without_screen_serializes_without_key() {
        let f: FrameNode = serde_json::from_str(r#"{"id":"f1"}"#).unwrap();
        assert_eq!(f.screen, None);
        // Canonical serialization of an unmarked frame must not grow a key.
        assert!(!serde_json::to_string(&f).unwrap().contains("screen"));
    }
}
