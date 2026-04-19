use crate::app::AppConfig;
use crate::lifecycle::AppLifecycleHooks;
use crate::logic_module::LogicModuleRef;
use crate::node::PenNode;
use crate::page::PenPage;
use crate::routes::RoutesConfig;
use crate::state::StateSchema;
use crate::variable::VariableDefinition;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PenDocument {
    // --- v0.x frozen fields ---
    /// Document format version stored in files since v0.x. Always present.
    pub version: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Wire shape: axis-name → ordered theme names. Frozen from v0.x.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub themes: Option<BTreeMap<String, Vec<String>>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<BTreeMap<String, VariableDefinition>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<Vec<PenPage>>,

    pub children: Vec<PenNode>,

    // --- Jian v1 extensions (all optional, additive, not present in v0.x) ---
    /// "1.0" when any v1 extension is present; undefined ⇒ legacy v0.x.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_version: Option<String>,

    /// App id (reverse-DNS). Required when `app` is set; otherwise optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<AppConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routes: Option<RoutesConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<StateSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<AppLifecycleHooks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logic_modules: Option<Vec<LogicModuleRef>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_document() {
        let input = r#"{"version":"0.8.0","children":[]}"#;
        let d: PenDocument = serde_json::from_str(input).unwrap();
        assert_eq!(d.version, "0.8.0");
        assert!(d.children.is_empty());

        let output = serde_json::to_string(&d).unwrap();
        assert_eq!(output, r#"{"version":"0.8.0","children":[]}"#);
    }

    #[test]
    fn document_with_jian_extensions() {
        let input = r#"{
          "formatVersion":"1.0",
          "version":"1.0.0",
          "id":"com.example.counter",
          "app":{"name":"Counter","version":"1.0.0","id":"com.example.counter"},
          "state":{"count":{"type":"int","default":0}},
          "lifecycle":{"onLaunch":[{"toast":"hi"}]},
          "children":[]
        }"#;
        let d: PenDocument = serde_json::from_str(input).unwrap();
        assert!(d.app.is_some());
        assert!(d.state.is_some());
        assert!(d.lifecycle.is_some());
        assert_eq!(d.format_version.as_deref(), Some("1.0"));
    }
}
