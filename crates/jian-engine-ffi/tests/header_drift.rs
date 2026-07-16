use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

const CBINDGEN_VERSION: &str = "cbindgen 0.29.4";

fn reports_expected_version(binary: &Path) -> Result<bool, String> {
    let output = match Command::new(binary).arg("--version").output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    if !output.status.success() {
        return Ok(false);
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if version != CBINDGEN_VERSION {
        return Err(format!(
            "jian.h requires {CBINDGEN_VERSION}, but {} reports {version}",
            binary.display()
        ));
    }
    Ok(true)
}

fn cbindgen_binary() -> Result<Option<PathBuf>, String> {
    let mut candidates = Vec::new();
    if let Some(binary) = env::var_os("CBINDGEN") {
        candidates.push(PathBuf::from(binary));
    }
    candidates.push(PathBuf::from("cbindgen"));
    if let Some(home) = env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".cargo/bin/cbindgen"));
    }

    for candidate in candidates {
        match reports_expected_version(&candidate) {
            Ok(true) => return Ok(Some(candidate)),
            Ok(false) => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

#[test]
fn missing_cbindgen_candidate_is_skippable() {
    let missing = env::temp_dir().join(format!("missing-cbindgen-{}", process::id()));
    assert_eq!(reports_expected_version(&missing), Ok(false));
}

fn first_difference(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .position(|(left, right)| left != right)
        .unwrap_or(left.len().min(right.len()))
}

fn c_compiler() -> Option<PathBuf> {
    let compiler = env::var_os("CC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cc"));
    Command::new(&compiler)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| compiler)
}

#[test]
fn checked_in_header_compiles_for_a_c_consumer() {
    let Some(compiler) = c_compiler() else {
        eprintln!("skipping jian.h C syntax check: no compiler found via CC or cc");
        return;
    };
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = env::temp_dir().join(format!("jian-header-consumer-{}.c", process::id()));
    let consumer = r#"
#include "jian.h"

static void on_redraw(void *data, bool has_wake, uint64_t wake_ms) {
  (void)data; (void)has_wake; (void)wake_ms;
}

static void on_capability(void *data, uint64_t id, const JianCapabilityRequest *request) {
  (void)data; (void)id; (void)request;
}

int main(void) {
  JianCallbacks callbacks = {0};
  callbacks.size = sizeof(JianCallbacks);
  callbacks.needs_redraw = on_redraw;
  callbacks.capability_request = on_capability;
  JianCreateDesc desc = {0};
  desc.size = sizeof(JianCreateDesc);
  desc.callbacks = &callbacks;
  JianCapabilityResult result = {0};
  result.size = sizeof(JianCapabilityResult);
  JianPointerPhase phase = JianPointerPhase_Down;
  JianImeControlOp ime_op = JianImeControlOp_Commit;
  JianTextGranularity granularity = JianTextGranularity_Character;
  return (int)(desc.size + result.size + phase + ime_op + granularity == 0);
}
"#;
    fs::write(&source, consumer)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", source.display()));
    let output = Command::new(&compiler)
        .current_dir(&crate_dir)
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
        .arg("-Iinclude")
        .arg(&source)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", compiler.display()));
    let _ = fs::remove_file(&source);
    assert!(
        output.status.success(),
        "jian.h is not valid C:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn generated_header_matches_checked_in_contract() {
    let binary = cbindgen_binary().unwrap_or_else(|error| panic!("{error}"));
    let Some(binary) = binary else {
        eprintln!(
            "skipping jian.h drift check: {CBINDGEN_VERSION} is not on PATH, in CBINDGEN, or in ~/.cargo/bin"
        );
        return;
    };

    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let checked_in = crate_dir.join("include/jian.h");
    let generated = env::temp_dir().join(format!(
        "jian-header-drift-{}-{}.h",
        process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let metadata = generated.with_extension("metadata.json");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let metadata_output = Command::new(cargo)
        .current_dir(&crate_dir)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
        ])
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo metadata: {error}"));
    assert!(
        metadata_output.status.success(),
        "cargo metadata failed:\n{}",
        String::from_utf8_lossy(&metadata_output.stderr)
    );
    fs::write(&metadata, metadata_output.stdout)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", metadata.display()));

    let output = Command::new(&binary)
        .current_dir(&crate_dir)
        .args([
            "--config",
            "cbindgen.toml",
            "--crate",
            "jian-engine-ffi",
            "--metadata",
        ])
        .arg(&metadata)
        .arg("--output")
        .arg(&generated)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", binary.display()));
    let _ = fs::remove_file(&metadata);
    assert!(
        output.status.success(),
        "cbindgen failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let expected = fs::read(&checked_in)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", checked_in.display()));
    let actual = fs::read(&generated)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", generated.display()));
    let _ = fs::remove_file(&generated);

    assert!(
        expected == actual,
        "jian.h drifted at byte {} (checked-in {} bytes, generated {} bytes); regenerate with `{}`",
        first_difference(&expected, &actual),
        expected.len(),
        actual.len(),
        "~/.cargo/bin/cbindgen --config crates/jian-engine-ffi/cbindgen.toml --crate jian-engine-ffi --output crates/jian-engine-ffi/include/jian.h"
    );
}
