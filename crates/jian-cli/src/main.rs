//! `jian` — CLI toolchain for `.op` files.
//!
//! Subcommands:
//! - `jian check PATH` — parse + validate a `.op`, print diagnostics.
//! - `jian pack PATH OUT` — zip a `.op` + manifest into `.op.pack`.
//! - `jian unpack PATH OUT_DIR` — inverse of pack.
//! - `jian new NAME` — scaffold a new project from an embedded template.
//! - `jian player PATH` — open the `.op` in a real desktop window
//!   (default `player` feature; needs the `jian-host-desktop` event loop).
//! - `jian dev PATH` — `player` plus a `notify` filesystem watcher;
//!   reloads the document on save while preserving `$state.*` values.
//!
//! `player` and `dev` ship under the default `player` cargo feature.
//! `--no-default-features` builds a headless toolchain (check / pack /
//! unpack / new) suitable for CI containers without a display.

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::process::ExitCode;

/// Clap value parser for `--dpi`. Accepts a positive finite f64; rejects
/// `0`, negative values, and `nan`/`inf` so the run loop can `unwrap_or`
/// without revalidating downstream.
#[cfg(feature = "player")]
fn parse_positive_dpi(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|_| format!("not a number: `{}`", s))?;
    if v.is_finite() && v > 0.0 {
        Ok(v)
    } else {
        Err(format!("must be a finite number > 0 (got `{}`)", s))
    }
}

/// Clap value parser for `jian perf compare`'s `--noise-floor-ms`.
/// Must be finite and non-negative — `NaN` / `inf` would silently
/// disable the floor or print nonsensical deltas; a negative floor
/// would gate on noise that's already below zero. Codex review of D4
/// (round 1) flagged the original raw-`f64` parse; round 2 flagged
/// `-0.0` slipping through `>= 0.0`, fixed by `is_sign_negative()`.
fn parse_finite_non_negative(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|_| format!("not a number: `{}`", s))?;
    if v.is_finite() && v >= 0.0 && !v.is_sign_negative() {
        Ok(v)
    } else {
        Err(format!("must be a finite number >= 0 (got `{}`)", s))
    }
}

/// Clap value parser for `jian perf compare`'s `--threshold`. A
/// regression-gate ratio belongs in `[0.0, 1.0]` — `0.0` means
/// "any positive delta regresses" (strictest), `1.0` means "fail
/// only when current is more than 2× baseline" (loosest). Anything
/// above 1.0 effectively disables the gate (codex round 2 NIT).
fn parse_threshold_ratio(s: &str) -> Result<f64, String> {
    let v = parse_finite_non_negative(s)?;
    if v <= 1.0 {
        Ok(v)
    } else {
        Err(format!(
            "threshold ratio must be in [0.0, 1.0] (got `{}`); use 0.15 for the canonical 15% gate",
            s
        ))
    }
}

mod commands;
mod diagnostic_render;
#[cfg(feature = "player")]
mod icon_loader;

#[derive(Parser, Debug)]
#[command(
    name = "jian",
    version,
    about = "Jian runtime CLI — check, pack, and scaffold .op files",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate a .op file: parse the schema, run Jian-extension compat
    /// checks, and print every diagnostic.
    Check(CheckArgs),
    /// Bundle a .op file + optional assets into a .op.pack zip archive.
    Pack(PackArgs),
    /// Unpack a .op.pack archive into a directory (inverse of `pack`).
    Unpack(UnpackArgs),
    /// Scaffold a new Jian project from an embedded template.
    New(NewArgs),
    /// Open a .op file in a desktop window and run its interactive
    /// pointer / scene pipeline (built with the `player` feature).
    #[cfg(feature = "player")]
    Player(PlayerArgs),
    /// Open a .op file like `player`, then watch the file and reload
    /// it on every save. Runtime state survives the reload.
    #[cfg(feature = "player")]
    Dev(DevArgs),
    /// Cold-start performance measurements. Subcommand `startup`
    /// runs the StartupDriver phase graph N times and prints a
    /// per-phase aggregated table (or JSON via `--format json`).
    Perf(PerfArgs),
}

#[derive(Parser, Debug)]
pub struct PerfArgs {
    #[command(subcommand)]
    pub cmd: PerfCommand,
}

#[derive(Subcommand, Debug)]
pub enum PerfCommand {
    /// Measure cold-start phase timings (Plan 19 Task 8).
    Startup(PerfStartupArgs),
    /// Diff two `jian perf startup --format json` outputs and gate
    /// the current run against a baseline (Plan 19 Task 19 / D4).
    /// Prints a per-metric delta table and exits non-zero when any
    /// tracked metric regresses by more than `--threshold` *and* the
    /// baseline is above the noise floor (`--noise-floor-ms`).
    Compare(PerfCompareArgs),
}

#[derive(Parser, Debug)]
pub struct PerfStartupArgs {
    pub path: PathBuf,
    /// Number of independent driver runs to aggregate. Min/median/p95
    /// are reported across all runs.
    #[arg(long, default_value_t = 10)]
    pub runs: usize,
    /// Output format: `table` (default, human-readable) or `json`.
    /// Validated at parse time — a typo fails before the run loop
    /// rather than silently defaulting and producing the wrong shape
    /// for a CI consumer.
    #[arg(long, value_enum, default_value_t = PerfFormat::Table)]
    pub format: PerfFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PerfFormat {
    Table,
    Json,
}

#[derive(Parser, Debug)]
pub struct PerfCompareArgs {
    /// Baseline JSON file (typically `main`'s artifact from the
    /// previous run of `jian perf startup --format json`).
    pub baseline: PathBuf,
    /// Current JSON file (the run under review).
    pub current: PathBuf,
    /// Regression threshold as a decimal ratio (`0.15` = 15%). Per-
    /// metric medians whose `(current - baseline) / baseline` exceeds
    /// this trip the gate. Must be in `[0.0, 1.0]` — `1e308`-style
    /// values would silently disable the gate (codex round 2 NIT).
    #[arg(long, default_value_t = 0.15, value_parser = parse_threshold_ratio)]
    pub threshold: f64,
    /// Noise floor in milliseconds. Metrics whose **baseline** median
    /// is below this value skip the threshold gate (timing noise on
    /// sub-millisecond samples produces unstable percentages); they
    /// still appear in the table but cannot fail the run. Must be
    /// finite and non-negative.
    #[arg(long, default_value_t = 1.0, value_parser = parse_finite_non_negative)]
    pub noise_floor_ms: f64,
    /// Output format: `table` (human-readable, default), `json`
    /// (structured), or `markdown` (PR-comment friendly).
    #[arg(long, value_enum, default_value_t = PerfCompareFormat::Table)]
    pub format: PerfCompareFormat,
    /// Override the platform label printed in the markdown / table
    /// header (e.g. `linux-x86_64`). Default: omitted.
    #[arg(long)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PerfCompareFormat {
    Table,
    Json,
    Markdown,
}

#[derive(Parser, Debug)]
pub struct CheckArgs {
    pub path: PathBuf,
    /// Emit diagnostics as NDJSON (one object per line).
    #[arg(long)]
    pub json: bool,
    /// Suppress the "OK, no diagnostics" success line so scripts can
    /// rely solely on the exit code (0 = clean, 1 = warnings, 2 =
    /// parse / semantic error). Warnings and errors are still printed
    /// — `--quiet` only silences the noise floor.
    #[arg(long, short = 'q')]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct PackArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Bundle every font file under `<input>/../assets/fonts/` into the
    /// archive at `assets/fonts/<original-filename>`. Recognised
    /// extensions: .ttf / .otf / .woff / .woff2. Missing directory is a
    /// no-op (not an error). Other files in the dir are skipped.
    #[arg(long)]
    pub include_fonts: bool,
    /// Bundle every image under `<input>/../assets/images/` into the
    /// archive at `assets/images/<blake3-128>.<ext>` — content-addressed
    /// (first 16 bytes of the BLAKE3 digest, 32 hex chars) so identical
    /// bytes dedupe to one archive entry. 128 bits comfortably outruns
    /// any pack's image count before a birthday collision risk. Missing
    /// directory is a no-op. Recognised extensions: .png / .jpg / .jpeg
    /// / .webp / .gif / .svg. Manifest exposes
    /// `images: { "<original-name>": "assets/images/<hash>.<ext>" }`
    /// so consumers can rewrite document references at load time.
    #[arg(long)]
    pub include_images: bool,
    /// Pre-compute the first-frame layout for `--aot-viewport` and
    /// embed it as `aot/initial_layout.bin` (Plan 19 D1 / Task 6).
    /// A reader that opens the pack at the same viewport can preload
    /// the rects and skip the runtime's `ComputeFirstLayout` pass.
    /// Off by default — AOT bytes only land when authors opt in.
    #[arg(long)]
    pub aot: bool,
    /// Default viewport for AOT-baked layout, in `WxH` form. Defaults
    /// to `800x600`. Ignored when `--aot` is unset.
    #[arg(long, default_value = "800x600")]
    pub aot_viewport: String,
}

#[derive(Parser, Debug)]
pub struct UnpackArgs {
    pub input: PathBuf,
    pub output_dir: PathBuf,
}

#[cfg(feature = "player")]
#[derive(Parser, Debug)]
pub struct PlayerArgs {
    pub path: PathBuf,
    /// Logical window size in `WxH` form. Defaults to 800x600.
    /// Mutually exclusive with `--fullscreen`.
    #[arg(long, conflicts_with = "fullscreen")]
    pub size: Option<String>,
    /// Override the window title. Defaults to the .op file's `app.name`
    /// when present, otherwise the path's file stem.
    #[arg(long)]
    pub title: Option<String>,
    /// Override the window icon. PNG file path; absolute or relative
    /// to the CWD. When unset, the runtime falls back to `app.icon`
    /// from the `.op` file (resolved relative to the `.op`'s
    /// directory). Pass `--icon=` to suppress both the override and
    /// the `app.icon` fallback for this run.
    #[arg(long)]
    pub icon: Option<PathBuf>,
    /// Open the window borderless-fullscreen on the current monitor.
    /// Mutually exclusive with `--size`.
    #[arg(long)]
    pub fullscreen: bool,
    /// Override the OS-reported DPI scale factor. Use 1.0 to force a
    /// non-HiDPI render on a Retina display, 2.0 to mimic Retina on a
    /// 1× monitor, etc. Must be > 0. When unset, follows the active
    /// monitor's reported scale and switches with the window.
    #[arg(long, value_parser = parse_positive_dpi)]
    pub dpi: Option<f64>,
    /// Render a developer HUD strip (size / scale / draw-op count)
    /// at the top-left corner of the window each frame. Off by
    /// default — flag-only, no value.
    #[arg(long = "debug-overlay")]
    pub debug_overlay: bool,
    /// Open a prod ASP (Agent Shell Protocol) listener on a
    /// local-only transport so an AI agent can drive the running
    /// app via `list_actions` + `tap` / `type` / `scroll` / `swipe`.
    ///
    /// Argument:
    /// - `auto` — pick a per-process default
    ///   (`$XDG_RUNTIME_DIR/jian/<pid>.asp.sock` on Unix,
    ///   `\\.\pipe\jian\<pid>\asp` on Windows).
    /// - `<path>` — bind to that filesystem path (Unix) or Named
    ///   Pipe (Windows starting `\\.\pipe\`).
    ///
    /// Network bind targets (TCP, `host:port`, URLs with a scheme)
    /// are rejected — prod ASP is local-only by design (Plan 18
    /// spec §6). Requires the `prod-asp` cargo feature.
    #[cfg(feature = "prod-asp")]
    #[arg(long = "asp")]
    pub asp: Option<String>,
}

#[cfg(feature = "player")]
#[derive(Parser, Debug)]
pub struct DevArgs {
    pub path: PathBuf,
    /// Mutually exclusive with `--fullscreen`.
    #[arg(long, conflicts_with = "fullscreen")]
    pub size: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    /// Override the window icon. PNG file path; absolute or relative
    /// to the CWD. Same semantics as `jian player --icon`.
    #[arg(long)]
    pub icon: Option<PathBuf>,
    /// Open the window borderless-fullscreen on the current monitor.
    /// Mutually exclusive with `--size`.
    #[arg(long)]
    pub fullscreen: bool,
    /// Same as `jian player --dpi`. Must be > 0.
    #[arg(long, value_parser = parse_positive_dpi)]
    pub dpi: Option<f64>,
    /// Same as `jian player --debug-overlay`.
    #[arg(long = "debug-overlay")]
    pub debug_overlay: bool,
    /// Open a stdio MCP server on this process's stdin/stdout while
    /// the window is running. AI clients can drive `tools/list` /
    /// `tools/call` against the live, hot-reloading document.
    /// Requires the `mcp` cargo feature.
    #[cfg(feature = "mcp")]
    #[arg(long, default_value_t = false)]
    pub mcp: bool,
}

#[derive(Parser, Debug)]
pub struct NewArgs {
    /// Project name — also used as the app id and directory name.
    pub name: String,
    /// Which embedded template to scaffold from. Default: `counter`.
    #[arg(long, default_value = "counter")]
    pub template: String,
    /// Directory to create the project in. Default: `./<name>`.
    #[arg(long)]
    pub path: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Command::Check(args) => commands::check::run(args),
        Command::Pack(args) => commands::pack::run(args),
        Command::Unpack(args) => commands::unpack::run(args),
        Command::New(args) => commands::new::run(args),
        #[cfg(feature = "player")]
        Command::Player(args) => commands::player::run(args),
        #[cfg(feature = "player")]
        Command::Dev(args) => commands::dev::run(args),
        Command::Perf(args) => match args.cmd {
            PerfCommand::Startup(a) => commands::perf::run(a),
            PerfCommand::Compare(a) => commands::perf::run_compare(a),
        },
    };

    match result {
        Ok(exit) => exit,
        Err(e) => {
            eprintln!("jian: error: {:#}", e);
            ExitCode::from(2)
        }
    }
}

// Both clap value parsers' tests share one module placed after
// `fn main` so clippy's `items_after_test_module` lint doesn't fire.
// `parse_positive_dpi` only exists under `feature = "player"`, so its
// tests are individually gated; `parse_finite_non_negative` is
// always-on (used by the `Compare` subcommand which lives outside the
// `player` feature gate) and its tests are unconditional.
#[cfg(test)]
mod parser_tests {
    use super::*;

    #[cfg(feature = "player")]
    #[test]
    fn parse_positive_dpi_accepts_typical_values() {
        assert_eq!(parse_positive_dpi("1.0").unwrap(), 1.0);
        assert_eq!(parse_positive_dpi("2").unwrap(), 2.0);
        assert_eq!(parse_positive_dpi("1.5").unwrap(), 1.5);
        assert_eq!(parse_positive_dpi("0.5").unwrap(), 0.5);
    }

    #[cfg(feature = "player")]
    #[test]
    fn parse_positive_dpi_rejects_zero_and_negative() {
        assert!(parse_positive_dpi("0").is_err());
        assert!(parse_positive_dpi("0.0").is_err());
        assert!(parse_positive_dpi("-1.5").is_err());
    }

    #[cfg(feature = "player")]
    #[test]
    fn parse_positive_dpi_rejects_non_finite() {
        assert!(parse_positive_dpi("nan").is_err());
        assert!(parse_positive_dpi("inf").is_err());
        assert!(parse_positive_dpi("not-a-number").is_err());
    }

    #[test]
    fn parse_finite_non_negative_accepts_zero_and_positive() {
        assert_eq!(parse_finite_non_negative("0").unwrap(), 0.0);
        assert_eq!(parse_finite_non_negative("0.0").unwrap(), 0.0);
        assert_eq!(parse_finite_non_negative("0.15").unwrap(), 0.15);
        assert_eq!(parse_finite_non_negative("1").unwrap(), 1.0);
        assert_eq!(parse_finite_non_negative("2.5").unwrap(), 2.5);
    }

    #[test]
    fn parse_finite_non_negative_rejects_negative_and_non_finite() {
        assert!(parse_finite_non_negative("-1").is_err());
        assert!(parse_finite_non_negative("-0.0001").is_err());
        // Negative zero parses to a value where `>= 0.0` is `true`,
        // so the predicate must also reject `is_sign_negative()`.
        // Codex round 2 NIT.
        assert!(parse_finite_non_negative("-0").is_err());
        assert!(parse_finite_non_negative("-0.0").is_err());
        assert!(parse_finite_non_negative("nan").is_err());
        assert!(parse_finite_non_negative("inf").is_err());
        assert!(parse_finite_non_negative("not-a-number").is_err());
    }

    #[test]
    fn parse_threshold_ratio_accepts_canonical_range() {
        assert_eq!(parse_threshold_ratio("0").unwrap(), 0.0);
        assert_eq!(parse_threshold_ratio("0.15").unwrap(), 0.15);
        assert_eq!(parse_threshold_ratio("1").unwrap(), 1.0);
        assert_eq!(parse_threshold_ratio("1.0").unwrap(), 1.0);
    }

    #[test]
    fn parse_threshold_ratio_rejects_above_one_and_negative() {
        // Above 1.0 effectively disables the gate (codex round 2 NIT).
        assert!(parse_threshold_ratio("1.0001").is_err());
        assert!(parse_threshold_ratio("100").is_err());
        assert!(parse_threshold_ratio("1e308").is_err());
        // The non-negative + finiteness checks still apply.
        assert!(parse_threshold_ratio("-0.1").is_err());
        assert!(parse_threshold_ratio("nan").is_err());
        assert!(parse_threshold_ratio("inf").is_err());
    }
}
