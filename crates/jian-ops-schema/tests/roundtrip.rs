use jian_ops_schema::document::PenDocument;
use std::fs;
use std::path::PathBuf;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

fn load(name: &str) -> String {
    let p = corpus_dir().join(name);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {}", p.display(), e))
}

fn assert_roundtrip(name: &str) {
    let raw = load(name);
    let doc: PenDocument =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {} failed: {}", name, e));
    let serialized = serde_json::to_string(&doc).unwrap();
    let doc2: PenDocument = serde_json::from_str(&serialized)
        .unwrap_or_else(|e| panic!("re-parse {} failed: {}", name, e));
    assert_eq!(doc, doc2, "roundtrip mismatch for {}", name);
}

#[test]
fn minimal() {
    assert_roundtrip("minimal.op");
}
#[test]
fn rectangle() {
    assert_roundtrip("rectangle.op");
}
#[test]
fn nested_frame() {
    assert_roundtrip("nested-frame.op");
}
#[test]
fn with_variables() {
    assert_roundtrip("with-variables.op");
}
#[test]
fn pages() {
    assert_roundtrip("pages.op");
}
// `pencil-demo.op` (OpenPencil v2.8 export) lives under
// `tests/forward-compat/` now — `forward_compat.rs` asserts the
// loader rejects v2 documents with `UnsupportedFormatVersion`.
#[test]
fn full_jian_extensions() {
    assert_roundtrip("full-jian-extensions.op");
}
