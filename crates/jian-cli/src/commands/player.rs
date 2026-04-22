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
use std::process::ExitCode;

pub fn run(args: PlayerArgs) -> Result<ExitCode> {
    let src =
        fs::read_to_string(&args.path).with_context(|| format!("read {}", args.path.display()))?;
    let schema = load_str(&src)
        .with_context(|| format!("parse {}", args.path.display()))?
        .value;

    // Title priority: --title > schema.app.name > file stem > "Jian".
    let title = args
        .title
        .clone()
        .or_else(|| schema.app.as_ref().map(|a| a.name.clone()))
        .or_else(|| {
            args.path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Jian".to_owned());

    let (w, h) = parse_size(args.size.as_deref())?;

    let mut rt = Runtime::new_from_document(schema)
        .with_context(|| format!("build runtime from {}", args.path.display()))?;
    rt.build_layout((w, h)).with_context(|| "layout")?;
    rt.rebuild_spatial();

    let cfg = HostConfig {
        title,
        initial_size: size(w, h),
    };
    let host = DesktopHost::with_config(rt, cfg);
    host.run().map_err(|e| anyhow!("event loop error: {}", e))?;
    Ok(ExitCode::SUCCESS)
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
}
