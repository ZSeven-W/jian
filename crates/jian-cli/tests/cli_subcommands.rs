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
fn check_warning_renders_rustc_style_caret_excerpt() {
    // The rustc-style renderer should print:
    //   - "warning: unknown field `mysteryField`"
    //   - a `path:line:col` location anchor
    //   - the source excerpt line containing the field
    //   - a row of `^` characters underlining the field key
    let dir = TempDir::new().unwrap();
    let path = write_tmp(&dir, "warn.op", WARNING_OP);
    let out = Command::cargo_bin("jian")
        .unwrap()
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("warning: unknown field `mysteryField`"),
        "missing rustc-style title, got:\n{}",
        stdout
    );
    // Path anchor includes :line:col after the file path.
    assert!(
        stdout.contains("warn.op:"),
        "missing path anchor, got:\n{}",
        stdout
    );
    // Excerpt: the source line is reproduced.
    assert!(
        stdout.contains("\"mysteryField\": 42"),
        "missing source excerpt, got:\n{}",
        stdout
    );
    // Caret row has at least one ^.
    assert!(
        stdout.contains("^^^^"),
        "missing caret underline, got:\n{}",
        stdout
    );
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

// `player` is feature-gated; without `--features player` the
// subcommand doesn't exist and clap returns "unrecognized
// subcommand" before any dpi / fullscreen parsing kicks in. Gate
// the player-flag tests so `cargo test --no-default-features`
// passes — pre-existing tests treated this as a bug, but the
// product contract is "no `player` feature → no `player` cmd".
#[cfg(feature = "player")]
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

#[cfg(feature = "player")]
#[test]
fn player_dpi_zero_is_rejected_by_clap() {
    // Negative-path test for `--dpi`: clap's `value_parser` rejects 0 /
    // negative / non-finite at parse time, so the run loop never starts
    // and no display is required for the assertion. This pins the
    // validation contract — a refactor that drops `parse_positive_dpi`
    // breaks here, not in a user terminal.
    let dir = TempDir::new().unwrap();
    let path = write_tmp(&dir, "anything.op", CLEAN_OP);
    let out = Command::cargo_bin("jian")
        .unwrap()
        .args(["player", "--dpi", "0", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "--dpi 0 should be rejected, stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("must be a finite number > 0"),
        "expected dpi error message, got stderr={}",
        stderr
    );
}

#[cfg(feature = "player")]
#[test]
fn player_dpi_negative_is_rejected_by_clap() {
    let dir = TempDir::new().unwrap();
    let path = write_tmp(&dir, "anything.op", CLEAN_OP);
    let out = Command::cargo_bin("jian")
        .unwrap()
        .args(["player", "--dpi", "-1.5", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[cfg(feature = "player")]
#[test]
fn player_dpi_non_numeric_is_rejected_by_clap() {
    let dir = TempDir::new().unwrap();
    let path = write_tmp(&dir, "anything.op", CLEAN_OP);
    let out = Command::cargo_bin("jian")
        .unwrap()
        .args(["player", "--dpi", "abc", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not a number"), "stderr={}", stderr);
}

#[cfg(feature = "player")]
#[test]
fn player_help_advertises_dpi_and_debug_overlay() {
    // `--help` exits before any window logic, so this works headless on
    // CI and proves the new flags are publicly visible.
    let out = Command::cargo_bin("jian")
        .unwrap()
        .args(["player", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--dpi"), "expected --dpi in help");
    assert!(
        stdout.contains("--debug-overlay"),
        "expected --debug-overlay in help"
    );
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

#[test]
fn pack_include_fonts_bundles_assets_fonts_directory() {
    let dir = TempDir::new().unwrap();
    let src = write_tmp(&dir, "src.op", CLEAN_OP);
    let fonts_dir = dir.path().join("assets").join("fonts");
    fs::create_dir_all(&fonts_dir).unwrap();
    fs::write(fonts_dir.join("Inter.ttf"), b"FAKE-TTF-1").unwrap();
    fs::write(fonts_dir.join("Roboto.otf"), b"FAKE-OTF-2").unwrap();
    fs::write(fonts_dir.join("README.md"), b"not a font").unwrap();
    let packed = dir.path().join("out.op.pack");

    Command::cargo_bin("jian")
        .unwrap()
        .args([
            "pack",
            "--include-fonts",
            src.to_str().unwrap(),
            packed.to_str().unwrap(),
        ])
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

    let inter = extracted.join("assets/fonts/Inter.ttf");
    let roboto = extracted.join("assets/fonts/Roboto.otf");
    assert!(inter.is_file());
    assert!(roboto.is_file());
    assert_eq!(fs::read(&inter).unwrap(), b"FAKE-TTF-1");
    assert_eq!(fs::read(&roboto).unwrap(), b"FAKE-OTF-2");
    // Non-font files in the dir are ignored.
    assert!(!extracted.join("assets/fonts/README.md").exists());

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(extracted.join("manifest.json")).unwrap()).unwrap();
    let entries: Vec<&str> = manifest["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(entries.contains(&"assets/fonts/Inter.ttf"));
    assert!(entries.contains(&"assets/fonts/Roboto.otf"));
}

#[test]
fn pack_include_images_content_addresses_and_dedupes() {
    let dir = TempDir::new().unwrap();
    let src = write_tmp(&dir, "src.op", CLEAN_OP);
    let images_dir = dir.path().join("assets").join("images");
    fs::create_dir_all(&images_dir).unwrap();
    fs::write(images_dir.join("cat.png"), b"PNG-DATA-A").unwrap();
    // Same content, different name → dedupes to one zip entry.
    fs::write(images_dir.join("cat-copy.png"), b"PNG-DATA-A").unwrap();
    fs::write(images_dir.join("dog.jpg"), b"JPG-DATA-B").unwrap();
    fs::write(images_dir.join("notes.txt"), b"not an image").unwrap();
    let packed = dir.path().join("out.op.pack");

    Command::cargo_bin("jian")
        .unwrap()
        .args([
            "pack",
            "--include-images",
            src.to_str().unwrap(),
            packed.to_str().unwrap(),
        ])
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

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(extracted.join("manifest.json")).unwrap()).unwrap();

    let images = manifest["images"].as_object().unwrap();
    let cat_path = images["cat.png"].as_str().unwrap();
    let cat_copy_path = images["cat-copy.png"].as_str().unwrap();
    let dog_path = images["dog.jpg"].as_str().unwrap();
    // Identical bytes → identical zip path. Different bytes → different path.
    assert_eq!(cat_path, cat_copy_path);
    assert_ne!(cat_path, dog_path);
    assert!(cat_path.starts_with("assets/images/"));
    assert!(cat_path.ends_with(".png"));
    assert!(dog_path.ends_with(".jpg"));

    // Both physical files unpacked successfully (cat content bundled once).
    let cat_bytes = fs::read(extracted.join(cat_path)).unwrap();
    let dog_bytes = fs::read(extracted.join(dog_path)).unwrap();
    assert_eq!(cat_bytes, b"PNG-DATA-A");
    assert_eq!(dog_bytes, b"JPG-DATA-B");
    assert!(!extracted.join("assets/images/notes.txt").exists());

    // Entries list also dedupes — three image inputs → two physical entries.
    let entries: Vec<&str> = manifest["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let asset_entries: Vec<&&str> = entries
        .iter()
        .filter(|e| e.starts_with("assets/images/"))
        .collect();
    assert_eq!(asset_entries.len(), 2, "dedup leaves two unique entries");
}

#[test]
fn pack_without_include_flags_omits_assets_dir() {
    // Even when assets/ exists, the absence of --include-fonts /
    // --include-images keeps the archive minimal — the bare-pack path
    // hasn't regressed.
    let dir = TempDir::new().unwrap();
    let src = write_tmp(&dir, "src.op", CLEAN_OP);
    let fonts_dir = dir.path().join("assets").join("fonts");
    fs::create_dir_all(&fonts_dir).unwrap();
    fs::write(fonts_dir.join("Inter.ttf"), b"FAKE-TTF").unwrap();
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

    assert!(!extracted.join("assets/fonts/Inter.ttf").exists());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(extracted.join("manifest.json")).unwrap()).unwrap();
    assert!(
        manifest.get("images").is_none(),
        "no `images` key when no images bundled"
    );
}
