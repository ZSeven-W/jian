//! Verify that unknown fields / future versions produce warnings but don't fail.

use jian_ops_schema::{load_str, LoadWarning, OpsSchemaError};

#[test]
fn unknown_top_level_field_warns() {
    let src = r#"{"version":"0.8.0","children":[],"customField":"value","anotherCustom":42}"#;
    let r = load_str(src).unwrap();
    let unknowns: Vec<&str> = r
        .warnings
        .iter()
        .filter_map(|w| match w {
            LoadWarning::UnknownField { field, .. } => Some(field.as_str()),
            _ => None,
        })
        .collect();
    assert!(unknowns.contains(&"customField"));
    assert!(unknowns.contains(&"anotherCustom"));
}

#[test]
fn future_major_rejected() {
    let src = r#"{"formatVersion":"2.0","version":"2.0","children":[]}"#;
    assert!(matches!(
        load_str(src),
        Err(OpsSchemaError::UnsupportedFormatVersion { .. })
    ));
}

#[test]
fn future_minor_accepted_with_warning_absent() {
    // formatVersion "1.5" still has major=1, so it loads; no future-version warning
    // since future *minors* are expected to be backward-compatible.
    let src = r#"{"formatVersion":"1.5","version":"1.5.0","children":[]}"#;
    let r = load_str(src).unwrap();
    assert!(!r
        .warnings
        .iter()
        .any(|w| matches!(w, LoadWarning::FutureFormatVersion { .. })));
}

/// `tests/forward-compat/pencil-demo.op` is a real OpenPencil v2.8
/// export. It uses the v0.x shape under the v2.8 editor format tag,
/// so once the loader sees `version: "2.8"` it must reject the file
/// before serde gets a chance to (incorrectly) accept it.
#[test]
fn pencil_demo_v2_export_rejected() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/forward-compat/pencil-demo.op");
    let src =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    match load_str(&src) {
        Err(OpsSchemaError::UnsupportedFormatVersion { found, .. }) => {
            assert!(
                found.starts_with('2'),
                "expected v2.x rejection, got found={found}"
            );
        }
        Err(other) => panic!("expected UnsupportedFormatVersion, got {other:?}"),
        Ok(_) => panic!("v2 export must be rejected by loader"),
    }
}

#[test]
fn logic_modules_produce_skip_warning() {
    let src = r#"{
      "formatVersion":"1.0",
      "version":"1.0.0",
      "children":[],
      "logicModules":[{"id":"x","source":"bundle://x.wasm","abi":"jian.wasm.v1"}]
    }"#;
    let r = load_str(src).unwrap();
    assert!(r
        .warnings
        .iter()
        .any(|w| matches!(w, LoadWarning::LogicModulesSkipped { .. })));
}
