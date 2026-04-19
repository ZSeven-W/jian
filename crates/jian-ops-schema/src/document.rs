use crate::node::PenNode;
use crate::page::PenPage;
use crate::variable::VariableDefinition;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level document. Matches existing v0.x `.op` files exactly.
/// Jian extension fields (app / routes / state / lifecycle / logicModules) are added
/// in Task 15. For now this supports the full v0.x file set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PenDocument {
    /// `version` in v0.x, becomes `formatVersion` in v1.0 via compat layer (Task 16).
    pub version: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Theme axes; values are ordered theme names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub themes: Option<BTreeMap<String, Vec<String>>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<BTreeMap<String, VariableDefinition>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<Vec<PenPage>>,

    /// Fallback when `pages` is absent.
    pub children: Vec<PenNode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_document() {
        let json = r#"{"version":"0.8.0","children":[]}"#;
        let d: PenDocument = serde_json::from_str(json).unwrap();
        assert_eq!(d.version, "0.8.0");
        assert!(d.children.is_empty());
        assert_eq!(serde_json::to_string(&d).unwrap(), json);
    }

    #[test]
    fn document_with_page_and_variable() {
        let json = r##"{
          "version":"0.8.0",
          "name":"My Doc",
          "variables":{"primary":{"type":"color","value":"#ff0000"}},
          "themes":{"mode":["light","dark"]},
          "pages":[{"id":"p1","name":"Home","children":[]}],
          "children":[]
        }"##;
        let d: PenDocument = serde_json::from_str(json).unwrap();
        assert_eq!(d.variables.as_ref().unwrap().len(), 1);
        assert_eq!(d.pages.as_ref().unwrap().len(), 1);
    }
}
