//! Cold-start phase graph + driver + per-phase timing report.
//!
//! See `superpowers/plans/2026-04-17-jian-plan-19-cold-start-optimization.md`
//! for the full design (C19 budgets, parallelism rules, future tasks).
//!
//! Task 1 (this commit) lands the foundation: an enum of phases with a
//! topological dependency graph, a layered async driver, and a timing
//! report. Later Plan 19 tasks register the real Runtime-coupled
//! implementations on top.

mod driver;
mod phase;
mod report;

pub use driver::{PhaseResult, StartupConfig, StartupDriver, StartupError};
pub use phase::StartupPhase;
pub use report::{PhaseTiming, StartupReport};
