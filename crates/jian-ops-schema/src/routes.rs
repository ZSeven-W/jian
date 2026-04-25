use crate::events::ActionList;
use crate::state::StateType;
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

    /// Path-parameter type declarations (v1.0 additive — 2026-04-24).
    /// Keys correspond to `:param` placeholders in the route path
    /// (e.g. path `/detail/:id` → key `id`). The AI Action Surface
    /// uses these types when synthesising the JsonSchema for the
    /// derived `open_*(p)` action; runtime does strict type-checking
    /// on incoming values rather than silent coercion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<BTreeMap<String, StateType>>,
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

    #[test]
    fn route_with_params() {
        use crate::state::PrimitiveType;
        let json = r#"{
          "pageId":"detail",
          "params":{"id":"int","slug":"string"}
        }"#;
        let r: RouteSpec = serde_json::from_str(json).unwrap();
        let params = r.params.unwrap();
        assert!(matches!(
            params.get("id"),
            Some(StateType::Primitive(PrimitiveType::Int))
        ));
        assert!(matches!(
            params.get("slug"),
            Some(StateType::Primitive(PrimitiveType::String))
        ));
    }

    #[test]
    fn route_params_optional_for_back_compat() {
        let json = r#"{"pageId":"home"}"#;
        let r: RouteSpec = serde_json::from_str(json).unwrap();
        assert!(r.params.is_none());
    }
}
