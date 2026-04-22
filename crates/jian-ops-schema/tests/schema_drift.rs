//! Guard against `ops.schema.json` drifting out of sync with the Rust types.
//! If this test fails, run:
//!   cargo run -p jian-ops-schema --bin export_schema
//! and commit the updated bindings/ops.schema.json.

use jian_ops_schema::document::PenDocument;
use schemars::schema_for;
use std::fs;
use std::path::PathBuf;

#[test]
fn schema_is_up_to_date() {
    let current = schema_for!(PenDocument);
    let current_json = serde_json::to_string_pretty(&current).unwrap();

    let tracked_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("bindings")
        .join("ops.schema.json");
    let tracked_raw = fs::read_to_string(&tracked_path)
        .unwrap_or_else(|_| panic!("missing tracked schema file: {}", tracked_path.display()));
    // Normalize line endings — on Windows `core.autocrlf` may rewrite the
    // tracked file into CRLF on checkout, which would fail byte-exact
    // comparison against the LF output of serde_json.
    let tracked = tracked_raw.replace("\r\n", "\n");

    if current_json != tracked {
        let cur_lines: Vec<_> = current_json.lines().collect();
        let tr_lines: Vec<_> = tracked.lines().collect();
        for (i, (c, t)) in cur_lines.iter().zip(tr_lines.iter()).enumerate() {
            if c != t {
                panic!(
                    "schema drift at line {}: current=`{}`, tracked=`{}`",
                    i + 1,
                    c,
                    t
                );
            }
        }
        panic!(
            "schema drift: length mismatch (current={}, tracked={})",
            cur_lines.len(),
            tr_lines.len()
        );
    }
}
