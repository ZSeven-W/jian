//! `jian pack INPUT OUTPUT` — bundle a `.op` into a `.op.pack` zip.
//!
//! MVP manifest schema (written as `manifest.json` inside the zip):
//!
//! ```json
//! {
//!   "format": "op.pack",
//!   "version": "0.1",
//!   "app":  { "id": "...", "name": "...", "version": "..." },
//!   "capabilities": ["network", "storage"],
//!   "entries": ["app.op"]
//! }
//! ```
//!
//! The original JSON is stored verbatim as `app.op`. Assets (fonts,
//! images) arrive in a later plan — the zip format supports them
//! already, this just doesn't bundle them yet.

use crate::PackArgs;
use anyhow::{Context, Result};
use jian_ops_schema::document::PenDocument;
use std::fs::File;
use std::io::Write;
use std::process::ExitCode;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

pub fn run(args: PackArgs) -> Result<ExitCode> {
    let src = std::fs::read_to_string(&args.input)
        .with_context(|| format!("read {}", args.input.display()))?;
    let loaded = jian_ops_schema::load_str(&src)
        .with_context(|| format!("parse {}", args.input.display()))?;
    let manifest = build_manifest(&loaded.value);

    let file =
        File::create(&args.output).with_context(|| format!("create {}", args.output.display()))?;
    let mut zw = zip::ZipWriter::new(file);
    let opts: SimpleFileOptions =
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zw.start_file("manifest.json", opts)?;
    zw.write_all(serde_json::to_vec_pretty(&manifest)?.as_slice())?;

    zw.start_file("app.op", opts)?;
    zw.write_all(src.as_bytes())?;

    zw.finish()?;

    println!(
        "jian pack: wrote {} ({} bytes app.op)",
        args.output.display(),
        src.len()
    );
    Ok(ExitCode::SUCCESS)
}

fn build_manifest(doc: &PenDocument) -> serde_json::Value {
    let app = doc.app.as_ref();
    let caps: Vec<String> = app
        .and_then(|a| a.capabilities.as_ref())
        .map(|cs| cs.iter().map(|c| capability_str(c).to_owned()).collect())
        .unwrap_or_default();
    serde_json::json!({
        "format": "op.pack",
        "version": "0.1",
        "app": {
            "id": app.map(|a| a.id.as_str()).unwrap_or(""),
            "name": app.map(|a| a.name.as_str()).unwrap_or(""),
            "version": app.map(|a| a.version.as_str()).unwrap_or(""),
        },
        "capabilities": caps,
        "entries": ["app.op"],
    })
}

fn capability_str(c: &jian_ops_schema::app::Capability) -> &'static str {
    use jian_ops_schema::app::Capability::*;
    match c {
        Storage => "storage",
        Network => "network",
        Camera => "camera",
        Microphone => "microphone",
        Location => "location",
        Notifications => "notifications",
        Clipboard => "clipboard",
        Biometric => "biometric",
        FileSystem => "file_system",
        Haptic => "haptic",
    }
}
