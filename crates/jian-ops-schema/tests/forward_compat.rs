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
    assert!(
        !r.warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::FutureFormatVersion { .. }))
    );
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
    assert!(
        r.warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::LogicModulesSkipped { .. }))
    );
}
