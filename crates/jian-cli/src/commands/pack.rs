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
//! Logic-module bundling (`logic/<id>.wasm`) is a Plan-19 follow-up.
//! AOT default-state (`aot/default_state.bin`) and AOT expressions
//! (`aot/expressions.bin`) ship under `--aot` alongside the layout
//! snapshot — all three are captured from the same probe runtime so
//! state values, layout rects, and the compiled-expression cache
//! reflect a single coherent schema-default seed at pack time.
//!
//! ### Expression coverage
//!
//! `aot/expressions.bin` carries every Tier-1 expression source the
//! schema declares: a static `serde_json::Value` walk via
//! `jian_core::expression::warm_cache_from_document` tries to
//! compile every string-typed leaf and inserts successful chunks
//! into the cache. Event-handler action expressions, bindings,
//! template literals, `NumberOrExpression` / `BoolOrExpression`
//! union strings — all flow through the same gate. A post-compile
//! filter drops bare-identifier chunks (single `PushScopeRef(s)` +
//! `Return` where `s` doesn't start with `$`) so node-id /
//! enum-value pollution stays out of the snapshot. Parse failures
//! drop silently per `cache::compile_error_not_cached`.

use crate::PackArgs;
use anyhow::{anyhow, Context, Result};
use jian_ops_schema::document::PenDocument;
use jian_ops_schema::pack::{
    DefaultStateSnapshot, ExpressionsSnapshot, InitialLayoutSnapshot, PackedRect,
    ENTRY_AOT_DEFAULT_STATE, ENTRY_AOT_EXPRESSIONS, ENTRY_AOT_INITIAL_LAYOUT,
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
    let mut loaded = jian_ops_schema::load_str(&src)
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

    // AOT initial-layout snapshot + default-state snapshot + compiled-
    // expression snapshot (Plan 19 D1+D2 / Task 6). Computed before
    // opening the zip so an error fails fast with a clear message
    // instead of a half-written archive. All three come from the SAME
    // probe runtime so the rect set, the seeded state, and the
    // compiled-expression cache agree on which document was hashed —
    // walking the schema twice would race a future loader change.
    let skip_responsive_aot = args.aot && loaded.value.is_responsive();
    if skip_responsive_aot {
        eprintln!("jian pack: warning: responsive documents skip AOT stages");
    }
    let aot_payload: Option<AotPayload> = if args.aot && !skip_responsive_aot {
        let viewport = parse_viewport(&args.aot_viewport)?;
        let (layout_snap, state_snap, exprs_snap) = compute_aot_payload(&loaded.value, viewport)
            .context(
                "computing AOT initial layout / default state / expressions (jian pack --aot). \
                 Falls back when ComputeFirstLayout fails",
            )?;
        let layout_bytes = layout_snap
            .write_bytes()
            .map_err(|e| anyhow!("encode AOT initial layout: {e}"))?;
        let state_bytes = state_snap
            .write_bytes()
            .map_err(|e| anyhow!("encode AOT default state: {e}"))?;
        let exprs_bytes = exprs_snap
            .write_bytes()
            .map_err(|e| anyhow!("encode AOT expressions: {e}"))?;
        Some(AotPayload {
            layout_snap,
            layout_bytes,
            state_snap,
            state_bytes,
            exprs_snap,
            exprs_bytes,
        })
    } else {
        None
    };

    let mut entries: Vec<String> = vec!["app.op".into()];
    let mut seen_entries: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for asset in fonts.iter().chain(images.iter()) {
        if seen_entries.insert(asset.zip_path.clone()) {
            entries.push(asset.zip_path.clone());
        }
    }
    if aot_payload.is_some() {
        entries.push(ENTRY_AOT_INITIAL_LAYOUT.to_owned());
        entries.push(ENTRY_AOT_DEFAULT_STATE.to_owned());
        entries.push(ENTRY_AOT_EXPRESSIONS.to_owned());
    }

    let images_manifest: BTreeMap<String, String> = images
        .iter()
        .map(|i| (i.original.clone(), i.zip_path.clone()))
        .collect();

    let manifest = build_manifest(
        &loaded.value,
        &entries,
        &images_manifest,
        aot_payload.as_ref().map(|p| p.layout_snap.viewport),
    );

    // design.md is editor-only metadata — strip it from the packaged
    // `app.op` so the runtime carries no design brief. When the field
    // is absent we ship the raw source verbatim to preserve formatting.
    let app_op_bytes: Vec<u8> = if loaded.value.design_md.take().is_some() {
        serde_json::to_vec_pretty(&loaded.value)
            .context("re-serialize app.op after stripping designMd")?
    } else {
        src.as_bytes().to_vec()
    };

    let file =
        File::create(&args.output).with_context(|| format!("create {}", args.output.display()))?;
    let mut zw = zip::ZipWriter::new(file);
    let opts: SimpleFileOptions =
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zw.start_file("manifest.json", opts)?;
    zw.write_all(serde_json::to_vec_pretty(&manifest)?.as_slice())?;

    zw.start_file("app.op", opts)?;
    zw.write_all(&app_op_bytes)?;

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

    if let Some(p) = aot_payload.as_ref() {
        zw.start_file(ENTRY_AOT_INITIAL_LAYOUT, opts)?;
        zw.write_all(&p.layout_bytes)?;
        zw.start_file(ENTRY_AOT_DEFAULT_STATE, opts)?;
        zw.write_all(&p.state_bytes)?;
        zw.start_file(ENTRY_AOT_EXPRESSIONS, opts)?;
        zw.write_all(&p.exprs_bytes)?;
    }

    zw.finish()?;

    let aot_msg = match aot_payload.as_ref() {
        Some(p) => format!(
            ", AOT layout {}×{} ({} rect(s), {} bytes), AOT state ({} app key(s), {} bytes), AOT exprs ({} cached, {} bytes)",
            p.layout_snap.viewport.width as i32,
            p.layout_snap.viewport.height as i32,
            p.layout_snap.rects.len(),
            p.layout_bytes.len(),
            p.state_snap.app.len(),
            p.state_bytes.len(),
            p.exprs_snap.len(),
            p.exprs_bytes.len(),
        ),
        None => String::new(),
    };
    println!(
        "jian pack: wrote {} ({} bytes app.op, {} font(s), {} image(s){})",
        args.output.display(),
        app_op_bytes.len(),
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

/// All-in-one AOT payload assembled in `pack --aot`. Holding the
/// snapshots and their already-encoded byte buffers together keeps
/// the writer pass linear: validate → encode → emit.
struct AotPayload {
    layout_snap: InitialLayoutSnapshot,
    layout_bytes: Vec<u8>,
    state_snap: DefaultStateSnapshot,
    state_bytes: Vec<u8>,
    exprs_snap: ExpressionsSnapshot,
    exprs_bytes: Vec<u8>,
}

/// Build a runtime, run `build_layout(viewport)`, then walk every
/// node and collect its scene-coord rect into an
/// [`InitialLayoutSnapshot`] alongside a snapshot of every state
/// scope's seeded values and the expression cache the layout pass
/// populated. Nodes without a layout rect (very rare — typically a
/// virtual `<ref>` placeholder the layout engine omitted) are
/// silently skipped; the host falls back to a fresh layout pass for
/// them.
///
/// First validates that every node id in the document is unique —
/// `NodeTree::insert_subtree` overwrites duplicates silently, which
/// would produce an incomplete AOT snapshot (codex round 3 MEDIUM).
///
/// The state snapshot reflects the schema-default seed at the moment
/// `Runtime::new_from_document` finishes; no events have fired and
/// no signals have been mutated, so the dump is exactly what
/// `SeedStateGraph` would otherwise reproduce. Capturing all three
/// payloads from the same runtime keeps the layout ↔ state ↔
/// expression-cache trio internally consistent.
///
/// The expression snapshot reflects every expression source the
/// schema declares: a static `serde_json::Value` walk over the
/// doc (`expression::warm_cache_from_document`) tries to compile
/// every string-typed leaf via `cache.get_or_compile`, plus
/// `Runtime::warm_expression_cache` covers the (today empty)
/// `DeferredBindingQueue`. Parse failures (text content, color
/// names, layout enums) drop silently per
/// `cache::compile_error_not_cached`. The per-source dedup is
/// automatic — `ExpressionCache::get_or_compile` stores at most
/// one chunk per source, so the dump is naturally minimal.
fn compute_aot_payload(
    doc: &PenDocument,
    viewport: (f32, f32),
) -> Result<(
    InitialLayoutSnapshot,
    DefaultStateSnapshot,
    ExpressionsSnapshot,
)> {
    if let Some(dup) = first_duplicate_node_id(doc) {
        return Err(anyhow!(
            "AOT initial layout: document has duplicate node id `{dup}`. \
             Fix: ensure every node's `id` is unique before `jian pack --aot`",
        ));
    }
    let mut rt = jian_core::Runtime::new_from_document_with_viewport(doc.clone(), viewport)
        .map_err(|e| anyhow!("Runtime::new_from_document_with_viewport: {e}"))?;
    rt.build_layout(viewport)
        .map_err(|e| anyhow!("build_layout({:?}): {e}", viewport))?;
    // Plan 19 D2: pre-compile every queued binding source AND
    // every static expression source the doc-walk extractor can
    // reach. `warm_expression_cache` covers `DeferredBindingQueue`
    // (today usually empty — loader doesn't populate it yet); the
    // walker covers bindings / NumberOrExpression / BoolOrExpression
    // / EventHandler action bodies / template literals statically.
    // Together they pull every realistic expression source the
    // schema declares into the cache before `cache.dump()` for the
    // AOT snapshot. Parse failures drop silently per
    // `ExpressionCache::compile_error_not_cached`'s contract.
    let _warmed = rt.warm_expression_cache();
    let _walked = jian_core::expression::warm_cache_from_document(doc, &rt.expr_cache);
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
    // Plan 19 D2: convert the runtime's BTreeMap<String, Chunk>
    // dump to the wire-stable `ExpressionsSnapshot` shape. The
    // helper is total — every Chunk has a 1:1 PackedChunk mirror.
    let exprs = jian_core::expression::chunks_to_snapshot(&rt.expr_cache.dump());
    Ok((layout, state, exprs))
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
                "expressions": ENTRY_AOT_EXPRESSIONS,
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
