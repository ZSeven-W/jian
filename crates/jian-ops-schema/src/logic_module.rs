use crate::app::Capability;
use serde::{Deserialize, Serialize};

/// An ABI version string for the Tier 3 WASM module. The only recognised
/// value today is `jian.wasm.v1`; Jian rejects unknown ABI at load time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct LogicAbi(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogicModuleRef {
    pub id: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
    pub abi: LogicAbi,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<Capability>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_logic_module() {
        let json = r#"{"id":"finance","source":"bundle://finance.wasm","abi":"jian.wasm.v1"}"#;
        let m: LogicModuleRef = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "finance");
        assert_eq!(m.abi.0, "jian.wasm.v1");
    }

    #[test]
    fn logic_module_with_integrity_and_caps() {
        let json = r#"{
          "id":"crypto",
          "source":"https://cdn.example.com/crypto.wasm",
          "integrity":"sha256-deadbeef",
          "abi":"jian.wasm.v1",
          "capabilities":["network"]
        }"#;
        let m: LogicModuleRef = serde_json::from_str(json).unwrap();
        assert_eq!(m.integrity.unwrap(), "sha256-deadbeef");
        assert_eq!(m.capabilities.unwrap().len(), 1);
    }
}
