use crate::events::ActionList;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum Transition {
    Push,
    Fade,
    Modal,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct RouteSpec {
    pub page_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preload: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guards: Option<ActionList>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct RoutesConfig {
    pub entry: String,
    pub routes: BTreeMap<String, RouteSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transitions: Option<BTreeMap<String, Transition>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_routes() {
        let json = r#"{
          "entry": "/",
          "routes": {
            "/":       {"pageId":"home"},
            "/detail": {"pageId":"detail"}
          }
        }"#;
        let r: RoutesConfig = serde_json::from_str(json).unwrap();
        assert_eq!(r.entry, "/");
        assert_eq!(r.routes.len(), 2);
    }

    #[test]
    fn routes_with_transitions() {
        let json = r#"{
          "entry":"/",
          "routes":{"/":{"pageId":"home"}},
          "transitions":{"/detail":"push","/settings":"modal"}
        }"#;
        let r: RoutesConfig = serde_json::from_str(json).unwrap();
        let tr = r.transitions.unwrap();
        assert_eq!(tr.get("/detail"), Some(&Transition::Push));
        assert_eq!(tr.get("/settings"), Some(&Transition::Modal));
    }
}
