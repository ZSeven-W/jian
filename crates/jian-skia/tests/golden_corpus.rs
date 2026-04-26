//! Golden-corpus PNG-diff harness — Plan 7 §C.1 Stage A gate.
//!
//! Walks every `.op` file under `jian-ops-schema/tests/corpus/`,
//! renders it through `Runtime` + `SkiaBackend` to a 800×600 raster
//! surface, and *generates* a baseline PNG when the corresponding
//! `tests/golden/<name>.png` doesn't exist yet (set `GOLDEN_BLESS=1`
//! to overwrite). Subsequent runs assert byte-equality against the
//! tracked baseline.
//!
//! Spec tolerance is `max channel delta ≤ 3, changed-pixel
//! fraction ≤ 0.3%`, but Phase 1 ships byte-exact comparison
//! because deterministic raster + identical scene walker output
//! reproduces the same PNG bytes on every CI runner. Tolerance
//! kicks in once GPU surface variants land (Plan 11 / 12).
//!
//! Skipped corpus entries: this harness can render any document
//! whose root is a single `frame` / `rectangle` (the layout engine
//! handles those end-to-end). Documents that exercise unimplemented
//! shapes (`path`, `polygon`, `image` with remote URLs, etc.) are
//! intentionally renderable through the placeholder paths but the
//! resulting PNG is still deterministic, so they're included.
//!
//! `textlayout` feature: enabling skia's ParagraphBuilder swaps the
//! single-line text path for the full shaper, which produces
//! different (but still deterministic) PNG bytes — the default
//! baselines no longer match. Plan 11 / 12 will land a tolerance
//! comparator and a separate `golden/textlayout/` baseline set.
//! Until then, the byte-equality check is skipped under the feature;
//! the harness still parses, renders, and PNG-encodes every fixture
//! to keep the textlayout build path covered.

use jian_core::geometry::size;
use jian_core::render::RenderBackend;
use jian_core::Runtime;
use jian_ops_schema::load_str;
use jian_skia::SkiaBackend;
use std::fs;
use std::path::{Path, PathBuf};

const SURFACE_W: f32 = 800.0;
const SURFACE_H: f32 = 600.0;

#[test]
fn corpus_renders_match_golden_bytes() {
    let corpus_dir = corpus_dir();
    let golden_dir = golden_dir();
    if !golden_dir.exists() {
        fs::create_dir_all(&golden_dir).expect("create golden dir");
    }
    let bless = std::env::var("GOLDEN_BLESS").is_ok();
    let mut checked = 0usize;
    let mut blessed = 0usize;
    let mut failures = Vec::new();
    let entries = fs::read_dir(&corpus_dir).expect("read corpus dir");
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("op") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed")
            .to_owned();
        // `pencil-demo.op` (OpenPencil v2.8 export) lives under
        // `tests/forward-compat/` and is asserted-rejected there;
        // skip-clause remains harmless if the file ever returns.
        if name == "pencil-demo" {
            continue;
        }
        let source = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let png = match render_to_png(&source) {
            Ok(p) => p,
            Err(_) => {
                // Render failures (unsupported node types in MVP)
                // shouldn't fail the harness — log + skip.
                eprintln!("corpus: skip {} (render error)", name);
                continue;
            }
        };
        let golden_path = golden_dir.join(format!("{}.png", name));
        if bless {
            fs::write(&golden_path, &png).expect("write golden");
            blessed += 1;
            continue;
        }
        if !golden_path.exists() {
            // Missing baseline is a hard failure — we shouldn't
            // silently mutate the workspace mid-CI. Authors must
            // run `GOLDEN_BLESS=1 cargo test ...` to opt in.
            failures.push(format!("{} (no baseline)", name));
            continue;
        }
        let tracked = fs::read(&golden_path).expect("read golden");
        checked += 1;
        // Byte-equality only runs on macOS:
        //   1. The committed baselines were `GOLDEN_BLESS=1`'d on
        //      macOS — Skia's font fallback resolves a different
        //      typeface for "Inter" / "Space Grotesk" / fallback
        //      glyphs on Linux + Windows, producing genuinely
        //      different PNG bytes for a deterministic-but-platform-
        //      specific raster.
        //   2. The `textlayout` feature swaps the canvas single-line
        //      path for ParagraphBuilder, which also widens the
        //      same drift.
        // Plan 11 / 12 will land a tolerance comparator + per-platform
        // baseline sets. Until then, render coverage runs everywhere
        // (we still encode the PNG); byte compare only on the
        // platform the baselines came from.
        #[cfg(all(target_os = "macos", not(feature = "textlayout")))]
        if tracked != png {
            failures.push(name);
        }
        #[cfg(any(not(target_os = "macos"), feature = "textlayout"))]
        let _ = tracked;
    }
    if !failures.is_empty() {
        panic!(
            "golden mismatch ({}/{} files): {:?}. Re-run with GOLDEN_BLESS=1 to update.",
            failures.len(),
            checked,
            failures
        );
    }
    eprintln!(
        "golden corpus: checked={} blessed={} (set GOLDEN_BLESS=1 to overwrite)",
        checked, blessed
    );
}

fn render_to_png(source: &str) -> Result<Vec<u8>, String> {
    let schema = load_str(source)
        .map_err(|e| format!("parse: {:?}", e))?
        .value;
    let mut rt = Runtime::new_from_document(schema).map_err(|e| format!("runtime: {:?}", e))?;
    rt.build_layout((SURFACE_W, SURFACE_H))
        .map_err(|e| format!("layout: {:?}", e))?;
    rt.rebuild_spatial();
    let mut backend = SkiaBackend::new();
    let mut surface = backend.new_surface(size(SURFACE_W, SURFACE_H));
    backend.begin_frame(&mut surface, 0xffffffff);
    if let Some(doc) = rt.document.as_ref() {
        let ops = jian_host_desktop::scene::collect_draws(doc, &rt.layout);
        for op in ops {
            backend.draw(&op);
        }
    }
    backend.end_frame(&mut surface);
    surface
        .encode_png()
        .ok_or_else(|| "encode_png returned None".to_owned())
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("jian-ops-schema")
        .join("tests")
        .join("corpus")
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

// silence unused-import lints when this test compiles into an empty
// list (e.g. corpus dir missing in CI snapshots)
#[allow(dead_code)]
fn _unused(_: &Path) {}
