use jian_ops_schema::document::PenDocument;
use schemars::schema_for;
use std::path::PathBuf;

fn main() {
    let schema = schema_for!(PenDocument);
    let pretty = serde_json::to_string_pretty(&schema).expect("serialize schema");

    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("bindings")
        .join("ops.schema.json");
    std::fs::create_dir_all(target.parent().unwrap()).expect("mkdir");
    std::fs::write(&target, &pretty).expect("write schema");

    eprintln!("Wrote {} ({} bytes)", target.display(), pretty.len());
}
