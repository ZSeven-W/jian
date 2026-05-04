//! `jian player PATH` — open a `.op` in a desktop window and run the
//! interactive pointer / scene pipeline.
//!
//! Available when `jian-cli` is built with the `player` feature
//! (default). Delegates to `jian_host_desktop::DesktopHost::run` which
//! pulls in winit + softbuffer (CPU-side pixel present) under its own
//! `run` feature — no per-platform GPU plumbing required.

use crate::pack_reader::{looks_like_op_pack, read_op_pack, snapshot_extra_keys, PackContents};
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
use jian_ops_schema::document::PenDocument;
use jian_ops_schema::load_str;
use jian_ops_schema::pack::{DefaultStateSnapshot, InitialLayoutSnapshot};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Output of the `.op.pack` vs raw-`.op` branch at the top of
/// `player::run`. Kept private so callers don't accidentally build
/// one without going through the load path.
struct LoadedSource {
    schema: PenDocument,
    initial_layout: Option<InitialLayoutSnapshot>,
    default_state: Option<DefaultStateSnapshot>,
    expressions: Option<jian_ops_schema::pack::ExpressionsSnapshot>,
}

pub fn run(args: PlayerArgs) -> Result<ExitCode> {
    // Plan 8 §T8 Windows leg: when a second `jian.exe` instance
    // is launched (file double-click, `jian://...` URL handler,
    // or `--asp` re-attach), Windows spawns a fresh process by
    // default — no Launch-Services-style routing the way macOS
    // gives us for free. We acquire a named-mutex singleton at
    // entry; if a primary is already running, we forward our
    // argv via WM_COPYDATA and exit cleanly. Only the primary
    // path falls through to the regular `jian player` startup
    // (window creation + event loop).
    //
    // The singleton guard is held in `_singleton` for the
    // lifetime of this function so the mutex stays acquired
    // across the player's whole run. Drop releases it.
    // Test-only escape hatch: when cargo's integration tests spawn
    // `jian.exe` in parallel, every child contends for the same
    // session-scoped mutex. Tests that exercise CLI-arg validation
    // (e.g. `--asp host:port`) need to fall through to the validation
    // path, but without this bypass the second-running jian sees
    // `Singleton::Secondary` and exits with our refusal message
    // before ever reaching the validation. The integration tests in
    // `tests/cli_subcommands.rs` set `JIAN_DISABLE_SINGLETON=1` for
    // every spawn so they never enter the singleton branch.
    #[cfg(target_os = "windows")]
    let _singleton: Option<jian_host_desktop::win_deeplink_receiver::SingletonGuard>;
    #[cfg(target_os = "windows")]
    let mut _receiver_window: Option<jian_host_desktop::win_deeplink_receiver::ReceiverWindow> =
        None;
    #[cfg(target_os = "windows")]
    if std::env::var_os("JIAN_DISABLE_SINGLETON").is_some() {
        _singleton = None;
    } else {
        match jian_host_desktop::win_deeplink_receiver::try_acquire_singleton() {
            jian_host_desktop::win_deeplink_receiver::Singleton::Primary(guard) => {
                // Codex round 5 HIGH: install a NullDeepLinkHandler in
                // the registry BEFORE the receiver window goes up so a
                // forwarded URL arriving during cold-start dispatches
                // cleanly (returning `false` = declined-but-protocol-
                // handled) instead of `NoHandlerRegistered`. Once the
                // host wires a real `DeepLinkHandler` (e.g. a router
                // that opens `jian://app/page` paths), this gets
                // replaced via `install_deeplink_handler`.
                let _prev = jian_host_desktop::win_deeplink::install_handler(Box::new(
                    jian_host_desktop::deeplink::NullDeepLinkHandler::default(),
                ));
                // Install the receiver window NOW (before any startup
                // work) so secondaries arriving during our cold-start
                // can FindWindowExW + SendMessageTimeoutW. The
                // `SingletonGuard` carries the ready event the receiver
                // signals; secondaries `WaitForSingleObject` on it for
                // a bounded interval before probing.
                match jian_host_desktop::win_deeplink_receiver::install_receiver_window(&guard) {
                    Ok(window) => {
                        _receiver_window = Some(window);
                    }
                    Err(e) => {
                        use std::io::Write;
                        let _ = writeln!(
                            std::io::stderr(),
                            "jian: warning — receiver window install failed ({e}); \
                         deep links will not route into this instance"
                        );
                    }
                }
                _singleton = Some(guard);
            }
            jian_host_desktop::win_deeplink_receiver::Singleton::Secondary => {
                // Codex round 3 MEDIUM Q4 #2: the receiver's
                // dispatch_url only accepts `jian://` URLs (it parses
                // via `JianUrl::parse` which rejects other schemes).
                // Forwarding a bare `.op` path or `file://...` URI
                // would always come back as `PrimaryRejected`. Until a
                // typed wire enum lands for file-open dispatch,
                // restrict secondary forwarding to `jian://` only.
                let raw = args.path.to_string_lossy();
                if !raw.starts_with("jian://") {
                    return Err(anyhow!(
                        "another `jian.exe` is already running. The Plan 8 §T8 \
                     receiver only forwards `jian://` URLs at this time; \
                     close the running instance first to open `{raw}`."
                    ));
                }
                use jian_host_desktop::win_deeplink_receiver::ForwardOutcome;
                match jian_host_desktop::win_deeplink_receiver::forward_url_to_primary(&raw) {
                    Ok(ForwardOutcome::Delivered) => return Ok(ExitCode::SUCCESS),
                    Ok(ForwardOutcome::PrimaryRejected) => {
                        // The primary is alive and saw the URL but
                        // refused it. Surface as an error so the
                        // user sees the rejection instead of a
                        // silent drop.
                        return Err(anyhow!(
                            "running `jian.exe` rejected the forwarded URL — \
                         see the primary's stderr for details"
                        ));
                    }
                    Ok(ForwardOutcome::NoPeer) => {
                        return Err(anyhow!(
                            "another `jian.exe` holds the singleton mutex but its \
                         receiver window was not found — refusing to start a \
                         second window"
                        ));
                    }
                    Ok(ForwardOutcome::SendTimedOut) => {
                        return Err(anyhow!(
                            "running `jian.exe` did not respond to the forwarded \
                         URL within 5 s — refusing to start a second window"
                        ));
                    }
                    Ok(ForwardOutcome::SendFailed { last_error }) => {
                        return Err(anyhow!(
                            "running `jian.exe` failed to receive the forwarded URL \
                         (Win32 error {last_error}) — refusing to start a second \
                         window"
                        ));
                    }
                    Err(e) => {
                        return Err(anyhow!("forward to running instance failed: {e}"));
                    }
                }
            }
        }
    }

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
    // Plan 19 D1 end-to-end: branch on `.op.pack` extension. A pack
    // archive carries the parsed schema plus optional AOT entries
    // (`aot/initial_layout.bin` + `aot/default_state.bin`); the
    // bootstrap below threads them through `install_data_path_with_aot`
    // so the runtime can skip `ComputeFirstLayout` and re-seed the
    // StateGraph from the canonical snapshot. A raw `.op` falls
    // through to the original load path with both AOT slots `None`.
    let LoadedSource {
        schema,
        initial_layout,
        default_state,
        expressions,
    } = if looks_like_op_pack(&resolved_path) {
        let PackContents {
            schema,
            initial_layout,
            default_state,
            expressions,
        } = read_op_pack(&resolved_path)
            .with_context(|| format!("read pack {}", resolved_path.display()))?;
        LoadedSource {
            schema,
            initial_layout,
            default_state,
            expressions,
        }
    } else {
        let src = fs::read_to_string(&resolved_path)
            .with_context(|| format!("read {}", resolved_path.display()))?;
        let schema = load_str(&src)
            .with_context(|| format!("parse {}", resolved_path.display()))?
            .value;
        LoadedSource {
            schema,
            initial_layout: None,
            default_state: None,
            expressions: None,
        }
    };

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
    // Plan 19 D1: when the source was a `.op.pack` carrying a
    // pre-baked initial-layout snapshot, feed it through the AOT
    // bootstrap variant so `SeedStateGraph` preloads it and
    // `ComputeFirstLayout` can short-circuit (only when coverage is
    // total + viewport bit-matches; partial / mismatched coverage
    // falls through to a real compute, see jian-core's
    // `install_data_path_with_aot` doc). A raw `.op` source has
    // `initial_layout == None` and we use the regular
    // `install_data_path` entry so the bootstrap's behaviour is
    // unchanged.
    // Plan 19 D2: when the pack ships `aot/expressions.bin`, feed
    // it alongside the layout snapshot so the bootstrap installs
    // pre-compiled chunks into the runtime's expression cache.
    // `install_data_path_with_aot_full` accepts both as `Option`,
    // so a layout-only pack and a JSON-only `.op` both fall through
    // to the original code paths unchanged.
    let bootstrap = if initial_layout.is_some() || expressions.is_some() {
        HostAgnosticBootstrap::install_data_path_with_aot_full(
            &mut driver,
            BootstrapSource::Schema(Box::new(schema)),
            (w, h),
            initial_layout,
            expressions,
        )
    } else {
        HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::Schema(Box::new(schema)),
            (w, h),
        )
    };
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
    // Plan 19 D1: pack-supplied AOT default-state overlays the
    // schema-default seed (`SeedStateGraph` already ran by this
    // point). Restore reuses Signal slot identity so binding
    // subscribers wired during `SeedStateGraph` survive the AOT
    // overlay — see `StateGraph::restore_default_state`.
    //
    // Codex round 1 MEDIUM #4: a stale snapshot (older `.op.pack`
    // paired with a newer `.op` schema that dropped a key) would
    // silently re-introduce ghost signals via the additive
    // `*_set` setters. Gate the restore on "snapshot is a subset
    // of what the schema just seeded"; on mismatch warn + skip
    // the whole restore so the runtime keeps the schema-fresh
    // seed.
    if let Some(state_snap) = default_state.as_ref() {
        let baseline = rt.state.dump_default_state();
        let extras = snapshot_extra_keys(state_snap, &baseline);
        if extras.is_empty() {
            rt.state.restore_default_state(state_snap);
        } else {
            eprintln!(
                "jian: warning — `aot/default_state.bin` carries {} key(s) the active \
                 schema no longer seeds ({}); skipping AOT state restore, runtime keeps \
                 schema-default seed",
                extras.len(),
                extras.join(", "),
            );
        }
    }
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
    let asp_permission = map_cli_permission(args.asp_permission);
    #[cfg(feature = "prod-asp")]
    let _asp_session = match args.asp.as_deref() {
        Some(arg) => Some(start_asp_listener_session(arg, &host, asp_permission)?),
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
            asp_permission,
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
/// lifetime. Drop sets the quit flag, cancels any pending blocking
/// I/O on the accept thread (Windows only), joins, and removes the
/// token file. The listener's own `Drop` (inside the joined thread)
/// unlinks the socket + parent dir, so the file system is clean
/// when the player exits.
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
    /// Path to the generated token file. Removed on drop.
    token_path: std::path::PathBuf,
    /// Win32 thread id of the accept thread. Used by `Drop` on
    /// Windows to call `CancelSynchronousIo` — the accept thread
    /// is parked in `ConnectNamedPipe` (a synchronous syscall the
    /// quit flag can't reach), and only an explicit cancel
    /// unblocks it. The Unix accept thread polls in non-blocking
    /// mode, so this id is unused there.
    #[cfg(windows)]
    accept_thread_id: u32,
}

#[cfg(feature = "prod-asp")]
impl Drop for AspSession {
    fn drop(&mut self) {
        // 1. Signal the accept loop to stop.
        self.quit_flag
            .store(true, std::sync::atomic::Ordering::Release);
        // 2. On Windows, the accept thread is parked in
        //    `ConnectNamedPipe`; only `CancelSynchronousIo` can
        //    unblock it. Pop the thread out of the syscall so the
        //    next loop iteration sees the quit flag and returns.
        //    `OpenThread` with `THREAD_TERMINATE` access is the
        //    minimum the cancel API needs (per MSDN).
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{OpenThread, THREAD_TERMINATE};
            // SAFETY: `accept_thread_id` was captured at thread
            // start (see the spawn site below); the OS thread is
            // guaranteed alive until our `join` completes. Even if
            // the thread already exited on its own, `OpenThread`
            // returns NULL which we handle gracefully.
            let h = OpenThread(THREAD_TERMINATE, 0, self.accept_thread_id);
            if !h.is_null() {
                // `CancelSynchronousIo` lives in `Win32_System_IO`.
                use windows_sys::Win32::System::IO::CancelSynchronousIo;
                let _ = CancelSynchronousIo(h);
                let _ = CloseHandle(h);
            }
        }
        // 3. Join — the next loop tick (Unix: ≤ 50 ms; Windows:
        //    immediately after the cancel above unblocks accept)
        //    sees the flag, breaks, and the listener's own Drop
        //    unlinks the socket file. `join()` returning Err means
        //    the thread panicked; we ignore it (the listener was
        //    already dropped on the way out).
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
        // 4. Best-effort token file cleanup. A crash that reaches
        //    process-exit before our Drop runs leaves the token
        //    file orphaned, but a stale token is harmless once the
        //    socket is gone (no listener, no attack surface).
        let _ = std::fs::remove_file(&self.token_path);
    }
}

/// Translate the CLI's `--asp-permission` value into the asp-side
/// `Permission` enum. Pure helper, kept in this module so the CLI
/// arg type doesn't leak into `jian-asp`.
#[cfg(feature = "prod-asp")]
fn map_cli_permission(level: crate::AspPermissionLevel) -> jian_asp::session::Permission {
    use jian_asp::session::Permission;
    match level {
        crate::AspPermissionLevel::Observe => Permission::Observe,
        crate::AspPermissionLevel::Act => Permission::Act,
        crate::AspPermissionLevel::Full => Permission::Full,
    }
}

/// Bind the listener, write the token file, and spawn the accept
/// thread. Returns the live session whose lifetime keeps everything
/// running.
#[cfg(all(unix, feature = "prod-asp"))]
fn start_asp_listener_session(
    arg: &str,
    host: &jian_host_desktop::DesktopHost,
    permission: jian_asp::session::Permission,
) -> Result<AspSession> {
    use jian_asp::session::FileTokenValidator;
    use jian_asp::transport::socket_path::{resolve_bind_arg, BindTarget};
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
    // File-reading validator: the operator can `rm <token-file>`
    // to revoke (next handshake fails with `token unavailable`) or
    // `echo new > <token-file>` to rotate (new connections need
    // the new secret). Existing sessions stay live until the agent
    // disconnects on its own — revoke is "no new connections", not
    // "kick in-flight".
    let _ = token; // path is the source of truth from here on
    let validator = FileTokenValidator::new(token_path.clone(), permission);
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

    let token =
        read_random_token_hex(32).map_err(|e| anyhow!("--asp: could not generate token: {}", e))?;
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
        .map_err(|e| {
            anyhow!(
                "--asp: write {}: {} \
             (if a previous run crashed, remove the stale token file and retry)",
                token_path.display(),
                e
            )
        })?;
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
    arg: &str,
    host: &jian_host_desktop::DesktopHost,
    permission: jian_asp::session::Permission,
) -> Result<AspSession> {
    use jian_asp::session::FileTokenValidator;
    use jian_asp::transport::socket_path::{resolve_bind_arg, BindTarget};
    use std::time::Instant;

    // Path-shape validation runs first.
    let target = resolve_bind_arg(arg, std::process::id(), |k| std::env::var(k).ok())
        .map_err(|e| anyhow!("--asp: {}", e))?;
    let BindTarget::NamedPipe(pipe_name) = target else {
        return Err(anyhow!(
            "--asp: internal: unexpected bind target on windows"
        ));
    };

    // Capability gate (mirror of Unix path).
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

    let listener = jian_asp::transport::NamedPipeListener::bind(&pipe_name)
        .map_err(|e| anyhow!("--asp: {}", e))?;

    // Token file: written next to the pipe-namespace path doesn't
    // make sense (Named Pipes aren't filesystem-rooted), so we put
    // the token under `%LOCALAPPDATA%\jian\<pid>.asp.token` with
    // no per-user ACL adjustments — the LocalAppData dir is
    // user-private by default on Windows, and the token file
    // inherits that.
    let token_path = {
        let local = std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or_else(|| {
                anyhow!("--asp: cannot resolve LOCALAPPDATA / USERPROFILE for token file")
            })?;
        let mut p = std::path::PathBuf::from(local);
        p.push("jian");
        std::fs::create_dir_all(&p)
            .map_err(|e| anyhow!("--asp: create_dir_all {}: {}", p.display(), e))?;
        p.push(format!("{}.asp.token", std::process::id()));
        p
    };
    let token = match write_token_file(&token_path) {
        Ok(t) => t,
        Err(e) => {
            drop(listener);
            return Err(e);
        }
    };
    eprintln!(
        "jian player: ASP listening on {} (token file: {})",
        pipe_name,
        token_path.display()
    );

    let (bridge, drain) = jian_asp::bridge::channel();
    // File-reading validator (operator-revoke + rotate path); see
    // the Unix branch for design notes.
    let _ = token;
    let validator = FileTokenValidator::new(token_path.clone(), permission);
    let quit_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let quit_flag_thread = quit_flag.clone();

    // Channel for the accept thread to report its OS thread id back
    // to the main thread. `Drop` uses the id to call
    // `CancelSynchronousIo` and unblock the thread parked in
    // `ConnectNamedPipe`. `sync_channel(1)` keeps the spawn cost
    // bounded (one slot, no allocation in the common path).
    let (tid_tx, tid_rx) = std::sync::mpsc::sync_channel::<u32>(1);

    let accept_thread = std::thread::Builder::new()
        .name("jian-asp-listener".into())
        .spawn(move || {
            // First action: report our thread id so main can cancel
            // us at shutdown. Failure to send means the main thread
            // already gave up before we reported — proceed anyway;
            // worst case the thread is reaped by process-exit.
            // SAFETY: GetCurrentThreadId is a stateless thread-
            // local query; safe to call without preconditions.
            let tid = unsafe {
                use windows_sys::Win32::System::Threading::GetCurrentThreadId;
                GetCurrentThreadId()
            };
            let _ = tid_tx.send(tid);

            loop {
                if quit_flag_thread.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                // Named Pipe `accept()` is blocking; on shutdown
                // `AspSession::Drop` calls
                // `CancelSynchronousIo(thread_handle)` which makes
                // this call return Err and the loop exit.
                let mut transport = match listener.accept() {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("jian player: ASP accept ended: {}", e);
                        return;
                    }
                };
                let session_result = jian_asp::server::run_prod_session_via_bridge(
                    &mut transport,
                    &validator,
                    &bridge,
                    Instant::now(),
                );
                if let Err(e) = session_result {
                    eprintln!("jian player: ASP session ended: {}", e);
                }
                // Re-arm for the next agent.
                if let Err(e) = listener.disconnect_and_reuse() {
                    eprintln!("jian player: ASP disconnect failed; ending listener: {}", e);
                    return;
                }
            }
        })
        .map_err(|e| anyhow!("--asp: spawn listener thread: {}", e))?;

    // Block on the thread reporting its id. `unwrap_or` to a
    // sentinel `0` value means an unhealthy spawn still produces a
    // valid (if unjoinable) `AspSession` — Drop then no-ops the
    // cancel because `OpenThread(0)` returns NULL.
    let accept_thread_id = tid_rx.recv().unwrap_or(0);

    Ok(AspSession {
        drain: std::sync::Mutex::new(Some(drain)),
        quit_flag,
        accept_thread: Some(accept_thread),
        token_path,
        accept_thread_id,
    })
}

/// Windows token file writer. `%LOCALAPPDATA%\jian\<pid>.asp.token`
/// inherits the user-private LocalAppData ACL; we still use
/// `create_new(true)` to refuse a stale token from a crashed prior
/// run.
#[cfg(all(windows, feature = "prod-asp"))]
fn write_token_file(token_path: &std::path::Path) -> Result<String> {
    use std::io::Write;
    let token = read_random_token_hex_windows(32)
        .map_err(|e| anyhow!("--asp: could not generate token: {}", e))?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(token_path)
        .map_err(|e| {
            anyhow!(
                "--asp: write {}: {} \
             (if a previous run crashed, remove the stale token file and retry)",
                token_path.display(),
                e
            )
        })?;
    if let Err(e) = f
        .write_all(token.as_bytes())
        .and_then(|_| f.write_all(b"\n"))
    {
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

/// Windows random token: read via `BCryptGenRandom`. We avoid
/// pulling in the `rand` crate; `BCryptGenRandom` is stable, in
/// CryptoAPI Next Gen, and produces cryptographic-quality output.
#[cfg(all(windows, feature = "prod-asp"))]
fn read_random_token_hex_windows(n: usize) -> std::io::Result<String> {
    // Open a system-default RNG provider. Algorithm
    // `BCRYPT_RNG_ALGORITHM` = "RNG"; `flags = 0` for default.
    // Output: `n` random bytes.
    extern "system" {
        fn BCryptGenRandom(
            hAlgorithm: *mut std::ffi::c_void,
            pbBuffer: *mut u8,
            cbBuffer: u32,
            dwFlags: u32,
        ) -> i32;
    }
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
    let mut buf = vec![0u8; n];
    // SAFETY: `buf` is a valid n-byte buffer; the
    // `BCRYPT_USE_SYSTEM_PREFERRED_RNG` flag means we don't have
    // to open / close an algorithm provider handle.
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            buf.as_mut_ptr(),
            n as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("BCryptGenRandom failed: 0x{:x}", status),
        ));
    }
    let mut hex = String::with_capacity(n * 2);
    for b in buf {
        hex.push_str(&format!("{:02x}", b));
    }
    Ok(hex)
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
