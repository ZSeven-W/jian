//! `jian pack INPUT OUTPUT [--include-fonts] [--include-images]` — bundle a
//! `.op` into a `.op.pack` zip.
//!
//! MVP manifest schema (written as `manifest.json` inside the zip):
//!
//! ```json
//! {
//!   "format": "op.pack",
//!   "version": "0.1",
//!   "app":  { "id": "...", "name": "...", "version": "..." },
//!   "capabilities": ["network", "storage"],
//!   "entries": ["app.op", "assets/fonts/Inter.ttf", ...],
//!   "images": { "cat.png": "assets/images/<blake3hex>.png" }   // only if --include-images
//! }
//! ```
//!
//! Asset layout (Plan 9 §Task 3):
//! - **Fonts** (`--include-fonts`): scans `<input>/../assets/fonts/` for
//!   `.ttf`/`.otf`/`.woff`/`.woff2`, stores them at `assets/fonts/<filename>`
//!   verbatim. Filenames carry the family-naming convention.
//! - **Images** (`--include-images`): scans `<input>/../assets/images/` for
//!   `.png`/`.jpg`/`.jpeg`/`.webp`/`.gif`/`.svg`, stores them at
//!   `assets/images/<blake3-16-hex>.<ext>` — content-addressed so two
//!   files with identical bytes collapse into one entry. The manifest's
//!   `images` map records each original filename → hashed path so a
//!   loader can rewrite `image.src` references when loading the pack.
//!
//! Logic-module bundling (`logic/<id>.wasm`) is a Plan-19 follow-up.

use crate::PackArgs;
use anyhow::{Context, Result};
use jian_ops_schema::document::PenDocument;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// File extensions accepted under `assets/fonts/` when `--include-fonts` is set.
const FONT_EXTS: &[&str] = &["ttf", "otf", "woff", "woff2"];

/// File extensions accepted under `assets/images/` when `--include-images` is set.
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "svg"];

pub fn run(args: PackArgs) -> Result<ExitCode> {
    let src = std::fs::read_to_string(&args.input)
        .with_context(|| format!("read {}", args.input.display()))?;
    let loaded = jian_ops_schema::load_str(&src)
        .with_context(|| format!("parse {}", args.input.display()))?;

    let parent = args.input.parent().unwrap_or(Path::new("."));
    let fonts = if args.include_fonts {
        collect_fonts(parent)?
    } else {
        Vec::new()
    };
    let images = if args.include_images {
        collect_images(parent)?
    } else {
        Vec::new()
    };

    let mut entries: Vec<String> = vec!["app.op".into()];
    let mut seen_entries: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for asset in fonts.iter().chain(images.iter()) {
        if seen_entries.insert(asset.zip_path.clone()) {
            entries.push(asset.zip_path.clone());
        }
    }

    let images_manifest: BTreeMap<String, String> = images
        .iter()
        .map(|i| (i.original.clone(), i.zip_path.clone()))
        .collect();

    let manifest = build_manifest(&loaded.value, &entries, &images_manifest);

    let file =
        File::create(&args.output).with_context(|| format!("create {}", args.output.display()))?;
    let mut zw = zip::ZipWriter::new(file);
    let opts: SimpleFileOptions =
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zw.start_file("manifest.json", opts)?;
    zw.write_all(serde_json::to_vec_pretty(&manifest)?.as_slice())?;

    zw.start_file("app.op", opts)?;
    zw.write_all(src.as_bytes())?;

    let mut written: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for asset in fonts.iter().chain(images.iter()) {
        // Image dedup leaves duplicate Asset rows with empty bytes pointing
        // at the same zip_path; only the first write actually emits a file.
        if !written.insert(asset.zip_path.clone()) {
            continue;
        }
        zw.start_file(&asset.zip_path, opts)?;
        zw.write_all(&asset.bytes)?;
    }

    zw.finish()?;

    println!(
        "jian pack: wrote {} ({} bytes app.op, {} font(s), {} image(s))",
        args.output.display(),
        src.len(),
        fonts.len(),
        images.len(),
    );
    Ok(ExitCode::SUCCESS)
}

struct Asset {
    /// Filename as found on disk (relative to the source directory).
    original: String,
    /// Path inside the archive (`assets/fonts/...` or `assets/images/...`).
    zip_path: String,
    bytes: Vec<u8>,
}

/// One row from a `read_dir` walk that already passed extension filtering.
struct Candidate {
    path: std::path::PathBuf,
    name: String,
    ext: String,
}

/// Sort `dir` by filename, keep plain files whose lowercase extension is in
/// `exts`. Non-existent dir is a no-op (returns empty). Errors propagate.
fn list_assets(dir: &Path, exts: &[&str]) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut listing: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("scan {}", dir.display()))?;
    listing.sort_by_key(|e| e.file_name());
    for entry in listing {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(OsStr::to_str).map(str::to_owned) else {
            continue;
        };
        let Some(ext) = path
            .extension()
            .and_then(OsStr::to_str)
            .map(|e| e.to_ascii_lowercase())
        else {
            continue;
        };
        if !exts.contains(&ext.as_str()) {
            continue;
        }
        out.push(Candidate { path, name, ext });
    }
    Ok(out)
}

fn collect_fonts(parent: &Path) -> Result<Vec<Asset>> {
    let dir = parent.join("assets").join("fonts");
    let mut out = Vec::new();
    for c in list_assets(&dir, FONT_EXTS)? {
        let bytes = fs::read(&c.path).with_context(|| format!("read {}", c.path.display()))?;
        out.push(Asset {
            zip_path: format!("assets/fonts/{}", c.name),
            original: c.name,
            bytes,
        });
    }
    Ok(out)
}

fn collect_images(parent: &Path) -> Result<Vec<Asset>> {
    let dir = parent.join("assets").join("images");
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for c in list_assets(&dir, IMAGE_EXTS)? {
        let bytes = fs::read(&c.path).with_context(|| format!("read {}", c.path.display()))?;
        let hex = hex_first16(blake3::hash(&bytes).as_bytes());
        let zip_path = format!("assets/images/{}.{}", hex, c.ext);
        // Dedup: same content hash + ext → same zip_path. Skip body bytes
        // for the duplicate but record the original→zip_path mapping so
        // the loader can resolve every reference.
        if seen.insert(zip_path.clone()) {
            out.push(Asset {
                original: c.name,
                zip_path,
                bytes,
            });
        } else {
            out.push(Asset {
                original: c.name,
                zip_path,
                bytes: Vec::new(),
            });
        }
    }
    Ok(out)
}

fn hex_first16(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(16);
    for b in &bytes[..8] {
        write!(s, "{:02x}", b).unwrap();
    }
    s
}

fn build_manifest(
    doc: &PenDocument,
    entries: &[String],
    images: &BTreeMap<String, String>,
) -> serde_json::Value {
    let app = doc.app.as_ref();
    let caps: Vec<String> = app
        .and_then(|a| a.capabilities.as_ref())
        .map(|cs| cs.iter().map(|c| capability_str(c).to_owned()).collect())
        .unwrap_or_default();
    let mut m = serde_json::json!({
        "format": "op.pack",
        "version": "0.1",
        "app": {
            "id": app.map(|a| a.id.as_str()).unwrap_or(""),
            "name": app.map(|a| a.name.as_str()).unwrap_or(""),
            "version": app.map(|a| a.version.as_str()).unwrap_or(""),
        },
        "capabilities": caps,
        "entries": entries,
    });
    if !images.is_empty() {
        m.as_object_mut()
            .expect("json! produces object")
            .insert("images".into(), serde_json::to_value(images).unwrap());
    }
    m
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_first16_padding_and_length() {
        let zeros = [0u8; 32];
        assert_eq!(hex_first16(&zeros), "0000000000000000");
        let mut bytes = [0u8; 32];
        bytes[0] = 0x0a;
        bytes[7] = 0xff;
        assert_eq!(hex_first16(&bytes), "0a000000000000ff");
    }
}
