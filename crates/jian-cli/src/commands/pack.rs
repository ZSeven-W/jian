//! `jian pack INPUT OUTPUT [--include-fonts] [--include-images] [--aot]
//! [--aot-viewport WxH]` — bundle a `.op` into a `.op.pack` zip.
//!
//! MVP manifest schema (written as `manifest.json` inside the zip):
//!
//! ```json
//! {
//!   "format": "op.pack",
//!   "version": "0.1",
//!   "app":  { "id": "...", "name": "...", "version": "..." },
//!   "capabilities": ["network", "storage"],
//!   "entries": ["app.op", "assets/fonts/Inter.ttf", "aot/initial_layout.bin", ...],
//!   "images": { "cat.png": "assets/images/<blake3hex>.png" },     // only if --include-images
//!   "aot": {                                                       // only if --aot
//!     "initial_layout": "aot/initial_layout.bin",
//!     "default_viewport": { "width": 800.0, "height": 600.0 }
//!   }
//! }
//! ```
//!
//! Asset layout (Plan 9 §Task 3 + Plan 19 §C19 D1):
//! - **Fonts** (`--include-fonts`): scans `<input>/../assets/fonts/` for
//!   `.ttf`/`.otf`/`.woff`/`.woff2`, stores them at `assets/fonts/<filename>`
//!   verbatim. Filenames carry the family-naming convention.
//! - **Images** (`--include-images`): scans `<input>/../assets/images/` for
//!   `.png`/`.jpg`/`.jpeg`/`.webp`/`.gif`/`.svg`, stores them at
//!   `assets/images/<blake3-16-hex>.<ext>` — content-addressed so two
//!   files with identical bytes collapse into one entry. The manifest's
//!   `images` map records each original filename → hashed path so a
//!   loader can rewrite `image.src` references when loading the pack.
//! - **AOT initial layout** (`--aot`, default viewport `800x600` via
//!   `--aot-viewport WxH`): builds a `Runtime`, runs `build_layout`,
//!   serialises every node's scene-coord rect via the
//!   [`jian_ops_schema::pack::InitialLayoutSnapshot`] little-endian
//!   format and embeds it as `aot/initial_layout.bin`. A future
//!   `BootstrapSource::Pack` reader preloads the rects to skip
//!   `ComputeFirstLayout` at runtime (~30-80 ms on real docs).
//!
//! Logic-module bundling (`logic/<id>.wasm`) and AOT expressions
//! (`aot/expressions.bin`) are Plan-19 follow-ups. AOT default-state
//! (`aot/default_state.bin`) ships under `--aot` alongside the layout
//! snapshot — both are dumped from the same probe runtime so the
//! state values reflect the schema-default seed at pack time.

use crate::PackArgs;
use anyhow::{anyhow, Context, Result};
use jian_ops_schema::document::PenDocument;
use jian_ops_schema::pack::{
    DefaultStateSnapshot, InitialLayoutSnapshot, PackedRect, ENTRY_AOT_DEFAULT_STATE,
    ENTRY_AOT_INITIAL_LAYOUT,
};
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

    // AOT initial-layout snapshot + default-state snapshot (Plan 19
    // D1 / Task 6). Computed before opening the zip so a layout
    // error fails fast with a clear message instead of a half-written
    // archive. Both come from the SAME probe runtime so the rect set
    // and the seeded state agree on which document was hashed —
    // walking the schema twice would race a future loader change.
    let aot_payload: Option<(
        InitialLayoutSnapshot,
        Vec<u8>,
        DefaultStateSnapshot,
        Vec<u8>,
    )> = if args.aot {
        let viewport = parse_viewport(&args.aot_viewport)?;
        let (layout_snap, state_snap) =
            compute_aot_payload(&loaded.value, viewport).context(
                "computing AOT initial layout / default state (jian pack --aot). \
                 Falls back when ComputeFirstLayout fails",
            )?;
        let layout_bytes = layout_snap
            .write_bytes()
            .map_err(|e| anyhow!("encode AOT initial layout: {e}"))?;
        let state_bytes = state_snap
            .write_bytes()
            .map_err(|e| anyhow!("encode AOT default state: {e}"))?;
        Some((layout_snap, layout_bytes, state_snap, state_bytes))
    } else {
        None
    };
    // Backwards-compat alias — the rest of the function reads the
    // layout pair directly.
    let aot_layout: Option<(InitialLayoutSnapshot, Vec<u8>)> = aot_payload
        .as_ref()
        .map(|(s, b, _, _)| (s.clone(), b.clone()));

    let mut entries: Vec<String> = vec!["app.op".into()];
    let mut seen_entries: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for asset in fonts.iter().chain(images.iter()) {
        if seen_entries.insert(asset.zip_path.clone()) {
            entries.push(asset.zip_path.clone());
        }
    }
    if aot_layout.is_some() {
        entries.push(ENTRY_AOT_INITIAL_LAYOUT.to_owned());
        entries.push(ENTRY_AOT_DEFAULT_STATE.to_owned());
    }

    let images_manifest: BTreeMap<String, String> = images
        .iter()
        .map(|i| (i.original.clone(), i.zip_path.clone()))
        .collect();

    let manifest = build_manifest(
        &loaded.value,
        &entries,
        &images_manifest,
        aot_layout.as_ref().map(|(s, _)| s.viewport),
    );

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

    if let Some((_, layout_bytes, _, state_bytes)) = aot_payload.as_ref() {
        zw.start_file(ENTRY_AOT_INITIAL_LAYOUT, opts)?;
        zw.write_all(layout_bytes)?;
        zw.start_file(ENTRY_AOT_DEFAULT_STATE, opts)?;
        zw.write_all(state_bytes)?;
    }

    zw.finish()?;

    let aot_msg = match aot_payload.as_ref() {
        Some((s, layout_bytes, state, state_bytes)) => format!(
            ", AOT layout {}×{} ({} rect(s), {} bytes), AOT state ({} app key(s), {} bytes)",
            s.viewport.width as i32,
            s.viewport.height as i32,
            s.rects.len(),
            layout_bytes.len(),
            state.app.len(),
            state_bytes.len(),
        ),
        None => String::new(),
    };
    println!(
        "jian pack: wrote {} ({} bytes app.op, {} font(s), {} image(s){})",
        args.output.display(),
        src.len(),
        fonts.len(),
        images.len(),
        aot_msg,
    );
    Ok(ExitCode::SUCCESS)
}

/// Parse a `WxH` viewport string (e.g. `800x600`) into `(f32, f32)`.
/// Both axes must be positive finite numbers; mirrors the player /
/// dev `--size` parsers' style.
fn parse_viewport(s: &str) -> Result<(f32, f32)> {
    let (w_s, h_s) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow!("expected WxH form (got `{s}`)"))?;
    let w: f32 = w_s
        .trim()
        .parse()
        .map_err(|e| anyhow!("invalid width in `{s}`: {e}"))?;
    let h: f32 = h_s
        .trim()
        .parse()
        .map_err(|e| anyhow!("invalid height in `{s}`: {e}"))?;
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return Err(anyhow!("viewport `{s}` must have positive finite dims"));
    }
    Ok((w, h))
}

/// Build a runtime, run `build_layout(viewport)`, then walk every
/// node and collect its scene-coord rect into an
/// [`InitialLayoutSnapshot`] alongside a snapshot of every state
/// scope's seeded values. Nodes without a layout rect (very rare —
/// typically a virtual `<ref>` placeholder the layout engine
/// omitted) are silently skipped; the host falls back to a fresh
/// layout pass for them.
///
/// First validates that every node id in the document is unique —
/// `NodeTree::insert_subtree` overwrites duplicates silently, which
/// would produce an incomplete AOT snapshot (codex round 3 MEDIUM).
///
/// The state snapshot reflects the schema-default seed at the moment
/// `Runtime::new_from_document` finishes; no events have fired and
/// no signals have been mutated, so the dump is exactly what
/// `SeedStateGraph` would otherwise reproduce. Capturing both
/// payloads from the same runtime keeps the layout↔state pair
/// internally consistent.
fn compute_aot_payload(
    doc: &PenDocument,
    viewport: (f32, f32),
) -> Result<(InitialLayoutSnapshot, DefaultStateSnapshot)> {
    if let Some(dup) = first_duplicate_node_id(doc) {
        return Err(anyhow!(
            "AOT initial layout: document has duplicate node id `{dup}`. \
             Fix: ensure every node's `id` is unique before `jian pack --aot`",
        ));
    }
    let mut rt = jian_core::Runtime::new_from_document(doc.clone())
        .map_err(|e| anyhow!("Runtime::new_from_document: {e}"))?;
    rt.build_layout(viewport)
        .map_err(|e| anyhow!("build_layout({:?}): {e}", viewport))?;
    let tree = rt
        .document
        .as_ref()
        .ok_or_else(|| anyhow!("runtime has no document after construction"))?
        .tree
        .by_id
        .clone();
    let mut rects = BTreeMap::new();
    for (id, key) in tree {
        if let Some(rect) = rt.layout.node_rect(key) {
            rects.insert(
                id,
                PackedRect::from_xywh((
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                )),
            );
        }
    }
    let layout = InitialLayoutSnapshot {
        viewport: jian_ops_schema::pack::DefaultViewport {
            width: viewport.0,
            height: viewport.1,
        },
        rects,
    };
    let state = rt.state.dump_default_state();
    Ok((layout, state))
}

/// Walk the **first-frame root set** of `doc` and return the first
/// node id that appears twice, or `None` if every id is unique.
///
/// The walker mirrors `jian_core::document::loader`'s root-selection
/// logic: when the doc declares non-empty `pages`, only `pages[0]
/// .children` are surfaced into the runtime `NodeTree`; otherwise
/// `doc.children` is the active root set. Walking only that set keeps
/// the duplicate-id check aligned with what the AOT snapshot will
/// actually contain — a duplicate buried in an inactive page can't
/// poison the snapshot, so rejecting on it would be a spurious error
/// (codex round 5 MEDIUM).
///
/// The walker is typed (recurses through known container fields
/// only) so raw `serde_json::Value` payloads on event actions or
/// `Ref::descendants` can't false-positive a duplicate id (codex
/// round 4 MEDIUM).
fn first_duplicate_node_id(doc: &PenDocument) -> Option<String> {
    let roots: &[jian_ops_schema::node::PenNode] = match (&doc.pages, &doc.children) {
        (Some(pages), _) if !pages.is_empty() => &pages[0].children,
        _ => doc.children.as_slice(),
    };
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for node in roots {
        if let Some(dup) = walk_node_ids_typed(node, &mut seen) {
            return Some(dup);
        }
    }
    None
}

fn walk_node_ids_typed(
    node: &jian_ops_schema::node::PenNode,
    seen: &mut std::collections::BTreeSet<String>,
) -> Option<String> {
    use jian_ops_schema::node::PenNode;
    let id = jian_core::document::tree::node_schema_id(node);
    if !seen.insert(id.to_owned()) {
        return Some(id.to_owned());
    }
    let children: Option<&Vec<PenNode>> = match node {
        PenNode::Frame(x) => x.children.as_ref(),
        PenNode::Group(x) => x.children.as_ref(),
        PenNode::Rectangle(x) => x.children.as_ref(),
        PenNode::Ref(x) => x.children.as_ref(),
        // Leaf nodes (Text / TextInput / Image / IconFont / Path /
        // Line / Ellipse / Polygon) have no descendant `PenNode`s.
        // Their event-handler payloads + `Ref::descendants` overrides
        // are raw JSON and intentionally skipped here.
        _ => None,
    };
    if let Some(children) = children {
        for c in children {
            if let Some(dup) = walk_node_ids_typed(c, seen) {
                return Some(dup);
            }
        }
    }
    None
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

/// Render the first 16 bytes (128 bits) of a digest as hex. 64-bit
/// truncation (the original 8-byte form) gave only ~2³² randomness
/// before a birthday collision and could silently dedup distinct
/// images into the same archive entry; 128 bits comfortably outruns
/// any pack's image count. Output length is fixed at 32 hex chars.
fn hex_first16(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(32);
    for b in &bytes[..16] {
        write!(s, "{:02x}", b).unwrap();
    }
    s
}

fn build_manifest(
    doc: &PenDocument,
    entries: &[String],
    images: &BTreeMap<String, String>,
    aot_viewport: Option<jian_ops_schema::pack::DefaultViewport>,
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
    if let Some(vp) = aot_viewport {
        // Manifest's `aot` field mirrors `pack::manifest::AotInventory`'s
        // wire shape — just the fields actually populated by --aot.
        // Switching to the typed `AotManifest` is a separate refactor
        // tracked in `pack/mod.rs`'s module doc.
        //
        // `measurement_backend` records which `MeasureBackend` baked
        // the rects (codex round 3 MEDIUM): a host using `SkiaMeasure`
        // (jian-skia's `textlayout` feature) computes different text
        // widths than the `EstimateBackend` heuristic. The AOT writer
        // uses the heuristic; a future runtime preload reader MUST
        // reject (fall back to fresh layout) when its own backend tag
        // doesn't match the manifest's. Plan 19 §C19 D1.
        m.as_object_mut().expect("json! produces object").insert(
            "aot".into(),
            serde_json::json!({
                "initial_layout": ENTRY_AOT_INITIAL_LAYOUT,
                "default_state": ENTRY_AOT_DEFAULT_STATE,
                "default_viewport": { "width": vp.width, "height": vp.height },
                "measurement_backend": AOT_MEASUREMENT_BACKEND,
            }),
        );
    }
    m
}

/// Identifier for the [`jian_core::layout::measure::MeasureBackend`]
/// the writer used. Today only the default `EstimateBackend` exists
/// inside this crate's dependency tree (the Skia-shaping
/// `SkiaMeasure` lives in `jian-skia` under the `textlayout` feature).
/// A future writer that opts into a different backend must coin a new
/// tag here so a reader can reject a mismatched preload.
const AOT_MEASUREMENT_BACKEND: &str = "estimate";

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
        assert_eq!(hex_first16(&zeros), "0".repeat(32));
        assert_eq!(hex_first16(&zeros).len(), 32);
        let mut bytes = [0u8; 32];
        bytes[0] = 0x0a;
        bytes[15] = 0xff;
        assert_eq!(hex_first16(&bytes), "0a0000000000000000000000000000ff");
    }
}
