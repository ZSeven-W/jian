//! `jian player PATH` — open a `.op` in a desktop window and run the
//! interactive pointer / scene pipeline.
//!
//! Available when `jian-cli` is built with the `player` feature
//! (default). Delegates to `jian_host_desktop::DesktopHost::run` which
//! pulls in winit + softbuffer (CPU-side pixel present) under its own
//! `run` feature — no per-platform GPU plumbing required.

use crate::PlayerArgs;
use anyhow::{anyhow, Context, Result};
use jian_core::geometry::size;
use jian_core::Runtime;
use jian_host_desktop::host::HostConfig;
use jian_host_desktop::DesktopHost;
use jian_ops_schema::load_str;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub fn run(args: PlayerArgs) -> Result<ExitCode> {
    // The Linux `.desktop` registers `Exec=jian player %U`, which
    // means the file manager hands us URI strings (`file:///path/x.op`)
    // for double-clicked files alongside `jian://...` deep links for
    // URL-scheme launches. clap stores the raw arg as a `PathBuf`
    // before we ever see it, so we resolve it here:
    //
    // - `file://...` → strip the scheme + percent-decode → real path.
    // - `jian://...` → unsupported by `player` directly (deep links
    //   need the running runtime's URL handler); error early.
    // - anything else → treat as a filesystem path verbatim.
    let resolved_path = resolve_path_arg(&args.path)?;
    let src = fs::read_to_string(&resolved_path)
        .with_context(|| format!("read {}", resolved_path.display()))?;
    let schema = load_str(&src)
        .with_context(|| format!("parse {}", resolved_path.display()))?
        .value;

    // Title priority: --title > schema.app.name > file stem > "Jian".
    let title = args
        .title
        .clone()
        .or_else(|| schema.app.as_ref().map(|a| a.name.clone()))
        .or_else(|| {
            resolved_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Jian".to_owned());

    // Size priority: --size > root-frame intrinsic size > 800x600.
    let root_size = root_frame_size(&schema);
    let (mut w, mut h) = match args.size.as_deref() {
        Some(s) => parse_size(Some(s))?,
        None => root_size.unwrap_or((800.0, 600.0)),
    };

    // Resolve the icon BEFORE moving `schema` into the Runtime —
    // resolve_app_icon needs to read schema.app.icon, and the
    // Runtime constructor takes ownership.
    let icon = crate::icon_loader::resolve_app_icon(&resolved_path, args.icon.as_deref(), &schema);

    let mut rt = Runtime::new_from_document(schema)
        .with_context(|| format!("build runtime from {}", resolved_path.display()))?;
    rt.build_layout((w, h)).with_context(|| "layout")?;

    // When auto-sizing, grow the window to cover any content that our
    // heuristic text measurement pushed below the declared height —
    // Sign Up-style bottom rows shouldn't clip.
    if args.size.is_none() {
        if let Some((mw, mh)) = measured_content_bounds(&rt) {
            if mw > w {
                w = mw.ceil();
            }
            if mh > h {
                h = (mh + 12.0).ceil(); // small safety margin
            }
            rt.build_layout((w, h)).with_context(|| "re-layout")?;
        }
    }
    rt.rebuild_spatial();

    // `--dpi`'s clap value_parser already filters out 0 / negative /
    // NaN / Inf, so the host-side fallback is just a straight pass-through.
    let cfg = HostConfig {
        title,
        initial_size: size(w, h),
        menu: None,
        icon,
        fullscreen: args.fullscreen,
        dpi_override: args.dpi,
        debug_overlay: args.debug_overlay,
    };
    let host = DesktopHost::with_config(rt, cfg).with_default_menu();
    let host = install_updater_from_doc(host);
    host.run().map_err(|e| anyhow!("event loop error: {}", e))?;
    Ok(ExitCode::SUCCESS)
}

/// Resolve a CLI argument that may be either a filesystem path or a
/// `file://` / `jian://` URI (see `dist/linux/jian.desktop`'s
/// `Exec=jian player %U`). Returns the on-disk path to read.
///
/// `file://` handling: strip the scheme, percent-decode the rest.
/// We don't pull in `url::Url` for this tiny case — desktop file
/// managers emit ASCII-only URIs for the common case (`file:///`
/// + an absolute path), and we percent-decode the handful of bytes
/// (`%20`, `%2F`, …) inline.
///
/// `jian://` is rejected with a clear error: deep links need to
/// route through the running runtime's URL handler, not the CLI's
/// fresh-document load path. Once `single-instance` ships, the CLI
/// can forward `jian://...` to the running peer instead.
fn resolve_path_arg(raw: &Path) -> Result<PathBuf> {
    let s = match raw.to_str() {
        Some(s) => s,
        // Non-UTF8 path → no URI scheme could match → use verbatim.
        None => return Ok(raw.to_path_buf()),
    };
    if let Some(rest) = s.strip_prefix("file://") {
        let decoded = percent_decode(rest);
        // POSIX `file:///abs/path` → `rest = "/abs/path"` after the
        // prefix strip; on Windows `file:///C:/path` → `rest =
        // "/C:/path"` and we strip the leading `/` so `PathBuf`
        // sees `C:/path`. Linux paths already start with `/` so we
        // leave that alone.
        #[cfg(windows)]
        let trimmed = decoded.strip_prefix('/').unwrap_or(&decoded).to_owned();
        #[cfg(not(windows))]
        let trimmed = decoded;
        return Ok(PathBuf::from(trimmed));
    }
    if s.starts_with("jian://") {
        return Err(anyhow!(
            "`jian player` cannot open `{}` directly — `jian://` deep links must \
             route through a running runtime instance",
            s
        ));
    }
    Ok(raw.to_path_buf())
}

/// Inline percent-decoder for the byte slice between `file://` and
/// the path end. Handles `%XX` byte triples; passes everything else
/// through unchanged. Stops at malformed escapes (treats `%X?` as
/// literal). Sufficient for the typical desktop-file-manager output
/// (`file:///path%20with%20spaces/foo.op`); a hand-crafted
/// `file:///%E4%B8%AD%E6%96%87.op` (CJK) likewise round-trips
/// because we operate on raw bytes.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_nybble(bytes[i + 1]);
            let lo = hex_nybble(bytes[i + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Round-trip via lossy String — the decoded bytes may contain
    // non-UTF8 sequences in pathological inputs, but `PathBuf`
    // tolerates that on the platforms we actually target (POSIX is
    // bytes; Windows already restricted us above).
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nybble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Read the runtime document's `app.updater` schema field and install
/// the matching backend on `host`. Schema-less / `kind: disabled` /
/// unknown kinds leave `host.updater` at its default `None`, so a
/// `MenuHandler` checking `host.updater.is_none()` can short-circuit
/// `app.check_updates` instead of dispatching to a no-op.
fn install_updater_from_doc(host: DesktopHost) -> DesktopHost {
    let cfg = host
        .runtime
        .document
        .as_ref()
        .and_then(|doc| doc.schema.app.as_ref())
        .and_then(|app| app.updater.as_ref());
    let Some(cfg) = cfg else {
        return host;
    };
    // Use the binary's own version (`jian-cli` and `jian-host-desktop`
    // share the workspace version) and the conventional binary name.
    match jian_host_desktop::updater::build_updater_from_schema(
        cfg,
        env!("CARGO_PKG_VERSION"),
        "jian",
    ) {
        Some(updater) => host.with_updater(updater),
        None => host,
    }
}

/// Walk every laid-out node and return the farthest `(max_x, max_y)`
/// reached by any node's bottom-right corner. Used to grow the window
/// when our char-count text measurement lets content overflow the
/// declared root-frame height (Sign Up rows, subtitle wraps, …).
fn measured_content_bounds(rt: &Runtime) -> Option<(f32, f32)> {
    let doc = rt.document.as_ref()?;
    let mut max_x: f32 = 0.0;
    let mut max_y: f32 = 0.0;
    for (key, _node) in doc.tree.nodes.iter() {
        if let Some(r) = rt.layout.node_rect(key) {
            max_x = max_x.max(r.max_x());
            max_y = max_y.max(r.max_y());
        }
    }
    if max_x <= 0.0 || max_y <= 0.0 {
        return None;
    }
    Some((max_x, max_y))
}

/// Read the root frame's explicit width/height so the window can open
/// at the design's intrinsic size. Returns `None` if the document
/// doesn't declare a top-level framed root (e.g. a bare children
/// array with no size metadata).
fn root_frame_size(schema: &jian_ops_schema::document::PenDocument) -> Option<(f32, f32)> {
    use jian_ops_schema::node::PenNode;
    use jian_ops_schema::sizing::SizingBehavior;

    fn pick(s: &Option<SizingBehavior>) -> Option<f32> {
        match s {
            Some(SizingBehavior::Number(n)) => Some(*n as f32),
            _ => None,
        }
    }

    let roots = match (&schema.pages, &schema.children) {
        (Some(pages), _) if !pages.is_empty() => &pages[0].children,
        _ => schema.children.as_slice(),
    };
    let first = roots.first()?;
    let (w, h) = match first {
        PenNode::Frame(f) => (pick(&f.container.width), pick(&f.container.height)),
        PenNode::Group(g) => (pick(&g.container.width), pick(&g.container.height)),
        PenNode::Rectangle(r) => (pick(&r.container.width), pick(&r.container.height)),
        _ => (None, None),
    };
    match (w, h) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((w, h)),
        _ => None,
    }
}

/// Parse `"WxH"` or `"WIDTHxHEIGHT"` (case-insensitive `x`). Returns
/// `(800.0, 600.0)` when `raw` is `None`.
fn parse_size(raw: Option<&str>) -> Result<(f32, f32)> {
    let s = match raw {
        None => return Ok((800.0, 600.0)),
        Some(s) => s,
    };
    let mut parts = s.splitn(2, ['x', 'X']);
    let w: f32 = parts
        .next()
        .and_then(|x| x.parse().ok())
        .ok_or_else(|| anyhow!("--size must be WxH (got `{}`)", s))?;
    let h: f32 = parts
        .next()
        .and_then(|x| x.parse().ok())
        .ok_or_else(|| anyhow!("--size must be WxH (got `{}`)", s))?;
    if w <= 0.0 || h <= 0.0 {
        return Err(anyhow!("--size W and H must both be positive"));
    }
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_size_defaults() {
        assert_eq!(parse_size(None).unwrap(), (800.0, 600.0));
    }

    #[test]
    fn parse_size_valid() {
        assert_eq!(parse_size(Some("1024x768")).unwrap(), (1024.0, 768.0));
        assert_eq!(parse_size(Some("640X480")).unwrap(), (640.0, 480.0));
    }

    #[test]
    fn parse_size_rejects_invalid() {
        assert!(parse_size(Some("1024")).is_err());
        assert!(parse_size(Some("0x100")).is_err());
        assert!(parse_size(Some("abcxdef")).is_err());
    }

    #[test]
    fn resolve_path_arg_passes_filesystem_paths_through() {
        let p = resolve_path_arg(Path::new("/tmp/foo.op")).unwrap();
        assert_eq!(p, PathBuf::from("/tmp/foo.op"));
        let p = resolve_path_arg(Path::new("relative/bar.op")).unwrap();
        assert_eq!(p, PathBuf::from("relative/bar.op"));
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_path_arg_decodes_file_uri_posix() {
        let p = resolve_path_arg(Path::new("file:///tmp/foo.op")).unwrap();
        assert_eq!(p, PathBuf::from("/tmp/foo.op"));
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_path_arg_decodes_percent_escapes() {
        let p =
            resolve_path_arg(Path::new("file:///tmp/with%20space/CJ%E4%B8%AD%E6%96%87.op"))
                .unwrap();
        assert_eq!(p, PathBuf::from("/tmp/with space/CJ中文.op"));
    }

    #[test]
    fn resolve_path_arg_rejects_jian_scheme() {
        let err = resolve_path_arg(Path::new("jian://app/path")).unwrap_err();
        assert!(err.to_string().contains("jian://"));
    }

    #[test]
    fn percent_decode_handles_malformed_escapes() {
        // Truncated `%X` and `%X?` keep the original bytes rather
        // than panicking — the OS may hand us malformed input.
        assert_eq!(percent_decode("foo%2"), "foo%2");
        assert_eq!(percent_decode("foo%2Z"), "foo%2Z");
        assert_eq!(percent_decode("plain"), "plain");
    }
}
