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
use jian_core::startup::{
    BootstrapSource, HostAgnosticBootstrap, StartupConfig, StartupDriver, StartupReport,
    StartupStage,
};
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

    // Auto-size detection runs BEFORE the bootstrap so the recorded
    // `ComputeFirstLayout` / `BuildVisibleSpatial` timings describe
    // the FINAL geometry the visual stage paints into. (Codex round
    // 1 of the B4 implementation review, HIGH: a post-bootstrap
    // re-layout would leave the report describing the first
    // attempt's geometry while the runtime used a second.)
    //
    // The probe `Runtime::new_from_document` runs in O(seedStateGraph
    // + buildNodeTree + buildLayout) cost; that's ~30 ms in the
    // worst case the perf harness has measured. The cost is paid
    // outside the bootstrap's recorded timeline because we drop the
    // probe runtime immediately after measuring — the bootstrap
    // re-constructs the runtime for real against the corrected
    // viewport.
    if args.size.is_none() {
        let mut probe = Runtime::new_from_document(schema.clone())
            .with_context(|| format!("probe runtime for {}", resolved_path.display()))?;
        probe.build_layout((w, h)).with_context(|| "probe layout")?;
        if let Some((mw, mh)) = measured_content_bounds(&probe) {
            if mw > w {
                w = mw.ceil();
            }
            if mh > h {
                h = (mh + 12.0).ceil(); // small safety margin
            }
        }
        drop(probe);
    }

    // Plan 19 capstone B4: drive stage 1 (DataPath) through the
    // host-agnostic bootstrap. Records real SeedStateGraph /
    // ComputeFirstLayout / BuildVisibleSpatial costs against the
    // user's `.op` at the **final auto-sized viewport**. ReadFile /
    // ParseSchema short-circuit because the schema was pre-parsed
    // for the title / size / icon resolution above. The cumulative
    // report flows into `DesktopHost`; the visual stage's first
    // redraw merges its own per-phase timings into the same report
    // on the winit thread (after a launch-epoch shift so the
    // timeline is monotonic).
    let launch_epoch = std::time::Instant::now();
    let mut driver = StartupDriver::new();
    let bootstrap = HostAgnosticBootstrap::install_data_path(
        &mut driver,
        BootstrapSource::Schema(Box::new(schema)),
        (w, h),
    );
    // Capture the offset between launch_epoch and the DataPath
    // stage's t0 (the moment block_on enters run_filtered) so the
    // recorded `started_at_ms` values can be rebased into the
    // launch frame before merge. The offset is sub-millisecond on
    // a healthy machine, but pinning it makes the cumulative
    // timeline strictly monotonic. (Codex full B review, round 1,
    // MEDIUM #3.)
    let datapath_t0_offset_ms = launch_epoch.elapsed().as_secs_f64() * 1000.0;
    let mut stage1_report = futures::executor::block_on(driver.run_stage(
        StartupStage::DataPath,
        &StartupReport::default(),
        StartupConfig::default(),
    ))
    .map_err(|e| anyhow!("data-path stage failed: {e}"))?;
    stage1_report.shift_started_at_by_ms(datapath_t0_offset_ms);
    let mut startup_report = StartupReport::default();
    startup_report
        .merge_into(stage1_report)
        .map_err(|e| anyhow!("{e}"))?;
    let rt = bootstrap
        .take_runtime()
        .ok_or_else(|| anyhow!("data-path bootstrap did not produce a runtime"))?;
    let hidden_bboxes = bootstrap.take_hidden_bboxes();

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
    let mut host = DesktopHost::with_config(rt, cfg)
        .with_default_menu()
        .with_startup_report(startup_report)
        .with_launch_epoch(launch_epoch);
    if let Some(bboxes) = hidden_bboxes {
        host = host.with_pending_hidden_bboxes(bboxes);
    }
    let host = install_updater_from_doc(host);

    // Plan 18 ASP prod mode / C4: if `--asp <arg>` was passed, bind
    // the listener + spawn the accept thread BEFORE entering the
    // winit event loop. The CLI generates a per-session secret,
    // writes it to a `<socket>.token` file (mode 0600), and hands
    // the bridge to the host so `about_to_wait` can drain pending
    // verbs against the live runtime.
    //
    // Pre-flight check: prod ASP requires `app.capabilities` to be
    // non-empty (spec §4 / C3b). Refuse to start the listener if
    // the loaded `.op` doesn't opt in — easier diagnosis than
    // letting the agent connect and hit `ProdCapabilitiesEmpty`.
    #[cfg(feature = "prod-asp")]
    let _asp_session = match args.asp.as_deref() {
        Some(arg) => Some(start_asp_listener_session(arg, &host)?),
        None => None,
    };

    #[cfg(feature = "prod-asp")]
    let host = match _asp_session.as_ref() {
        Some(s) => host.with_asp(
            // SAFETY: `_asp_session` keeps the bridge alive; we hand
            // the drain to the host and the listener thread keeps
            // its bridge clone. The drain is `!Clone` so we can
            // only install it once.
            s.drain
                .lock()
                .unwrap()
                .take()
                .expect("drain already moved into host"),
            jian_asp::session::Permission::Act,
            "jian-player",
            env!("CARGO_PKG_VERSION"),
        ),
        None => host,
    };

    host.run().map_err(|e| anyhow!("event loop error: {}", e))?;
    Ok(ExitCode::SUCCESS)
}

/// Live state of an ASP listener bound to `--asp <path>`. Holding
/// the value alive keeps the listener thread (and the bridge end
/// the host's drain talks to) running for the whole `host.run()`
/// lifetime. Drop sets the quit flag, joins the accept thread, and
/// removes the token file. The listener's own `Drop` (inside the
/// joined thread) unlinks the socket + parent dir, so the file
/// system is clean when the player exits.
#[cfg(feature = "prod-asp")]
struct AspSession {
    /// Drain handed off to the host via `with_asp`. Wrapped in
    /// `Mutex<Option<_>>` so the surrounding code can `.take()` it
    /// once at host-build time without owning `&mut self` across
    /// the move.
    drain: std::sync::Mutex<Option<jian_asp::bridge::AspDrain>>,
    /// Quit flag the accept thread polls every poll-tick. Set on
    /// `Drop` so the loop falls out and the listener (held inside
    /// the thread) drops cleanly — its `Drop` impl unlinks the
    /// socket file.
    quit_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// `Option` so `Drop` can `take()` the `JoinHandle` and `join`
    /// it without borrowing `&mut self` across the move.
    accept_thread: Option<std::thread::JoinHandle<()>>,
    /// Path to the generated `<socket>.token` file. Removed on drop.
    token_path: std::path::PathBuf,
}

#[cfg(feature = "prod-asp")]
impl Drop for AspSession {
    fn drop(&mut self) {
        // 1. Signal the accept loop to stop.
        self.quit_flag
            .store(true, std::sync::atomic::Ordering::Release);
        // 2. Join — the next poll tick (≤ 50 ms) sees the flag,
        //    breaks the loop, and the listener's own Drop unlinks
        //    the socket file + reaps the parent dir. If the thread
        //    has somehow already exited (panic), `join()` returns
        //    Err and we ignore it.
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
        // 3. Best-effort token file cleanup. A crash that reaches
        //    process-exit before our Drop runs leaves the token
        //    file orphaned, but a stale token is harmless once the
        //    socket is gone (no listener, no attack surface).
        let _ = std::fs::remove_file(&self.token_path);
    }
}

/// Bind the listener, write the token file, and spawn the accept
/// thread. Returns the live session whose lifetime keeps everything
/// running.
#[cfg(all(unix, feature = "prod-asp"))]
fn start_asp_listener_session(
    arg: &str,
    host: &jian_host_desktop::DesktopHost,
) -> Result<AspSession> {
    use jian_asp::session::{Permission, StaticTokenValidator};
    use jian_asp::transport::socket_path::{resolve_bind_arg, BindTarget};
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::Instant;

    // Path-shape validation runs first so a typo'd `--asp tcp://...`
    // refuses with a clear arg-parsing error regardless of the
    // loaded doc. Capability + bind happen after — by then the
    // operator at least knows the path is well-formed.
    let target = resolve_bind_arg(arg, std::process::id(), |k| std::env::var(k).ok())
        .map_err(|e| anyhow!("--asp: {}", e))?;
    let BindTarget::UnixSocket(path) = target else {
        return Err(anyhow!("--asp: internal: unexpected bind target on unix"));
    };

    // Capability gate: refuse to start the listener if the loaded
    // doc has no `app.capabilities`. Same check `run_prod_session`
    // applies at session-start; surfacing it now produces a clear
    // CLI error rather than a confusing per-connection refusal
    // after the operator has already run the agent.
    let capabilities_opt_in = host
        .runtime
        .document
        .as_ref()
        .and_then(|d| d.schema.app.as_ref())
        .and_then(|a| a.capabilities.as_ref())
        .is_some_and(|caps| !caps.is_empty());
    if !capabilities_opt_in {
        return Err(anyhow!(
            "--asp: refusing to start listener — `app.capabilities` is empty or absent. \
             Prod ASP requires the .op author to declare at least one capability \
             (Plan 18 spec §4)."
        ));
    }
    let listener = jian_asp::transport::UnixSocketListener::bind(&path)
        .map_err(|e| anyhow!("--asp: {}", e))?;
    // Non-blocking + poll loop so the accept thread can check the
    // quit flag (set by `AspSession::Drop`) without blocking
    // forever in `accept()` when the user closes the player.
    listener
        .set_nonblocking(true)
        .map_err(|e| anyhow!("--asp: set_nonblocking: {}", e))?;
    let socket_path = listener.path().to_path_buf();

    // Token file path: `<socket>.token` sibling. Lives in the
    // same parent dir, which `UnixSocketListener::bind` already
    // mode-locked to 0700, so even with `0644` perms the token
    // wouldn't be reachable by another local user. The 0600 mode
    // is belt-and-braces for backup tooling.
    let token_path = {
        let mut p = socket_path.clone();
        p.set_file_name(format!(
            "{}.token",
            socket_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("asp.sock")
        ));
        p
    };

    // Generate the token + write the file. On any failure after the
    // socket bound, drop the listener (which unlinks the socket)
    // and the partial token file before returning the error so the
    // file system is clean. (Codex C4-followup round 1 MEDIUM #6.)
    let token = match write_token_file(&token_path) {
        Ok(t) => t,
        Err(e) => {
            // Listener still owned locally; explicit drop unlinks
            // the socket + parent dir.
            drop(listener);
            return Err(e);
        }
    };
    eprintln!(
        "jian player: ASP listening on {} (token file: {})",
        socket_path.display(),
        token_path.display()
    );

    let (bridge, drain) = jian_asp::bridge::channel();
    let validator = StaticTokenValidator::new(token, Permission::Act);
    let quit_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let quit_flag_thread = quit_flag.clone();

    // Accept loop: one connection at a time. Concurrent agents are
    // out of scope for prod ASP today (an .op typically has a
    // single live agent); a future revision can spawn per-conn
    // threads off this loop.
    let accept_thread = std::thread::Builder::new()
        .name("jian-asp-listener".into())
        .spawn(move || {
            // 50 ms poll interval — fine-grained enough that
            // `host.run()` returning unblocks within a frame, but
            // coarse enough that an idle listener costs negligible
            // CPU (the syscall is one accept attempt).
            let poll_interval = std::time::Duration::from_millis(50);
            loop {
                if quit_flag_thread.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                let mut transport = match listener.try_accept() {
                    Ok(Some(t)) => t,
                    Ok(None) => {
                        // No pending connection — sleep + recheck
                        // the quit flag.
                        std::thread::sleep(poll_interval);
                        continue;
                    }
                    Err(e) => {
                        eprintln!("jian player: ASP accept ended: {}", e);
                        return;
                    }
                };
                // Each accepted session blocks until the agent
                // exits or hangs up. The agent's transport is a
                // brand-new `UnixStream` (blocking by default)
                // even though the listener is non-blocking; we
                // want the per-connection read to block normally.
                let session_result = jian_asp::server::run_prod_session_via_bridge(
                    &mut transport,
                    &validator,
                    &bridge,
                    Instant::now(),
                );
                if let Err(e) = session_result {
                    eprintln!("jian player: ASP session ended: {}", e);
                }
                // Loop back and accept the next connection. The
                // shared `bridge` clone lives for the whole
                // listener lifetime; the host's drain stays the
                // same end.
            }
        })
        .map_err(|e| anyhow!("--asp: spawn listener thread: {}", e))?;

    Ok(AspSession {
        drain: std::sync::Mutex::new(Some(drain)),
        quit_flag,
        accept_thread: Some(accept_thread),
        token_path,
    })
}

/// Generate a 32-byte random token from `/dev/urandom`, hex-encode
/// it, and atomically write it to `token_path` with mode `0o600`.
/// `create_new(true)` refuses to clobber an existing file — a
/// pre-existing token from a crashed prior run is suspicious and
/// reusing it would silently grant the old token's bearer access
/// to the new session.
///
/// On any failure after the file was created, removes the partial
/// file before returning so the caller's listener-drop cleanup
/// leaves nothing behind.
#[cfg(all(unix, feature = "prod-asp"))]
fn write_token_file(token_path: &std::path::Path) -> Result<String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let token = read_random_token_hex(32)
        .map_err(|e| anyhow!("--asp: could not generate token: {}", e))?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        // `create_new(true)` is `O_EXCL` on Unix — refuses if the
        // file already exists, including a stale token from a
        // crashed run. Operator must `rm` the stale file first;
        // safer than silently reusing it (codex C4 follow-up
        // round 1 MEDIUM #3).
        .create_new(true)
        .mode(0o600)
        .open(token_path)
        .map_err(|e| anyhow!(
            "--asp: write {}: {} \
             (if a previous run crashed, remove the stale token file and retry)",
            token_path.display(),
            e
        ))?;
    let written = f
        .write_all(token.as_bytes())
        .and_then(|_| f.write_all(b"\n"));
    if let Err(e) = written {
        // Partial write — drop the file handle and remove the
        // half-written file before propagating.
        drop(f);
        let _ = std::fs::remove_file(token_path);
        return Err(anyhow!(
            "--asp: write {}: {} (partial write removed)",
            token_path.display(),
            e
        ));
    }
    Ok(token)
}

#[cfg(all(windows, feature = "prod-asp"))]
fn start_asp_listener_session(
    _arg: &str,
    _host: &jian_host_desktop::DesktopHost,
) -> Result<AspSession> {
    Err(anyhow!(
        "--asp on Windows: not yet implemented — Named Pipe transport is a stub. \
         Tracking issue: jian-asp/src/transport/named_pipe.rs"
    ))
}

/// Read `n` cryptographic-random bytes from `/dev/urandom` and
/// hex-encode them. We avoid pulling in `rand` for one helper.
#[cfg(all(unix, feature = "prod-asp"))]
fn read_random_token_hex(n: usize) -> std::io::Result<String> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    let mut f = std::fs::File::open("/dev/urandom")?;
    f.read_exact(&mut buf)?;
    let mut hex = String::with_capacity(n * 2);
    for b in buf {
        hex.push_str(&format!("{:02x}", b));
    }
    Ok(hex)
}


/// Resolve a CLI argument that may be either a filesystem path or a
/// `file://` / `jian://` URI (see `packaging/linux/jian.desktop`'s
/// `Exec=jian player %U`). Returns the on-disk path to read.
///
/// `file://` handling: strip the scheme, percent-decode the rest.
/// We don't pull in `url::Url` for this tiny case — desktop file
/// managers emit ASCII-only URIs for the common case
/// (`file:///` + an absolute path), and we percent-decode the
/// handful of bytes (`%20`, `%2F`, …) inline.
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
        // POSIX `file:///abs/path` → `rest = "/abs/path"` after the
        // prefix strip; on Windows `file:///C:/path` → `rest =
        // "/C:/path"` and we strip the leading `/` so `PathBuf`
        // sees `C:/path`. Linux paths already start with `/` so we
        // leave that alone.
        let decoded = percent_decode_bytes(rest.as_bytes());
        let trimmed = strip_windows_drive_slash(decoded);
        return Ok(path_from_bytes(trimmed));
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
/// literal). Returns raw bytes so non-UTF-8 POSIX file names
/// (`file:///%FF%FE-bad-utf8.op`) survive the round-trip into
/// `PathBuf` — a previous version went via `String::from_utf8_lossy`,
/// which silently mangled those paths into `?` replacement
/// characters.
fn percent_decode_bytes(input: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            let hi = hex_nybble(input[i + 1]);
            let lo = hex_nybble(input[i + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

fn hex_nybble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// On Windows `file:///C:/path` becomes `/C:/path` after the scheme
/// strip; chop the leading `/` so `PathBuf` sees a real drive-letter
/// path. POSIX leaves the leading `/` intact (it IS the path root).
#[cfg(windows)]
fn strip_windows_drive_slash(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.first() == Some(&b'/') {
        bytes.remove(0);
    }
    bytes
}
#[cfg(not(windows))]
fn strip_windows_drive_slash(bytes: Vec<u8>) -> Vec<u8> {
    bytes
}

/// Build a `PathBuf` from raw bytes, preserving non-UTF-8 sequences
/// on POSIX where filenames are bytes-not-UTF-8. Windows paths are
/// always UTF-16 internally, but `file://` URIs on Windows are
/// already restricted to ASCII / percent-encoded UTF-8 by the URI
/// spec, so the lossy fallback is safe there.
#[cfg(unix)]
fn path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(bytes))
}
#[cfg(not(unix))]
fn path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
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
        let p = resolve_path_arg(Path::new(
            "file:///tmp/with%20space/CJ%E4%B8%AD%E6%96%87.op",
        ))
        .unwrap();
        assert_eq!(p, PathBuf::from("/tmp/with space/CJ中文.op"));
    }

    #[cfg(windows)]
    #[test]
    fn resolve_path_arg_decodes_file_uri_windows() {
        // The Windows branch strips the leading `/` so `PathBuf`
        // sees a drive-letter path. Pin both the strip and the
        // percent-decoding pass on that platform.
        let p = resolve_path_arg(Path::new("file:///C:/foo%20bar.op")).unwrap();
        assert_eq!(p, PathBuf::from("C:/foo bar.op"));
    }

    #[test]
    fn resolve_path_arg_rejects_jian_scheme() {
        let err = resolve_path_arg(Path::new("jian://app/path")).unwrap_err();
        assert!(err.to_string().contains("jian://"));
    }

    #[test]
    fn percent_decode_bytes_handles_malformed_escapes() {
        // Truncated `%X` and `%X?` keep the original bytes rather
        // than panicking — the OS may hand us malformed input.
        assert_eq!(percent_decode_bytes(b"foo%2"), b"foo%2");
        assert_eq!(percent_decode_bytes(b"foo%2Z"), b"foo%2Z");
        assert_eq!(percent_decode_bytes(b"plain"), b"plain");
    }

    #[cfg(unix)]
    #[test]
    fn resolve_path_arg_preserves_non_utf8_posix_filenames() {
        // POSIX filenames are arbitrary byte sequences. A file
        // manager could hand us `file:///%FF.op` (a single 0xFF
        // byte filename) and the previous lossy-UTF8 path mangled
        // it into the replacement char. Round-trip the bytes
        // through `PathBuf` and pin that the underlying
        // `OsStr` still has the raw `0xFF` byte.
        use std::os::unix::ffi::OsStrExt;
        let p = resolve_path_arg(Path::new("file:///%FF.op")).unwrap();
        let bytes = p.as_os_str().as_bytes();
        assert_eq!(bytes, b"/\xff.op");
    }
}
