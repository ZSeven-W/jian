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
    eprintln!("TS bindings written under {target_dir}");
}
