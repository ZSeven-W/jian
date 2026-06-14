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

/// Options controlling compat-load behaviour.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoadOptions {
    /// Promote explicitly marked legacy frames to widget nodes
    /// (in-memory only). Runtime hosts set this; the OP editor keeps
    /// it off so migration stays an explicit user action.
    pub promote_legacy_widgets: bool,
}

/// Parse an `.op` JSON blob into a `PenDocument` with compat warnings.
pub fn load_str(src: &str) -> OpsResult<LoadResult<PenDocument>> {
    load_str_with(src, LoadOptions::default())
}

/// Like [`load_str`], but with explicit [`LoadOptions`].
pub fn load_str_with(src: &str, opts: LoadOptions) -> OpsResult<LoadResult<PenDocument>> {
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
        let (cur_major, cur_minor) = version::parse(Some(version::FORMAT_VERSION_CURRENT));
        let (major, minor) = version::parse(Some(fv));
        // Warn on any version newer than we know — including a newer
        // *minor*, which may carry node types or fields we silently
        // drop. (A newer major is already rejected above.)
        if major > cur_major || (major == cur_major && minor > cur_minor) {
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

    let mut doc: PenDocument = serde_json::from_str(src)?;

    if opts.promote_legacy_widgets {
        for n in crate::promote::promote_document(&mut doc) {
            warnings.push(LoadWarning::LegacyRolePromoted {
                path: n.node_id,
                from_role: n.from_role,
                to: n.to,
            });
        }
    }

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
    fn future_minor_warns_instead_of_silent() {
        let src = r#"{"formatVersion":"1.7","version":"1.7.0","children":[]}"#;
        let r = load_str(src).unwrap();
        assert!(r
            .warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::FutureFormatVersion { .. })));
    }

    #[test]
    fn current_minor_does_not_warn() {
        let src = r#"{"formatVersion":"1.1","version":"1.1.0","children":[]}"#;
        let r = load_str(src).unwrap();
        assert!(!r
            .warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::FutureFormatVersion { .. })));
    }

    #[test]
    fn promote_is_off_by_default_on_for_load_str_with() {
        let legacy = r#"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"f1","role":"input","children":[]}]}"#;
        // Default load leaves the legacy frame as a frame.
        let r = load_str(legacy).unwrap();
        assert!(matches!(
            r.value.children[0],
            crate::node::PenNode::Frame(_)
        ));
        assert!(!r
            .warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::LegacyRolePromoted { .. })));
        // Opt-in load promotes it and reports a warning.
        let r2 = load_str_with(
            legacy,
            LoadOptions {
                promote_legacy_widgets: true,
            },
        )
        .unwrap();
        assert!(matches!(
            r2.value.children[0],
            crate::node::PenNode::TextInput(_)
        ));
        assert!(r2.warnings.iter().any(
            |w| matches!(w, LoadWarning::LegacyRolePromoted { to, .. } if *to == "text_input")
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
