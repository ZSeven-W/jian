//! Backward/forward compat loader.
//!
//! Responsibilities:
//! 1. Parse JSON into `PenDocument` (serde-level).
//! 2. Check `formatVersion` / `version` and reject unsupported majors.
//! 3. Collect non-fatal warnings for unknown fields (future-compat).
//! 4. Return document + warnings.

use crate::document::PenDocument;
use crate::error::{LoadResult, LoadWarning, OpsResult, OpsSchemaError};
use crate::version;

/// Parse an `.op` JSON blob into a `PenDocument` with compat warnings.
pub fn load_str(src: &str) -> OpsResult<LoadResult<PenDocument>> {
    let raw: serde_json::Value = serde_json::from_str(src)?;

    let format_version = raw.get("formatVersion").and_then(|v| v.as_str());
    let legacy_version = raw.get("version").and_then(|v| v.as_str());
    let v = format_version.or(legacy_version);

    if !version::supports(v) {
        return Err(OpsSchemaError::UnsupportedFormatVersion {
            found: v.unwrap_or("<missing>").to_owned(),
            supported: version::FORMAT_VERSION_CURRENT,
        });
    }

    let mut warnings = Vec::new();
    if let serde_json::Value::Object(map) = &raw {
        for k in map.keys() {
            if !KNOWN_TOP_LEVEL_FIELDS.contains(&k.as_str()) {
                warnings.push(LoadWarning::UnknownField {
                    path: "$".to_owned(),
                    field: k.to_owned(),
                });
            }
        }
    }

    if let Some(fv) = format_version {
        let (major, _) = version::parse(Some(fv));
        if major > 1 {
            warnings.push(LoadWarning::FutureFormatVersion {
                found: fv.to_owned(),
                supported_max: version::FORMAT_VERSION_CURRENT,
            });
        }
    }

    if raw.get("logicModules").is_some() {
        warnings.push(LoadWarning::LogicModulesSkipped {
            reason: "Tier 3 WASM is not implemented in this build",
        });
    }

    let doc: PenDocument = serde_json::from_str(src)?;

    Ok(LoadResult {
        value: doc,
        warnings,
    })
}

const KNOWN_TOP_LEVEL_FIELDS: &[&str] = &[
    "formatVersion",
    "version",
    "id",
    "name",
    "themes",
    "variables",
    "pages",
    "children",
    "app",
    "routes",
    "state",
    "lifecycle",
    "logicModules",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_v0() {
        let s = r#"{"version":"0.8.0","children":[]}"#;
        let r = load_str(s).unwrap();
        assert!(r.value.format_version.is_none());
        assert_eq!(r.warnings.len(), 0);
    }

    #[test]
    fn load_v1_minimal() {
        let s = r#"{"formatVersion":"1.0","version":"1.0.0","id":"x","children":[]}"#;
        let r = load_str(s).unwrap();
        assert_eq!(r.value.format_version.as_deref(), Some("1.0"));
        assert_eq!(r.warnings.len(), 0);
    }

    #[test]
    fn load_unknown_field_produces_warning() {
        let s = r#"{"version":"0.8.0","children":[],"myExperimental":42}"#;
        let r = load_str(s).unwrap();
        assert!(r.warnings.iter().any(
            |w| matches!(w, LoadWarning::UnknownField { field, .. } if field == "myExperimental")
        ));
    }

    #[test]
    fn load_v2_is_rejected() {
        let s = r#"{"formatVersion":"2.0","version":"2","children":[]}"#;
        assert!(matches!(
            load_str(s),
            Err(OpsSchemaError::UnsupportedFormatVersion { .. })
        ));
    }
}
