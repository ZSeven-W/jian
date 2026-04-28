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

mod commands;
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
    /// archive at `assets/images/<blake3-hash>.<ext>` — content-addressed
    /// so identical bytes dedupe. Missing directory is a no-op.
    /// Recognised extensions: .png / .jpg / .jpeg / .webp / .gif / .svg.
    /// Manifest exposes `images: { "<original-name>": "assets/images/<hash>.<ext>" }`
    /// so consumers can rewrite document references at load time.
    #[arg(long)]
    pub include_images: bool,
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

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn parse_positive_dpi_accepts_typical_values() {
        assert_eq!(parse_positive_dpi("1.0").unwrap(), 1.0);
        assert_eq!(parse_positive_dpi("2").unwrap(), 2.0);
        assert_eq!(parse_positive_dpi("1.5").unwrap(), 1.5);
        assert_eq!(parse_positive_dpi("0.5").unwrap(), 0.5);
    }

    #[test]
    fn parse_positive_dpi_rejects_zero_and_negative() {
        assert!(parse_positive_dpi("0").is_err());
        assert!(parse_positive_dpi("0.0").is_err());
        assert!(parse_positive_dpi("-1.5").is_err());
    }

    #[test]
    fn parse_positive_dpi_rejects_non_finite() {
        assert!(parse_positive_dpi("nan").is_err());
        assert!(parse_positive_dpi("inf").is_err());
        assert!(parse_positive_dpi("not-a-number").is_err());
    }
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
