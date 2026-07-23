use crate::lifecycle::PageLifecycleHooks;
use crate::node::PenNode;
use crate::state::StateSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PenPage {
    pub id: String,
    pub name: String,
    pub children: Vec<PenNode>,

    /// Optional infinite-canvas background for this page. When absent,
    /// editors use their normal canvas surface and grid treatment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "export-ts", ts(optional = nullable))]
    pub background_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<StateSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<PageLifecycleHooks>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_json_without_background_color_still_round_trips() {
        let json = r#"{"id":"page-1","name":"Page 1","children":[]}"#;
        let page: PenPage = serde_json::from_str(json).expect("legacy PenPage");
        assert_eq!(page.background_color, None);
        assert_eq!(serde_json::to_string(&page).unwrap(), json);
    }

    #[test]
    fn background_color_uses_camel_case_wire_name() {
        let page: PenPage = serde_json::from_str(
            r##"{"id":"page-1","name":"Page 1","children":[],"backgroundColor":"#d7e4f380"}"##,
        )
        .expect("PenPage with background");
        assert_eq!(page.background_color.as_deref(), Some("#d7e4f380"));
    }
}
