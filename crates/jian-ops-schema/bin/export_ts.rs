//! Emit bindings/ops.ts by recursively exporting PenDocument and its deps.

use jian_ops_schema::document::PenDocument;
use ts_rs::TS;

fn main() {
    // Force ts-rs to resolve `export_to` paths relative to this crate's
    // `bindings/` folder regardless of cwd.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let target_dir = format!("{manifest_dir}/bindings");
    // Safety: no other threads, setting process env at startup.
    unsafe {
        std::env::set_var("TS_RS_EXPORT_DIR", &target_dir);
    }

    PenDocument::export_all().expect("ts-rs export failed");
    // ts-rs emits a space before some documentation-driven line breaks.
    // Normalize generated output so rerunning the exporter cannot make
    // `git diff --check` fail.
    let output = std::path::Path::new(&target_dir).join("ops.ts");
    let generated = std::fs::read_to_string(&output).expect("read generated TypeScript");
    let mut normalized = generated
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    if generated.ends_with('\n') {
        normalized.push('\n');
    }
    std::fs::write(&output, normalized).expect("normalize generated TypeScript");
    eprintln!("TS bindings written under {target_dir}");
}
