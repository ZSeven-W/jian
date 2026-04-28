//! End-to-end CLI tests — spawn the `jian` binary via `assert_cmd`.
//!
//! Covers:
//! - `jian check FILE` on clean / warning / malformed input
//! - `jian new NAME` scaffolds a project that subsequently passes `check`
//! - `jian pack` + `jian unpack` roundtrip

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

const CLEAN_OP: &str = r##"{
  "formatVersion": "1.0",
  "version": "1.0.0",
  "id": "x",
  "app": { "name": "x", "version": "1", "id": "x" },
  "children": []
}"##;

const WARNING_OP: &str = r##"{
  "formatVersion": "1.0",
  "version": "1.0.0",
  "id": "x",
  "app": { "name": "x", "version": "1", "id": "x" },
  "children": [],
  "mysteryField": 42
}"##;

const MALFORMED_OP: &str = r##"{ "formatVersion": "1.0", "version": " }"##;

fn write_tmp(dir: &TempDir, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.path().join(name);
    fs::write(&p, body).unwrap();
    p
}

#[test]
fn check_clean_exits_zero() {
    let dir = TempDir::new().unwrap();
    let path = write_tmp(&dir, "clean.op", CLEAN_OP);
    Command::cargo_bin("jian")
        .unwrap()
        .args(["check", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("OK, no diagnostics"));
}

#[test]
fn check_warning_exits_one() {
    let dir = TempDir::new().unwrap();
    let path = write_tmp(&dir, "warn.op", WARNING_OP);
    Command::cargo_bin("jian")
        .unwrap()
        .args(["check", path.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("mysteryField"));
}

#[test]
fn check_malformed_exits_two() {
    let dir = TempDir::new().unwrap();
    let path = write_tmp(&dir, "bad.op", MALFORMED_OP);
    Command::cargo_bin("jian")
        .unwrap()
        .args(["check", path.to_str().unwrap()])
        .assert()
        .code(2);
}

#[test]
fn player_size_and_fullscreen_are_mutually_exclusive() {
    // No window is opened — clap's argument parser rejects the
    // combination during arg validation, before player.rs ever runs.
    // The test pins the `conflicts_with` contract so a future arg-
    // refactor that drops it triggers here, not in a user's terminal.
    let dir = TempDir::new().unwrap();
    let path = write_tmp(&dir, "anything.op", CLEAN_OP);
    let out = Command::cargo_bin("jian")
        .unwrap()
        .args([
            "player",
            "--size",
            "640x480",
            "--fullscreen",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "expected clap to reject --size + --fullscreen combo"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be used with"),
        "expected clap conflict message, got stderr={stderr:?}"
    );
}

#[test]
fn check_quiet_silences_success_line_only() {
    let dir = TempDir::new().unwrap();
    let clean = write_tmp(&dir, "clean.op", CLEAN_OP);
    // Clean run + --quiet: no stdout at all (success line suppressed),
    // exit code still 0.
    let out = Command::cargo_bin("jian")
        .unwrap()
        .args(["check", "--quiet", clean.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "expected empty stdout under --quiet on a clean check, got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );

    // Warning run + --quiet: warnings still printed, exit 1.
    let warn = write_tmp(&dir, "warn.op", WARNING_OP);
    Command::cargo_bin("jian")
        .unwrap()
        .args(["check", "--quiet", warn.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("mysteryField"));
}

#[test]
fn check_json_emits_ndjson_per_warning() {
    let dir = TempDir::new().unwrap();
    let path = write_tmp(&dir, "warn.op", WARNING_OP);
    let out = Command::cargo_bin("jian")
        .unwrap()
        .args(["check", "--json", path.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1);
    let parsed: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(parsed["kind"], "unknown_field");
}

#[test]
fn new_then_check_is_clean() {
    let dir = TempDir::new().unwrap();
    // Scaffold into the temp dir.
    Command::cargo_bin("jian")
        .unwrap()
        .current_dir(dir.path())
        .args(["new", "hello"])
        .assert()
        .success();
    let op_path = dir.path().join("hello/app.op");
    assert!(op_path.exists(), "template should create app.op");

    // The scaffolded document should parse cleanly.
    Command::cargo_bin("jian")
        .unwrap()
        .args(["check", op_path.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn new_rejects_path_traversal_in_name() {
    let dir = TempDir::new().unwrap();
    for bad in ["..", "../evil", "a/b", "a\\b", "."] {
        Command::cargo_bin("jian")
            .unwrap()
            .current_dir(dir.path())
            .args(["new", bad])
            .assert()
            .failure();
    }
}

#[test]
fn check_flags_missing_top_level_id_as_semantic_error() {
    // The spec says `id` is required when `app` is set. serde alone
    // doesn't enforce that — `check` does.
    const NO_ID: &str = r##"{
      "formatVersion": "1.0",
      "version": "1.0.0",
      "app": { "name": "x", "version": "1", "id": "x" },
      "children": []
    }"##;
    let dir = TempDir::new().unwrap();
    let path = write_tmp(&dir, "no_id.op", NO_ID);
    Command::cargo_bin("jian")
        .unwrap()
        .args(["check", path.to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("top-level `id`"));
}

#[test]
fn new_form_template_scaffolds_and_checks_clean() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("jian")
        .unwrap()
        .current_dir(dir.path())
        .args(["new", "contact", "--template", "form"])
        .assert()
        .success();
    let op_path = dir.path().join("contact/app.op");
    Command::cargo_bin("jian")
        .unwrap()
        .args(["check", op_path.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn pack_then_unpack_roundtrips_app_op() {
    let dir = TempDir::new().unwrap();
    let src = write_tmp(&dir, "src.op", CLEAN_OP);
    let packed = dir.path().join("out.op.pack");

    Command::cargo_bin("jian")
        .unwrap()
        .args(["pack", src.to_str().unwrap(), packed.to_str().unwrap()])
        .assert()
        .success();

    let extracted = dir.path().join("extracted");
    Command::cargo_bin("jian")
        .unwrap()
        .args([
            "unpack",
            packed.to_str().unwrap(),
            extracted.to_str().unwrap(),
        ])
        .assert()
        .success();

    // The extracted app.op should match the source byte-for-byte.
    let out = fs::read_to_string(extracted.join("app.op")).unwrap();
    assert_eq!(out, CLEAN_OP);
    assert!(extracted.join("manifest.json").exists());
}
