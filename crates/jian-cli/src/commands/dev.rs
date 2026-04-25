//! `jian dev PATH` — open `.op` in a window and reload on every save.
//!
//! Reuses every line of `jian player`'s setup path (auto-size, drift
//! guard, in-memory router/storage, softbuffer raster) and bolts a
//! filesystem watcher onto the side: each Modify/Create event for
//! `PATH` re-reads + re-parses the file and pushes the schema through
//! `DesktopHost::with_reloader`'s mpsc channel. The event loop drains
//! the channel in `about_to_wait` and rebuilds layout against the
//! current logical surface — runtime state (`$state.*`) survives the
//! reload, so an iterating designer doesn't lose their cursor /
//! counter / selection on each save.
//!
//! The watcher thread is detached: when `dev` exits via window close,
//! the channel `Sender` drops, `recv` returns `Err(Disconnected)`, and
//! the thread exits. No explicit shutdown plumbing needed.

use crate::DevArgs;
use anyhow::{anyhow, Context, Result};
use jian_core::geometry::size;
use jian_core::Runtime;
use jian_host_desktop::host::HostConfig;
use jian_host_desktop::DesktopHost;
use jian_ops_schema::load_str;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn run(args: DevArgs) -> Result<ExitCode> {
    let path = args
        .path
        .canonicalize()
        .with_context(|| format!("resolve {}", args.path.display()))?;
    let src = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let schema = load_str(&src)
        .with_context(|| format!("parse {}", path.display()))?
        .value;

    let title = args
        .title
        .clone()
        .or_else(|| schema.app.as_ref().map(|a| a.name.clone()))
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| format!("{} (dev)", s))
        })
        .unwrap_or_else(|| "Jian (dev)".to_owned());

    let root_size = root_frame_size(&schema);
    let (w, h) = match args.size.as_deref() {
        Some(s) => parse_size(Some(s))?,
        None => root_size.unwrap_or((800.0, 600.0)),
    };

    let mut rt = Runtime::new_from_document(schema)
        .with_context(|| format!("build runtime from {}", path.display()))?;
    rt.build_layout((w, h)).with_context(|| "layout")?;
    rt.rebuild_spatial();

    let (tx, rx) = mpsc::channel();

    // Watcher thread. Owns the `notify::RecommendedWatcher` so it
    // survives until the host loop closes the receiver. Each filesystem
    // event re-reads + re-parses; parse errors are reported but don't
    // bring down the loop — the designer fixes the typo and saves
    // again.
    let watch_path = path.clone();
    std::thread::Builder::new()
        .name("jian-dev-watcher".into())
        .spawn(move || {
            // Use a short-lived intra-thread channel so notify's
            // closure can stay `Send + 'static`.
            let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<Event>>();
            let mut watcher = match notify::recommended_watcher(move |res| {
                let _ = raw_tx.send(res);
            }) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("jian dev: cannot create watcher: {}", e);
                    return;
                }
            };
            // Watch the parent directory non-recursively — editors
            // often save via `rename(tmp, target)` which does NOT fire
            // a Modify event on `target`. The directory-level Create
            // event picks it up.
            let watch_dir = watch_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            );
            if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
                eprintln!("jian dev: cannot watch {}: {}", watch_dir.display(), e);
                return;
            }
            eprintln!("jian dev: watching {}", watch_path.display());

            // Coalesce bursts (most editors emit ≥ 2 events per save).
            // Wait up to 50ms for follow-ups, then re-read once.
            loop {
                match raw_rx.recv() {
                    Ok(Ok(ev)) => {
                        if !is_relevant_event(&ev, &watch_path) {
                            continue;
                        }
                        // Drain pending events before re-reading.
                        while let Ok(_) = raw_rx.recv_timeout(Duration::from_millis(50)) {}
                        let started = Instant::now();
                        match reparse(&watch_path) {
                            Ok(doc) => {
                                if tx.send(doc).is_err() {
                                    return; // host gone
                                }
                                eprintln!(
                                    "jian dev: reloaded in {} ms",
                                    started.elapsed().as_millis()
                                );
                            }
                            Err(e) => eprintln!("jian dev: parse failed: {:#}", e),
                        }
                    }
                    Ok(Err(e)) => eprintln!("jian dev: watcher error: {}", e),
                    Err(_) => return, // raw_tx dropped
                }
            }
        })
        .context("spawn watcher thread")?;

    let cfg = HostConfig {
        title,
        initial_size: size(w, h),
    };
    let host = DesktopHost::with_config(rt, cfg).with_reloader(rx);
    host.run().map_err(|e| anyhow!("event loop error: {}", e))?;
    Ok(ExitCode::SUCCESS)
}

/// Notify fires events for everything in the watched directory; we only
/// care about modify / create on the target path. We compare canonical
/// paths so editor temp-file dances (`save → rename`) still match.
fn is_relevant_event(ev: &Event, target: &std::path::Path) -> bool {
    let kind_match = matches!(
        ev.kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Any
    );
    if !kind_match {
        return false;
    }
    ev.paths.iter().any(|p| {
        p.canonicalize()
            .map(|cp| cp == target)
            .unwrap_or_else(|_| p == target)
    })
}

fn reparse(path: &std::path::Path) -> Result<jian_ops_schema::document::PenDocument> {
    let src = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let parsed = load_str(&src).with_context(|| format!("parse {}", path.display()))?;
    Ok(parsed.value)
}

// --- shared helpers (mirror player.rs) -------------------------------

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
