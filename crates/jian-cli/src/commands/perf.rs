//! `jian perf startup` — measure cold-start phase timings (Plan 19 Task 8).
//!
//! Loads the `.op`, builds a [`StartupDriver`] with no-op phase impls
//! for every variant in [`StartupPhase::ALL`], runs it N times, and
//! prints a per-phase aggregated table (or JSON when `--format json`).
//!
//! ### What this measures today
//!
//! Phase impls are currently `futures::future::ready(Ok(()))` placeholders
//! — Plan 19 Tasks 2-7 land the real Runtime-coupled implementations
//! (eager GPU init, lazy expression compile, font subset, visible-only
//! spatial, etc.) over multiple commits. Until each phase has a real
//! impl, the column shows the **framework's own overhead** — the cost
//! of the layered `FuturesUnordered` dispatch, the per-phase
//! `PhaseTiming` allocation, and the `Instant::now()` deltas. That's
//! genuinely useful as a regression guard (a sudden 10× jump signals a
//! scheduler bug) but it is not yet a representative cold-start cost.
//!
//! When real phase impls are registered, this command's output becomes
//! the canonical "did we hit the C19 budget?" answer. The CLI surface
//! and the aggregator are stable; only the registered phase closures
//! change.
//!
//! ### Aggregation across N runs
//!
//! With `--runs N` the command runs N independent driver instances and
//! collects per-phase `duration_ms` samples. The output reports min,
//! median, and p95 per phase (skipping mean — p95 is the more robust
//! signal under noisy hardware). Total wall-clock and critical-path
//! aggregates use the same statistics.

use crate::{PerfFormat, PerfStartupArgs};
use anyhow::{Context, Result};
use jian_core::startup::{
    PhaseResult, StartupConfig, StartupDriver, StartupPhase, StartupReport,
};
#[cfg(test)]
use jian_core::startup::PhaseTiming;
use std::collections::BTreeMap;
use std::process::ExitCode;

pub fn run(args: PerfStartupArgs) -> Result<ExitCode> {
    // Verify the .op parses up-front so a typo doesn't waste N runs of
    // measurement before erroring out. We don't actually use the loaded
    // document yet — phase impls are no-ops — but the parse cost is on
    // the cold-start critical path so it's the right thing to check.
    let src = std::fs::read_to_string(&args.path)
        .with_context(|| format!("read {}", args.path.display()))?;
    jian_ops_schema::load_str(&src)
        .with_context(|| format!("parse {}", args.path.display()))?;

    let runs = args.runs.max(1);
    let mut reports: Vec<StartupReport> = Vec::with_capacity(runs);
    for _ in 0..runs {
        let mut driver = StartupDriver::new();
        for phase in StartupPhase::ALL {
            driver.register(*phase, || async { PhaseResult::Ok(()) });
        }
        let report = futures::executor::block_on(driver.run(StartupConfig::default()))
            .map_err(|e| anyhow::anyhow!("startup driver run: {e}"))?;
        reports.push(report);
    }

    let agg = aggregate(&reports);

    match args.format {
        PerfFormat::Json => {
            let payload = agg.to_json(runs);
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        PerfFormat::Table => {
            print!("{}", agg.pretty(runs));
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Aggregated samples for a single phase.
#[derive(Debug, Clone)]
struct PhaseAgg {
    phase: StartupPhase,
    on_critical: bool,
    durations_ms: Vec<f64>,
}

impl PhaseAgg {
    fn min(&self) -> f64 {
        self.durations_ms
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min)
    }

    fn median(&self) -> f64 {
        let mut sorted = self.durations_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = sorted.len();
        if n == 0 {
            0.0
        } else if n % 2 == 1 {
            sorted[n / 2]
        } else {
            (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
        }
    }

    /// 95th-percentile duration via the **nearest-rank** method:
    /// `index = ceil(0.95 * N) - 1`. For N=1 that's 0 (the single
    /// sample); for N=20 that's 18; for N=100 that's 94. Codex round
    /// 1 noted the previous `ceil(0.95 * (N-1))` formula was
    /// non-standard and produced max-pinned values for N=20 — fixed
    /// to match the textbook nearest-rank percentile.
    fn p95(&self) -> f64 {
        if self.durations_ms.is_empty() {
            return 0.0;
        }
        let mut sorted = self.durations_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[nearest_rank_index(sorted.len(), 0.95)]
    }
}

/// Nearest-rank percentile index: `ceil(p * N) - 1`, clamped to
/// `[0, N-1]`. `p` is in `[0, 1]`. `N` must be positive (caller
/// short-circuits the empty case before calling).
fn nearest_rank_index(n: usize, p: f64) -> usize {
    debug_assert!(n > 0);
    debug_assert!((0.0..=1.0).contains(&p));
    // ceil(p*N) is at least 1 for p>0 and N>=1 because both terms are
    // strictly positive; subtract 1 for zero-based indexing. The
    // saturating_sub guards p=0, where ceil(0*N)=0 would otherwise
    // unsigned-underflow the subtraction; in that case we want index 0.
    ((p * n as f64).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1)
}

#[derive(Debug, Clone)]
struct StartupAgg {
    phases: Vec<PhaseAgg>,
    first_interactive_ms_samples: Vec<f64>,
    critical_path_ms_samples: Vec<f64>,
    wall_clock_ms_samples: Vec<f64>,
}

impl StartupAgg {
    fn pretty(&self, runs: usize) -> String {
        use std::fmt::Write as _;
        let phase_w = self
            .phases
            .iter()
            .map(|p| p.phase.as_str().len())
            .max()
            .unwrap_or(5)
            .max("Phase".len());

        let mut out = String::new();
        let _ = writeln!(out, "jian perf startup — {runs} run(s)\n");
        let _ = writeln!(
            out,
            "{:width$} │ Min ms │ Med ms │ p95 ms │ Critical",
            "Phase",
            width = phase_w
        );
        let _ = writeln!(
            out,
            "{:─<width$}─┼────────┼────────┼────────┼─────────",
            "",
            width = phase_w
        );
        for p in &self.phases {
            let mark = if p.on_critical { "✓" } else { " " };
            let _ = writeln!(
                out,
                "{name:width$} │ {min:>6.2} │ {med:>6.2} │ {p95:>6.2} │    {mark}",
                name = p.phase.as_str(),
                width = phase_w,
                min = p.min(),
                med = p.median(),
                p95 = p.p95(),
                mark = mark,
            );
        }
        let _ = writeln!(
            out,
            "{:─<width$}─┴────────┴────────┴────────┴─────────",
            "",
            width = phase_w
        );
        let _ = writeln!(
            out,
            "First interactive (median):   {:.2} ms",
            median(&self.first_interactive_ms_samples)
        );
        let _ = writeln!(
            out,
            "Critical path serial (median): {:.2} ms",
            median(&self.critical_path_ms_samples)
        );
        let _ = writeln!(
            out,
            "Wall clock (median):           {:.2} ms",
            median(&self.wall_clock_ms_samples)
        );
        out
    }

    fn to_json(&self, runs: usize) -> serde_json::Value {
        let phases: Vec<_> = self
            .phases
            .iter()
            .map(|p| {
                serde_json::json!({
                    "phase": p.phase.as_str(),
                    "on_critical": p.on_critical,
                    "min_ms":    p.min(),
                    "median_ms": p.median(),
                    "p95_ms":    p.p95(),
                })
            })
            .collect();
        serde_json::json!({
            "runs": runs,
            "phases": phases,
            "first_interactive_ms": {
                "median": median(&self.first_interactive_ms_samples),
                "min":    min_f64(&self.first_interactive_ms_samples),
                "p95":    p95(&self.first_interactive_ms_samples),
            },
            "critical_path_ms": {
                "median": median(&self.critical_path_ms_samples),
                "min":    min_f64(&self.critical_path_ms_samples),
                "p95":    p95(&self.critical_path_ms_samples),
            },
            "wall_clock_ms": {
                "median": median(&self.wall_clock_ms_samples),
                "min":    min_f64(&self.wall_clock_ms_samples),
                "p95":    p95(&self.wall_clock_ms_samples),
            },
        })
    }
}

fn aggregate(reports: &[StartupReport]) -> StartupAgg {
    // Collect per-phase samples.
    let mut by_phase: BTreeMap<usize, PhaseAgg> = BTreeMap::new();
    for report in reports {
        for timing in &report.phases {
            let key = StartupPhase::ALL
                .iter()
                .position(|p| p == &timing.phase)
                .unwrap_or(usize::MAX);
            let entry = by_phase.entry(key).or_insert_with(|| PhaseAgg {
                phase: timing.phase,
                on_critical: timing.on_critical,
                durations_ms: Vec::with_capacity(reports.len()),
            });
            entry.durations_ms.push(timing.duration_ms);
        }
    }

    // Materialise ordered by ALL declaration so the table reads
    // top-down through the cold-start pipeline.
    let phases: Vec<PhaseAgg> = by_phase.into_values().collect();

    StartupAgg {
        phases,
        first_interactive_ms_samples: collect(reports, |r| r.first_interactive_ms),
        critical_path_ms_samples: collect(reports, |r| r.critical_path_ms()),
        wall_clock_ms_samples: collect(reports, |r| r.total_wall_clock_ms()),
    }
}

fn collect<F>(reports: &[StartupReport], f: F) -> Vec<f64>
where
    F: Fn(&StartupReport) -> f64,
{
    reports.iter().map(f).collect()
}

fn median(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

fn min_f64(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(f64::INFINITY, f64::min)
}

fn p95(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s[nearest_rank_index(s.len(), 0.95)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_with(phases: &[(StartupPhase, f64)]) -> StartupReport {
        let mut report = StartupReport::default();
        for (phase, duration_ms) in phases {
            report.phases.push(PhaseTiming {
                phase: *phase,
                started_at_ms: 0.0,
                duration_ms: *duration_ms,
                on_critical: phase.is_critical(),
                notes: None,
            });
        }
        report
    }

    #[test]
    fn aggregate_collects_durations_per_phase() {
        let r1 = report_with(&[(StartupPhase::ReadFile, 1.0), (StartupPhase::ParseSchema, 5.0)]);
        let r2 = report_with(&[(StartupPhase::ReadFile, 3.0), (StartupPhase::ParseSchema, 7.0)]);
        let agg = aggregate(&[r1, r2]);
        let read = agg.phases.iter().find(|p| p.phase == StartupPhase::ReadFile).unwrap();
        assert_eq!(read.durations_ms, vec![1.0, 3.0]);
        let parse = agg
            .phases
            .iter()
            .find(|p| p.phase == StartupPhase::ParseSchema)
            .unwrap();
        assert_eq!(parse.durations_ms, vec![5.0, 7.0]);
    }

    #[test]
    fn phase_agg_min_median_p95_match_expected() {
        let p = PhaseAgg {
            phase: StartupPhase::ReadFile,
            on_critical: true,
            durations_ms: vec![10.0, 20.0, 30.0, 40.0, 100.0],
        };
        assert_eq!(p.min(), 10.0);
        assert_eq!(p.median(), 30.0);
        // Nearest-rank p95 with N=5: ceil(0.95*5)-1 = ceil(4.75)-1
        // = 5-1 = 4 → samples[4] = 100.0.
        assert_eq!(p.p95(), 100.0);
    }

    #[test]
    fn nearest_rank_index_matches_textbook() {
        // N=1: only sample.
        assert_eq!(nearest_rank_index(1, 0.95), 0);
        // N=20: ceil(0.95*20)-1 = ceil(19.0)-1 = 19-1 = 18 (NOT 19,
        // which the previous ceil(0.95*(N-1)) formula produced).
        assert_eq!(nearest_rank_index(20, 0.95), 18);
        // N=100: ceil(0.95*100)-1 = 95-1 = 94.
        assert_eq!(nearest_rank_index(100, 0.95), 94);
        // Median (p=0.5) sanity check.
        assert_eq!(nearest_rank_index(10, 0.5), 4);
    }

    #[test]
    fn phase_agg_median_handles_even_length() {
        let p = PhaseAgg {
            phase: StartupPhase::ReadFile,
            on_critical: true,
            durations_ms: vec![10.0, 20.0, 30.0, 40.0],
        };
        assert_eq!(p.median(), 25.0);
    }

    #[test]
    fn phase_agg_p95_with_single_sample_equals_that_sample() {
        let p = PhaseAgg {
            phase: StartupPhase::ReadFile,
            on_critical: true,
            durations_ms: vec![42.0],
        };
        assert_eq!(p.p95(), 42.0);
    }

    #[test]
    fn empty_report_list_produces_empty_aggregate() {
        let agg = aggregate(&[]);
        assert!(agg.phases.is_empty());
        assert!(agg.first_interactive_ms_samples.is_empty());
    }

    #[test]
    fn pretty_table_includes_every_run_phase() {
        let r = report_with(&[
            (StartupPhase::ReadFile, 1.5),
            (StartupPhase::ParseSchema, 3.2),
            (StartupPhase::DecodeImages, 50.0),
        ]);
        let agg = aggregate(&[r]);
        let table = agg.pretty(1);
        for phase in [
            StartupPhase::ReadFile,
            StartupPhase::ParseSchema,
            StartupPhase::DecodeImages,
        ] {
            assert!(
                table.contains(phase.as_str()),
                "table missing {phase:?}: {table}"
            );
        }
        assert!(table.contains("First interactive"));
        assert!(table.contains("Critical path"));
        assert!(table.contains("Wall clock"));
    }

    #[test]
    fn json_output_contains_runs_and_phase_summaries() {
        let r = report_with(&[(StartupPhase::ReadFile, 2.0)]);
        let agg = aggregate(&[r]);
        let v = agg.to_json(1);
        assert_eq!(v["runs"], 1);
        let phases = v["phases"].as_array().unwrap();
        assert_eq!(phases.len(), 1);
        let read = &phases[0];
        assert_eq!(read["phase"], "ReadFile");
        assert_eq!(read["on_critical"], true);
        assert_eq!(read["min_ms"], 2.0);
    }

    #[test]
    fn p95_helper_with_empty_samples_returns_zero() {
        assert_eq!(p95(&[]), 0.0);
    }

    #[test]
    fn median_helper_with_empty_samples_returns_zero() {
        assert_eq!(median(&[]), 0.0);
    }
}
