//! Filesystem PNG loader used by `jian player` / `jian dev` to honour
//! the schema's `app.icon: Option<String>` field and the
//! `--icon <PATH>` CLI override.
//!
//! [`resolve_app_icon`] is the entry point each CLI subcommand calls:
//! given the `.op` file's path, the CLI override, and the parsed
//! schema, it decides which source string to load (override beats
//! schema; if neither is set, returns `None`) and decodes via
//! [`FsIconLoader`]. Decoder failures log to stderr and return
//! `None` so a broken icon never blocks the app from launching.
//!
//! Implements [`jian_host_desktop::app_icon::AppIconLoader`] for a
//! source string interpreted as a filesystem path. The loader decodes
//! PNGs via the `png` crate (a lighter-weight dep than full
//! `image-rs`); ICO / JPEG / data-URI sources are out of scope for
//! the CLI's default loader. Hosts that need richer formats register
//! their own `AppIconLoader` impl and call `DesktopHost::with_icon`
//! directly.
//!
//! The loader stores a `base_dir` that relative paths in
//! `app.icon` are resolved against — typically the directory of the
//! `.op` file the user is running. Absolute paths are honoured as-is.
//! Tilde expansion (`~/...`) is intentionally NOT performed; users
//! who want it pass an absolute path or `--icon $HOME/...`.

use jian_host_desktop::app_icon::{AppIcon, AppIconLoader, IconError};
use jian_ops_schema::PenDocument;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

/// Resolves source strings against `base_dir` and decodes PNGs.
pub struct FsIconLoader {
    base_dir: PathBuf,
}

impl FsIconLoader {
    /// Build a loader that resolves relative `app.icon` paths
    /// against `base_dir`. Pass the parent directory of the `.op`
    /// file being run; absolute icon paths are honoured as-is.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    fn resolve(&self, source: &str) -> PathBuf {
        let p = Path::new(source);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.base_dir.join(p)
        }
    }
}

impl AppIconLoader for FsIconLoader {
    fn load(&self, source: &str) -> Result<AppIcon, IconError> {
        if source.is_empty() {
            return Err(IconError::UnreadableSource(
                "empty icon source string".into(),
            ));
        }
        let path = self.resolve(source);
        let file = File::open(&path).map_err(|e| {
            IconError::UnreadableSource(format!("open {}: {e}", path.display()))
        })?;
        // We always want RGBA8 output. The `png` crate's
        // `set_transformations` normalises common PNG variants —
        // Indexed → RGBA, RGB → RGBA via alpha=0xff, 16-bit → 8-bit
        // — so the buffer arrives in canonical form regardless of
        // what the file declared.
        let mut decoder = png::Decoder::new(BufReader::new(file));
        decoder.set_transformations(
            png::Transformations::EXPAND
                | png::Transformations::ALPHA
                | png::Transformations::STRIP_16,
        );
        let mut reader = decoder
            .read_info()
            .map_err(|e| IconError::Decode(format!("png read_info: {e}")))?;
        let width = reader.info().width;
        let height = reader.info().height;
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let frame = reader
            .next_frame(&mut buf)
            .map_err(|e| IconError::Decode(format!("png next_frame: {e}")))?;
        // Trim to the actual frame size — output_buffer_size() can
        // be larger when the source is interlaced or has unusual
        // strides.
        buf.truncate(frame.buffer_size());

        // Codex round 2 MAJOR: png 0.17 keeps Grayscale / GrayscaleAlpha
        // unchanged under EXPAND | ALPHA (it only expands paletted +
        // 1/2/4 bit depths and adds alpha to RGB). Promote the two
        // grayscale variants to RGBA8 ourselves so the post-pipeline
        // buffer is uniformly width*height*4 regardless of the source
        // PNG's color type.
        let buf = match frame.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Grayscale => {
                // 1 byte / pixel → 4 bytes / pixel: replicate luma to
                // R/G/B, alpha = 0xff.
                let mut out = Vec::with_capacity(buf.len() * 4);
                for &g in &buf {
                    out.extend_from_slice(&[g, g, g, 0xff]);
                }
                out
            }
            png::ColorType::GrayscaleAlpha => {
                // 2 bytes / pixel → 4 bytes / pixel: replicate luma,
                // preserve original alpha byte.
                let mut out = Vec::with_capacity(buf.len() * 2);
                for chunk in buf.chunks_exact(2) {
                    let g = chunk[0];
                    let a = chunk[1];
                    out.extend_from_slice(&[g, g, g, a]);
                }
                out
            }
            other => {
                return Err(IconError::Decode(format!(
                    "png: unsupported post-transform color type {other:?}; \
                     expected Rgba / Grayscale / GrayscaleAlpha"
                )));
            }
        };

        // Defensive validation: post-transformation len should be
        // width*height*4. If `png`'s transformation pipeline ever
        // produces something else for an odd PNG variant, we'd
        // rather catch it here than ship a corrupt icon.
        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        if buf.len() != expected {
            return Err(IconError::Decode(format!(
                "png post-transform buffer length {} != expected {expected} for {}x{} RGBA8",
                buf.len(),
                width,
                height
            )));
        }

        AppIcon::new(width, height, buf)
    }
}

/// Where the icon source came from. Drives the quiet-vs-loud policy
/// in [`resolve_app_icon`]:
///
/// - **Schema-field source + missing file**: silent. A fresh
///   `jian new` project declares `app.icon: "icon.png"` in its
///   scaffold but doesn't ship the file; we don't yell at every
///   launch.
/// - **Schema-field source + file exists but unreadable** (permission
///   denied, malformed PNG, decoder mismatch): loud. The user can
///   fix it and probably wants to know.
/// - **CLI override + any failure**: loud. The user explicitly typed
///   the path, so any failure is actionable.
/// - **CLI override + empty path** (clap `--icon=`): silent. The
///   documented "explicit suppression" knob.
enum IconSourceOrigin {
    CliOverride,
    SchemaField,
}

/// Resolve the runtime window icon for a CLI subcommand: the
/// `--icon` override beats the `.op`'s `app.icon` field; if neither
/// is set, returns `None`. Decoder errors return `None` so a broken
/// icon doesn't block the app from launching; severity of the
/// stderr message depends on the source (CLI loud, schema quiet for
/// missing files).
///
/// `op_path` is the full path to the `.op` file; relative paths in
/// `app.icon` are resolved against the `.op`'s parent directory.
/// CLI override paths are resolved against the CWD (clap's default
/// for `PathBuf` args).
pub fn resolve_app_icon(
    op_path: &Path,
    cli_override: Option<&Path>,
    document: &PenDocument,
) -> Option<AppIcon> {
    let (source_string, base_dir, origin) = if let Some(p) = cli_override {
        // Codex round 2 MODERATE: clap parses `--icon=` as an empty
        // path. Document says it suppresses both override and the
        // schema fallback; honour that cleanly here (return None
        // silently) instead of letting it slip into the loader and
        // emit a loud "icon load failed" stderr message.
        if p.as_os_str().is_empty() {
            return None;
        }
        (
            p.to_string_lossy().into_owned(),
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            IconSourceOrigin::CliOverride,
        )
    } else {
        let icon_field = document.app.as_ref().and_then(|a| a.icon.clone())?;
        if icon_field.is_empty() {
            return None;
        }
        let base = op_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        (icon_field, base, IconSourceOrigin::SchemaField)
    };

    // Codex rounds 3 + 4: distinguish "file is simply missing" (a
    // fresh `jian new` scaffold that hasn't shipped icon.png yet —
    // silent for schema-field sources) from real I/O / decode
    // failures (permission denied, traversal error, malformed PNG —
    // always loud because the user can fix them and they signal a
    // real config bug). Pre-resolve + existence-check before
    // handing to the loader so we can branch on the cause.
    //
    // Three try_exists() outcomes, each handled distinctly:
    //   Ok(true)  — file is there; let the loader read it. Any
    //               failure inside the loader (decode, partial
    //               read, …) is loud regardless of origin.
    //   Ok(false) — file is genuinely absent. Silent for schema
    //               (the scaffold case); loud for CLI override
    //               (the user typed the path).
    //   Err(_)    — could NOT determine existence (permission
    //               denied on a parent dir, transient IO error).
    //               Fall through to the loader without short-
    //               circuiting; File::open will produce the
    //               accurate loud error a moment later. Round 4
    //               caught this case being silently suppressed
    //               under the previous `unwrap_or(false)` shape.
    let resolved_path = {
        let p = Path::new(&source_string);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            base_dir.join(p)
        }
    };
    match resolved_path.try_exists() {
        Ok(true) => { /* fall through to the loader */ }
        Ok(false) => {
            if matches!(origin, IconSourceOrigin::SchemaField) {
                // Quiet path: scaffold's `app.icon` not on disk yet.
                return None;
            }
            eprintln!(
                "jian: icon file not found: {} (resolved from {source_string:?})",
                resolved_path.display()
            );
            return None;
        }
        Err(_) => {
            // Couldn't stat. Don't short-circuit silently — fall
            // through to the loader and let its loud `File::open`
            // error path do the talking.
        }
    }

    let loader = FsIconLoader::new(base_dir);
    match loader.load(&source_string) {
        Ok(icon) => Some(icon),
        Err(e) => {
            // File exists (or stat failed) but loading failed:
            // permission denied, malformed PNG, decoder mismatch,
            // etc. Always loud — actionable config bug.
            eprintln!("jian: icon load failed for {source_string:?}: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    /// Build a minimal RGBA8 PNG file at `path`. Writes a single-pixel
    /// frame so the encoder's path is the simplest valid PNG; the
    /// decoder's job is what we're exercising.
    fn write_test_png(path: &Path, width: u32, height: u32, pixel: [u8; 4]) {
        // Use the `png` crate's encoder so the bytes round-trip
        // through its decoder cleanly.
        let mut file = File::create(path).unwrap();
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let pixels: Vec<u8> = std::iter::repeat(pixel)
                .take(width as usize * height as usize)
                .flatten()
                .collect();
            writer.write_image_data(&pixels).unwrap();
        }
        file.write_all(&buf).unwrap();
    }

    #[test]
    fn loads_rgba_png_from_relative_path() {
        let tmp = TempDir::new().unwrap();
        let icon_path = tmp.path().join("icon.png");
        write_test_png(&icon_path, 4, 4, [0xff, 0x00, 0x00, 0xff]);

        let loader = FsIconLoader::new(tmp.path());
        let icon = loader.load("icon.png").expect("decode succeeds");
        assert_eq!(icon.width(), 4);
        assert_eq!(icon.height(), 4);
        assert_eq!(icon.rgba().len(), 4 * 4 * 4);
        // Every pixel red, full alpha.
        for chunk in icon.rgba().chunks_exact(4) {
            assert_eq!(chunk, &[0xff, 0x00, 0x00, 0xff]);
        }
    }

    #[test]
    fn honours_absolute_path() {
        let tmp = TempDir::new().unwrap();
        let icon_path = tmp.path().join("icon.png");
        write_test_png(&icon_path, 2, 2, [0x10, 0x20, 0x30, 0xff]);

        // Loader with a different base_dir; absolute path bypasses it.
        let loader = FsIconLoader::new("/this/does/not/exist");
        let icon = loader.load(icon_path.to_str().unwrap()).expect("decode");
        assert_eq!(icon.width(), 2);
        assert_eq!(icon.height(), 2);
    }

    #[test]
    fn empty_source_returns_unreadable_source() {
        let loader = FsIconLoader::new("/tmp");
        match loader.load("") {
            Err(IconError::UnreadableSource(_)) => {}
            other => panic!("expected UnreadableSource, got {other:?}"),
        }
    }

    #[test]
    fn missing_file_returns_unreadable_source() {
        let tmp = TempDir::new().unwrap();
        let loader = FsIconLoader::new(tmp.path());
        match loader.load("nope.png") {
            Err(IconError::UnreadableSource(msg)) => {
                assert!(msg.contains("nope.png"), "msg = {msg}");
            }
            other => panic!("expected UnreadableSource, got {other:?}"),
        }
    }

    /// Helper: write a PNG of arbitrary color_type / depth so the
    /// grayscale-conversion regression tests can exercise the
    /// non-RGBA decode path without depending on hand-crafted bytes.
    fn write_png(
        path: &Path,
        width: u32,
        height: u32,
        color_type: png::ColorType,
        bit_depth: png::BitDepth,
        pixels: &[u8],
    ) {
        let mut file = File::create(path).unwrap();
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, width, height);
            encoder.set_color(color_type);
            encoder.set_depth(bit_depth);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixels).unwrap();
        }
        file.write_all(&buf).unwrap();
    }

    #[test]
    fn loads_grayscale_png_promotes_to_rgba() {
        // Codex round 2 MAJOR: png 0.17's EXPAND | ALPHA does not
        // promote Grayscale → RGBA. The loader's post-pipeline branch
        // does. A 4×4 grayscale PNG with luma 0x80 must decode to
        // (0x80, 0x80, 0x80, 0xff) per pixel.
        let tmp = TempDir::new().unwrap();
        let icon_path = tmp.path().join("gray.png");
        let pixels: Vec<u8> = vec![0x80; 16];
        write_png(
            &icon_path,
            4,
            4,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            &pixels,
        );
        let loader = FsIconLoader::new(tmp.path());
        let icon = loader.load("gray.png").expect("grayscale decodes");
        assert_eq!(icon.width(), 4);
        assert_eq!(icon.height(), 4);
        assert_eq!(icon.rgba().len(), 4 * 4 * 4);
        for chunk in icon.rgba().chunks_exact(4) {
            assert_eq!(chunk, &[0x80, 0x80, 0x80, 0xff]);
        }
    }

    #[test]
    fn loads_grayscale_alpha_png_preserves_alpha() {
        let tmp = TempDir::new().unwrap();
        let icon_path = tmp.path().join("ga.png");
        // 2 bytes/pixel: luma + alpha. Use luma=0x40, alpha=0x80
        // for every pixel of a 2×2 image.
        let pixels: Vec<u8> = std::iter::repeat([0x40, 0x80]).take(4).flatten().collect();
        write_png(
            &icon_path,
            2,
            2,
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Eight,
            &pixels,
        );
        let loader = FsIconLoader::new(tmp.path());
        let icon = loader.load("ga.png").expect("grayscale+alpha decodes");
        assert_eq!(icon.rgba().len(), 2 * 2 * 4);
        for chunk in icon.rgba().chunks_exact(4) {
            assert_eq!(chunk, &[0x40, 0x40, 0x40, 0x80]);
        }
    }

    #[test]
    fn resolve_with_empty_cli_override_returns_none_silently() {
        // Codex round 2 MODERATE: clap parses `--icon=` as `Some("")`.
        // The doc says this suppresses both the override and the
        // schema fallback. Verify it returns None silently (i.e.
        // before reaching the loader, so no stderr noise).
        let tmp = TempDir::new().unwrap();
        let op_path = tmp.path().join("app.op");
        // Build a PenDocument with NO app block so the schema
        // fallback would also be None — narrows the test to the
        // "empty CLI override" path.
        let doc: PenDocument = serde_json::from_value(serde_json::json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "test",
            "children": []
        }))
        .unwrap();
        let result = resolve_app_icon(&op_path, Some(Path::new("")), &doc);
        assert!(result.is_none(), "empty CLI override should suppress");
    }

    #[test]
    fn resolve_schema_missing_file_returns_none() {
        // Codex round 3 MINOR: the schema declares an icon path, but
        // the file isn't there yet (typical `jian new` scaffold).
        // Should return None silently — not surface as a loud error
        // because the user hasn't done anything wrong.
        let tmp = TempDir::new().unwrap();
        let op_path = tmp.path().join("app.op");
        let doc: PenDocument = serde_json::from_value(serde_json::json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "test",
            "app": {
                "name": "T",
                "version": "0",
                "id": "test",
                "icon": "missing.png"
            },
            "children": []
        }))
        .unwrap();
        // No CLI override; schema declares the icon but the file is
        // absent. Should return None without going through the
        // loader's loud-error path.
        assert!(resolve_app_icon(&op_path, None, &doc).is_none());
    }

    #[test]
    fn resolve_cli_override_missing_file_still_returns_none() {
        // Compare with the schema-missing case above: a missing file
        // referenced by --icon is still suppressed (the runtime
        // continues without an icon), but the difference is in
        // logging — that's verified by code inspection rather than
        // an stderr-capture test.
        let tmp = TempDir::new().unwrap();
        let op_path = tmp.path().join("app.op");
        let doc: PenDocument = serde_json::from_value(serde_json::json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "test",
            "children": []
        }))
        .unwrap();
        let missing = tmp.path().join("nope.png");
        assert!(resolve_app_icon(&op_path, Some(&missing), &doc).is_none());
    }

    #[test]
    fn resolve_with_empty_cli_override_skips_schema_fallback() {
        // Even when the schema declares app.icon, an empty CLI
        // override blocks the fallback (the override branch wins).
        let tmp = TempDir::new().unwrap();
        let op_path = tmp.path().join("app.op");
        let icon_path = tmp.path().join("icon.png");
        write_test_png(&icon_path, 1, 1, [0xff, 0xff, 0xff, 0xff]);
        let doc: PenDocument = serde_json::from_value(serde_json::json!({
            "formatVersion": "1.0",
            "version": "1.0.0",
            "id": "test",
            "app": {
                "name": "T",
                "version": "0",
                "id": "test",
                "icon": "icon.png"
            },
            "children": []
        }))
        .unwrap();
        // Without --icon=, schema fallback would load icon.png.
        let with_fallback = resolve_app_icon(&op_path, None, &doc);
        assert!(with_fallback.is_some(), "schema fallback should load");
        // With --icon=, the override wins and suppresses cleanly.
        let with_override = resolve_app_icon(&op_path, Some(Path::new("")), &doc);
        assert!(with_override.is_none(), "empty override should suppress");
    }

    #[test]
    fn non_png_file_returns_decode_error() {
        let tmp = TempDir::new().unwrap();
        let bad = tmp.path().join("not-a-png.png");
        std::fs::write(&bad, b"not a PNG").unwrap();

        let loader = FsIconLoader::new(tmp.path());
        match loader.load("not-a-png.png") {
            Err(IconError::Decode(msg)) => {
                assert!(
                    msg.contains("png"),
                    "decode error message should mention png: {msg}"
                );
            }
            other => panic!("expected Decode, got {other:?}"),
        }
    }
}
