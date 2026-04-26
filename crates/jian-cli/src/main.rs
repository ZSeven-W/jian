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

mod commands;

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
}

#[derive(Parser, Debug)]
pub struct PackArgs {
    pub input: PathBuf,
    pub output: PathBuf,
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
    #[arg(long)]
    pub size: Option<String>,
    /// Override the window title. Defaults to the .op file's `app.name`
    /// when present, otherwise the path's file stem.
    #[arg(long)]
    pub title: Option<String>,
}

#[cfg(feature = "player")]
#[derive(Parser, Debug)]
pub struct DevArgs {
    pub path: PathBuf,
    #[arg(long)]
    pub size: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
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
