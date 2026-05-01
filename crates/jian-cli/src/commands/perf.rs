//! `jian perf startup` — measure cold-start phase timings (Plan 19 Task 8).
//!
//! Loads the `.op`, builds a [`StartupDriver`] with the host-agnostic
//! [`HostAgnosticBootstrap`] DataPath impls (Plan 19 capstone B1), runs
//! it N times, and prints a per-phase aggregated table (or JSON when
//! `--format json`).
//!
//! ### What this measures today
//!
//! `DataPath` phases (ReadFile / ParseSchema / SeedStateGraph /
//! BuildNodeTree / InitGpuContext / LoadCoreFonts /
//! ComputeFirstLayout / BuildVisibleSpatial) record real Runtime-
//! coupled work since B1 — file I/O, schema parsing, state seeding,
//! layout, visible-only spatial. The numbers are representative of a
//! single-process headless cold-start.
//!
//! `Visual` phases (RenderSplash / RenderFirstFrame / PresentToSurface
//! / EventPumpReady) and `Background` phases (BuildFullSpatial /
//! LoadRemainingFonts / DecodeImages) stay no-op'd in this command:
//! `jian perf startup` is intentionally headless (no winit, no real
//! draw surface) and Background phases need data the Visual stage
//! produces. Their entries surface in the report so the column shape
//! is stable, but the timings reflect framework overhead only.
//!
//! ### Aggregation across N runs
//!
//! With `--runs N` the command runs N independent driver instances and
//! collects per-phase `duration_ms` samples. The output reports min,
//! median, and p95 per phase (skipping mean — p95 is the more robust
//! signal under noisy hardware). Total wall-clock and critical-path
//! aggregates use the same statistics.

use crate::{PerfCompareArgs, PerfCompareFormat, PerfFormat, PerfStartupArgs};
use anyhow::{Context, Result};
#[cfg(test)]
use jian_core::startup::PhaseTiming;
use jian_core::startup::{
    BootstrapSource, HostAgnosticBootstrap, PhaseResult, StartupConfig, StartupDriver,
    StartupPhase, StartupReport, StartupStage,
};
use jian_ops_schema::document::PenDocument;
use std::collections::BTreeMap;
use std::process::ExitCode;

pub fn run(args: PerfStartupArgs) -> Result<ExitCode> {
    // Parse once up front so a typo doesn't waste N runs of
    // measurement. The parsed schema is reused across runs to keep
    // the harness focused on the runtime-coupled cost rather than
    // re-paying the disk-read tax every iteration. (`ReadFile` /
    // `ParseSchema` still record measurable timings inside the
    // bootstrap because the `BootstrapSource::Schema` short-circuit
    // is the *correct* answer for the perf harness — it skips disk
    // I/O the user already paid for once and surfaces the rest of
    // the data path's cost honestly.)
    let src = std::fs::read_to_string(&args.path)
        .with_context(|| format!("read {}", args.path.display()))?;
    let schema: PenDocument = jian_ops_schema::load_str(&src)
        .with_context(|| format!("parse {}", args.path.display()))?
        .value;

    // Viewport: prefer the schema's root-frame extents (same heuristic
    // `jian player` uses) so different fixtures are measured under
    // their representative dimensions. Falls back to 800×600 when the
    // root node has no explicit size — picks a reasonable default
    // rather than zero rects that would skew layout / spatial timings.
    // (Codex review of B3, MEDIUM: a hardcoded viewport biases
    // ComputeFirstLayout / BuildVisibleSpatial across diverse fixtures.)
    let viewport = root_frame_size(&schema).unwrap_or((800.0, 600.0));

    let runs = args.runs.max(1);
    let mut reports: Vec<StartupReport> = Vec::with_capacity(runs);
    for _ in 0..runs {
        let mut driver = StartupDriver::new();
        // DataPath: real impls via the bootstrap. The handles are
        // dropped at end-of-iteration — the perf harness measures
        // the construction cost, not the ongoing runtime.
        let _handles = HostAgnosticBootstrap::install_data_path(
            &mut driver,
            BootstrapSource::Schema(Box::new(schema.clone())),
            viewport,
        );
        // Visual + Background: no-ops for headless measurement.
        // The perf command intentionally skips real visual work
        // (Plan 19 capstone B2.2 / B4 ship those for the desktop
        // host); their timing entries record framework overhead so
        // the report's column shape stays uniform across runs.
        for phase in StartupPhase::ALL {
            if phase.stage() != StartupStage::DataPath {
                driver.register(*phase, || async { PhaseResult::Ok(()) });
            }
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

/// Read the first root-node's declared `width` × `height` from the
/// parsed schema. Mirrors `jian player`'s `root_frame_size` so the
/// perf harness measures `ComputeFirstLayout` / `BuildVisibleSpatial`
/// under each fixture's intended dimensions. Falls back to `None`
/// when the root has no explicit numeric size (the caller picks a
/// platform default in that case).
fn root_frame_size(schema: &PenDocument) -> Option<(f32, f32)> {
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

// ──────────────────────────────────────────────────────────────────
// `jian perf compare` — diff two startup-budget JSON outputs and gate
// the current run against a baseline (Plan 19 Task 19 / D4).
// ──────────────────────────────────────────────────────────────────

/// One metric's baseline → current pair, with the relative delta.
/// `delta_ratio = (current - baseline) / baseline` when baseline > 0;
/// `0.0` when baseline is exactly 0 (avoids dividing by zero — the
/// noise floor handles small-but-nonzero baselines).
#[derive(Debug, Clone)]
struct DiffRow {
    metric: String,
    baseline_ms: f64,
    current_ms: f64,
    delta_ratio: f64,
    /// `true` when the baseline median is below the user-specified
    /// noise floor; the row stays in the table but is exempt from
    /// the threshold gate.
    below_noise_floor: bool,
    /// `true` when this row's `delta_ratio` exceeds the threshold
    /// AND `below_noise_floor` is false. Only rows with this flag
    /// fail the gate.
    regressed: bool,
}

/// Aggregated diff used by every output formatter. Holds per-phase
/// `median_ms` deltas plus the three roll-up metrics (`first_interactive`,
/// `critical_path`, `wall_clock`). Built by [`build_diff`]; rendered by
/// [`render_table`] / [`render_markdown`] / [`render_json`].
///
/// `missing_rollups` records any of the three mandatory roll-up
/// medians (first_interactive / critical_path / wall_clock) that
/// couldn't be parsed from either JSON. A non-empty list means the
/// input is structurally broken — `run_compare` fails on this case
/// rather than silently exiting green with an empty table (codex
/// round 2 HIGH).
#[derive(Debug, Clone)]
struct Diff {
    rows: Vec<DiffRow>,
    threshold: f64,
    noise_floor_ms: f64,
    missing_rollups: Vec<&'static str>,
}

impl Diff {
    fn any_regressed(&self) -> bool {
        self.rows.iter().any(|r| r.regressed)
    }

    /// `true` when the comparison can't be trusted: either no rows
    /// matched at all, or one of the mandatory roll-up medians is
    /// absent from the inputs. The CLI exits non-zero on this case
    /// so a corrupt JSON can't silently exit green.
    fn is_structurally_broken(&self) -> bool {
        self.rows.is_empty() || !self.missing_rollups.is_empty()
    }
}

pub fn run_compare(args: PerfCompareArgs) -> Result<ExitCode> {
    let baseline_text = std::fs::read_to_string(&args.baseline)
        .with_context(|| format!("read baseline {}", args.baseline.display()))?;
    let current_text = std::fs::read_to_string(&args.current)
        .with_context(|| format!("read current {}", args.current.display()))?;
    let baseline: serde_json::Value =
        serde_json::from_str(&baseline_text).context("parse baseline JSON")?;
    let current: serde_json::Value =
        serde_json::from_str(&current_text).context("parse current JSON")?;

    let diff = build_diff(&baseline, &current, args.threshold, args.noise_floor_ms);

    match args.format {
        PerfCompareFormat::Table => print!("{}", render_table(&diff, args.label.as_deref())),
        PerfCompareFormat::Markdown => print!("{}", render_markdown(&diff, args.label.as_deref())),
        PerfCompareFormat::Json => {
            let v = render_json(&diff, args.label.as_deref());
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
    }

    // Fail loudly on structurally broken inputs (codex round 2 HIGH):
    // a malformed JSON that produces zero comparable rows or omits a
    // mandatory roll-up median must NOT exit green. The diff text the
    // user just saw still helps them debug.
    if diff.is_structurally_broken() {
        eprintln!(
            "jian perf compare: structurally broken input — {} row(s), missing rollups: {:?}",
            diff.rows.len(),
            diff.missing_rollups
        );
        return Ok(ExitCode::from(2));
    }

    Ok(if diff.any_regressed() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// Build a [`Diff`] by walking every phase in `current` and matching
/// against the same-named phase in `baseline`. Phases present in
/// `baseline` but missing from `current` (or vice versa) are skipped
/// — `jian perf startup`'s phase set is stable across runs, so a
/// missing phase is a CI bug we'd rather surface as a noisy
/// passthrough than a confusing partial regression.
///
/// The three roll-up metrics (first-interactive / critical-path /
/// wall-clock medians) round out the per-phase rows; total wall
/// clock is the single most actionable number a reviewer reads.
fn build_diff(
    baseline: &serde_json::Value,
    current: &serde_json::Value,
    threshold: f64,
    noise_floor_ms: f64,
) -> Diff {
    let mut rows = Vec::new();

    // Per-phase median deltas. `phases` is an array; index by `phase`
    // string so a CI mismatch on phase order doesn't desync the diff.
    let baseline_phases = baseline
        .get("phases")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let current_phases = current
        .get("phases")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for cp in &current_phases {
        let Some(name) = cp.get("phase").and_then(|v| v.as_str()).map(str::to_owned) else {
            continue;
        };
        // Both sides treated symmetrically: a missing or non-numeric
        // `median_ms` on EITHER side skips the row. The earlier
        // `unwrap_or(0.0)` on the current side could turn a broken
        // tool output into a fake "improved" delta; codex round 1
        // MEDIUM. The phase-coverage sanity guard in the budget tests
        // (D3) catches actual missing-phase regressions; this loop's
        // job is just to compute deltas where both sides agree.
        let Some(cur_med) = cp.get("median_ms").and_then(|v| v.as_f64()) else {
            continue;
        };
        let Some(base_med) = baseline_phases
            .iter()
            .find(|bp| bp.get("phase").and_then(|v| v.as_str()) == Some(&name))
            .and_then(|bp| bp.get("median_ms").and_then(|v| v.as_f64()))
        else {
            continue;
        };
        rows.push(make_row(name, base_med, cur_med, threshold, noise_floor_ms));
    }

    // Roll-up medians. Order matches what a reviewer reads top-down:
    // first-interactive (the user-visible signal) before the internal
    // critical-path / wall-clock totals. These three are mandatory in
    // any well-formed `jian perf startup --format json` output — a
    // missing one signals structurally broken input (codex round 2
    // HIGH). Track absence in `missing_rollups` so `run_compare` can
    // exit non-zero rather than silently surfacing zero rows.
    let mut missing_rollups: Vec<&'static str> = Vec::new();
    for (key, label) in [
        ("first_interactive_ms", "first_interactive (median)"),
        ("critical_path_ms", "critical_path (median)"),
        ("wall_clock_ms", "wall_clock (median)"),
    ] {
        let base = baseline
            .get(key)
            .and_then(|v| v.get("median"))
            .and_then(|v| v.as_f64());
        let cur = current
            .get(key)
            .and_then(|v| v.get("median"))
            .and_then(|v| v.as_f64());
        match (base, cur) {
            (Some(b), Some(c)) => {
                rows.push(make_row(label.to_owned(), b, c, threshold, noise_floor_ms));
            }
            _ => missing_rollups.push(key),
        }
    }

    Diff {
        rows,
        threshold,
        noise_floor_ms,
        missing_rollups,
    }
}

fn make_row(
    metric: String,
    baseline_ms: f64,
    current_ms: f64,
    threshold: f64,
    noise_floor_ms: f64,
) -> DiffRow {
    let delta_ratio = if baseline_ms > 0.0 {
        (current_ms - baseline_ms) / baseline_ms
    } else {
        0.0
    };
    let below_noise_floor = baseline_ms < noise_floor_ms;
    let regressed = !below_noise_floor && delta_ratio > threshold;
    DiffRow {
        metric,
        baseline_ms,
        current_ms,
        delta_ratio,
        below_noise_floor,
        regressed,
    }
}

fn render_table(diff: &Diff, label: Option<&str>) -> String {
    use std::fmt::Write as _;
    let metric_w = diff
        .rows
        .iter()
        .map(|r| r.metric.len())
        .max()
        .unwrap_or(6)
        .max("Metric".len());
    let mut out = String::new();
    if let Some(lbl) = label {
        let _ = writeln!(out, "jian perf compare — {lbl}");
    } else {
        let _ = writeln!(out, "jian perf compare");
    }
    let _ = writeln!(
        out,
        "threshold: {:.0}%   noise floor: {:.2} ms\n",
        diff.threshold * 100.0,
        diff.noise_floor_ms
    );
    let _ = writeln!(
        out,
        "{:width$} │ Baseline ms │ Current ms │   Δ%    │ Status",
        "Metric",
        width = metric_w
    );
    let _ = writeln!(
        out,
        "{:─<width$}─┼─────────────┼────────────┼─────────┼────────",
        "",
        width = metric_w
    );
    for row in &diff.rows {
        let status = row_status(row);
        let _ = writeln!(
            out,
            "{name:width$} │ {b:>11.3} │ {c:>10.3} │ {d:>+6.1}% │ {s}",
            name = row.metric,
            width = metric_w,
            b = row.baseline_ms,
            c = row.current_ms,
            d = row.delta_ratio * 100.0,
            s = status,
        );
    }
    let _ = writeln!(
        out,
        "{:─<width$}─┴─────────────┴────────────┴─────────┴────────",
        "",
        width = metric_w
    );
    // Verdict order: structural breakage first (it overrides
    // regression analysis — the inputs aren't trustworthy), then
    // regression, then green. Codex round 3 MEDIUM: the markdown
    // rendering used to print "All metrics within threshold" even on
    // empty / corrupt input.
    if diff.is_structurally_broken() {
        let _ = writeln!(
            out,
            "BROKEN — structurally invalid input ({} row(s); missing rollups: {:?})",
            diff.rows.len(),
            diff.missing_rollups
        );
    } else if diff.any_regressed() {
        let _ = writeln!(
            out,
            "REGRESSED — {} metric(s) over threshold",
            diff.rows.iter().filter(|r| r.regressed).count()
        );
    } else {
        let _ = writeln!(out, "OK — all metrics within threshold");
    }
    out
}

fn render_markdown(diff: &Diff, label: Option<&str>) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let header = match label {
        Some(lbl) => format!("### startup-budget — {lbl}"),
        None => "### startup-budget".to_owned(),
    };
    let _ = writeln!(out, "{header}");
    let _ = writeln!(
        out,
        "_threshold: {:.0}%, noise floor: {:.2} ms_",
        diff.threshold * 100.0,
        diff.noise_floor_ms
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "| Metric | Baseline ms | Current ms | Δ% | Status |");
    let _ = writeln!(out, "| --- | ---: | ---: | ---: | --- |");
    for row in &diff.rows {
        let _ = writeln!(
            out,
            "| `{name}` | {b:.3} | {c:.3} | {d:+.1}% | {s} |",
            name = row.metric,
            b = row.baseline_ms,
            c = row.current_ms,
            d = row.delta_ratio * 100.0,
            s = row_status(row),
        );
    }
    // Same verdict ordering as `render_table` — broken first so a
    // PR comment can't say "All metrics within threshold." for a
    // structurally invalid input (codex round 3 MEDIUM).
    if diff.is_structurally_broken() {
        let _ = writeln!(
            out,
            "\n**BROKEN** — structurally invalid input ({} row(s); missing rollups: {:?}).",
            diff.rows.len(),
            diff.missing_rollups
        );
    } else if diff.any_regressed() {
        let _ = writeln!(
            out,
            "\n**REGRESSED** — {} metric(s) over the {:.0}% threshold.",
            diff.rows.iter().filter(|r| r.regressed).count(),
            diff.threshold * 100.0
        );
    } else {
        let _ = writeln!(out, "\nAll metrics within threshold.");
    }
    out
}

fn render_json(diff: &Diff, label: Option<&str>) -> serde_json::Value {
    let rows: Vec<_> = diff
        .rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "metric": r.metric,
                "baseline_ms": r.baseline_ms,
                "current_ms":  r.current_ms,
                "delta_ratio": r.delta_ratio,
                "below_noise_floor": r.below_noise_floor,
                "regressed": r.regressed,
            })
        })
        .collect();
    // Single authoritative `verdict` string so a JSON consumer that
    // only inspects one field can't accidentally treat corrupt input
    // as passing. The peer booleans (`regressed`, `structurally_broken`)
    // remain for backwards-compat / fine-grained inspection. Codex
    // round 4 NIT.
    let verdict = if diff.is_structurally_broken() {
        "broken"
    } else if diff.any_regressed() {
        "regressed"
    } else {
        "ok"
    };
    serde_json::json!({
        "label": label,
        "threshold": diff.threshold,
        "noise_floor_ms": diff.noise_floor_ms,
        "rows": rows,
        "verdict": verdict,
        "regressed": diff.any_regressed(),
        "structurally_broken": diff.is_structurally_broken(),
        "missing_rollups": diff.missing_rollups,
    })
}

fn row_status(row: &DiffRow) -> &'static str {
    if row.regressed {
        "REGRESSED"
    } else if row.below_noise_floor {
        "noise"
    } else if row.delta_ratio < -0.05 {
        "improved"
    } else {
        "ok"
    }
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
        let r1 = report_with(&[
            (StartupPhase::ReadFile, 1.0),
            (StartupPhase::ParseSchema, 5.0),
        ]);
        let r2 = report_with(&[
            (StartupPhase::ReadFile, 3.0),
            (StartupPhase::ParseSchema, 7.0),
        ]);
        let agg = aggregate(&[r1, r2]);
        let read = agg
            .phases
            .iter()
            .find(|p| p.phase == StartupPhase::ReadFile)
            .unwrap();
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

    // ──────────────────────────────────────────────────────────────
    // `jian perf compare` (D4) — diff + threshold gate
    // ──────────────────────────────────────────────────────────────

    fn perf_json(phases: &[(&str, f64)], wall_clock_median: f64) -> serde_json::Value {
        let phases: Vec<_> = phases
            .iter()
            .map(|(name, med)| {
                serde_json::json!({
                    "phase": name,
                    "on_critical": true,
                    "min_ms": *med,
                    "median_ms": *med,
                    "p95_ms": *med,
                })
            })
            .collect();
        serde_json::json!({
            "runs": 10,
            "phases": phases,
            "first_interactive_ms": { "median": wall_clock_median, "min": wall_clock_median, "p95": wall_clock_median },
            "critical_path_ms":     { "median": wall_clock_median, "min": wall_clock_median, "p95": wall_clock_median },
            "wall_clock_ms":        { "median": wall_clock_median, "min": wall_clock_median, "p95": wall_clock_median },
        })
    }

    #[test]
    fn build_diff_rows_per_phase_plus_three_rollups() {
        // Two phases + three roll-up rows = five total when both
        // sides agree on the phase set. Confirms the row order
        // matches the documented "phases first, then rollups".
        let base = perf_json(&[("ReadFile", 1.0), ("ParseSchema", 2.0)], 5.0);
        let cur = perf_json(&[("ReadFile", 1.0), ("ParseSchema", 2.0)], 5.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        assert_eq!(diff.rows.len(), 5);
        assert_eq!(diff.rows[0].metric, "ReadFile");
        assert_eq!(diff.rows[1].metric, "ParseSchema");
        assert_eq!(diff.rows[2].metric, "first_interactive (median)");
        assert_eq!(diff.rows[3].metric, "critical_path (median)");
        assert_eq!(diff.rows[4].metric, "wall_clock (median)");
        assert!(!diff.any_regressed());
    }

    #[test]
    fn regression_above_threshold_and_above_noise_floor_trips_gate() {
        // Baseline 10 ms → current 12 ms = +20% delta. With a 15%
        // threshold and 0.5 ms noise floor, this trips the gate.
        let base = perf_json(&[("ReadFile", 10.0)], 50.0);
        let cur = perf_json(&[("ReadFile", 12.0)], 50.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        assert!(diff.any_regressed(), "20% over 15% should regress");
        let read = diff.rows.iter().find(|r| r.metric == "ReadFile").unwrap();
        assert!((read.delta_ratio - 0.20).abs() < 1e-9);
        assert!(read.regressed);
    }

    #[test]
    fn regression_below_noise_floor_does_not_trip_gate() {
        // Baseline 0.04 ms → current 0.10 ms = +150% delta. The
        // delta ratio is huge but the baseline is below the 0.5 ms
        // noise floor — sub-millisecond medians are dominated by CI
        // jitter, gating on them produces false positives. Row stays
        // marked `below_noise_floor`, the gate stays open.
        let base = perf_json(&[("ReadFile", 0.04)], 50.0);
        let cur = perf_json(&[("ReadFile", 0.10)], 50.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        assert!(!diff.any_regressed(), "below noise floor cannot regress");
        let read = diff.rows.iter().find(|r| r.metric == "ReadFile").unwrap();
        assert!(read.below_noise_floor);
        assert!(!read.regressed);
    }

    #[test]
    fn improvement_below_negative_five_percent_marks_improved() {
        // Baseline 10 ms → current 8 ms = -20%. Above the noise
        // floor, well below -5% → status="improved".
        let base = perf_json(&[("ReadFile", 10.0)], 50.0);
        let cur = perf_json(&[("ReadFile", 8.0)], 50.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        let read = diff.rows.iter().find(|r| r.metric == "ReadFile").unwrap();
        assert_eq!(row_status(read), "improved");
        assert!(!diff.any_regressed());
    }

    #[test]
    fn phase_missing_from_baseline_is_skipped_not_failed() {
        // CI mismatch: baseline omits a phase the current run has.
        // The compare must skip the row rather than crash or false-
        // regress. The remaining phase still drives the verdict.
        let base = perf_json(&[("ReadFile", 1.0)], 5.0);
        let cur = perf_json(&[("ReadFile", 1.0), ("ParseSchema", 2.0)], 5.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        assert!(diff.rows.iter().all(|r| r.metric != "ParseSchema"));
        assert!(diff.rows.iter().any(|r| r.metric == "ReadFile"));
        assert!(!diff.any_regressed());
    }

    #[test]
    fn corrupt_json_with_no_rollups_is_structurally_broken() {
        // Codex round 2 HIGH: a fully corrupt current JSON (no
        // phases, no rollups) used to slip through with zero
        // comparable rows and exit success. Must now flag broken.
        let baseline = perf_json(&[("ReadFile", 10.0)], 50.0);
        let current = serde_json::json!({ "runs": 0, "phases": [] });
        let diff = build_diff(&baseline, &current, 0.15, 0.5);
        assert!(diff.is_structurally_broken());
        assert!(!diff.missing_rollups.is_empty());
    }

    #[test]
    fn missing_one_rollup_is_structurally_broken() {
        // A baseline that lacks `wall_clock_ms` (e.g. an older
        // schema someone forgot to migrate) must trip the broken
        // gate even if the other metrics line up cleanly.
        let baseline = serde_json::json!({
            "runs": 10,
            "phases": [{"phase": "ReadFile", "on_critical": true, "min_ms": 5.0, "median_ms": 10.0, "p95_ms": 15.0}],
            "first_interactive_ms": {"median": 50.0, "min": 40.0, "p95": 60.0},
            "critical_path_ms":     {"median": 50.0, "min": 40.0, "p95": 60.0},
            // wall_clock_ms intentionally absent
        });
        let current = perf_json(&[("ReadFile", 10.0)], 50.0);
        let diff = build_diff(&baseline, &current, 0.15, 0.5);
        assert!(diff.is_structurally_broken());
        assert_eq!(diff.missing_rollups, vec!["wall_clock_ms"]);
    }

    #[test]
    fn well_formed_input_is_not_structurally_broken() {
        // Sanity: the happy-path test pair must NOT flag broken.
        let base = perf_json(&[("ReadFile", 10.0)], 50.0);
        let cur = perf_json(&[("ReadFile", 11.0)], 50.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        assert!(!diff.is_structurally_broken());
        assert!(diff.missing_rollups.is_empty());
    }

    #[test]
    fn current_phase_with_missing_median_is_skipped_not_zeroed() {
        // Codex round 1 MEDIUM: a current phase whose `median_ms`
        // is missing or non-numeric used to be `unwrap_or(0.0)`'d,
        // which produced a fake "improved" delta. The fix treats
        // both sides symmetrically — missing -> skip the row.
        let baseline = serde_json::json!({
            "runs": 10,
            "phases": [
                {"phase": "ReadFile", "on_critical": true, "min_ms": 5.0, "median_ms": 10.0, "p95_ms": 15.0}
            ],
            "first_interactive_ms": {"median": 50.0, "min": 40.0, "p95": 60.0},
            "critical_path_ms":     {"median": 50.0, "min": 40.0, "p95": 60.0},
            "wall_clock_ms":        {"median": 50.0, "min": 40.0, "p95": 60.0},
        });
        let current = serde_json::json!({
            "runs": 10,
            "phases": [
                {"phase": "ReadFile", "on_critical": true, "min_ms": 5.0, "p95_ms": 15.0}
            ],
            "first_interactive_ms": {"median": 50.0, "min": 40.0, "p95": 60.0},
            "critical_path_ms":     {"median": 50.0, "min": 40.0, "p95": 60.0},
            "wall_clock_ms":        {"median": 50.0, "min": 40.0, "p95": 60.0},
        });
        let diff = build_diff(&baseline, &current, 0.15, 0.5);
        assert!(
            diff.rows.iter().all(|r| r.metric != "ReadFile"),
            "phase with missing current median_ms must be skipped"
        );
        assert!(
            !diff.any_regressed(),
            "missing-median phase must not silently improve and must not regress"
        );
    }

    #[test]
    fn zero_baseline_does_not_divide_by_zero() {
        // A 0.0 ms baseline (e.g. a no-op phase) must not produce
        // inf or NaN deltas. The row records 0.0 and stays in the
        // table; the noise floor handles whether to gate it.
        let base = perf_json(&[("ReadFile", 0.0)], 5.0);
        let cur = perf_json(&[("ReadFile", 5.0)], 5.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        let read = diff.rows.iter().find(|r| r.metric == "ReadFile").unwrap();
        assert_eq!(read.delta_ratio, 0.0);
        assert!(read.below_noise_floor);
        assert!(!read.regressed);
    }

    #[test]
    fn render_table_includes_metric_and_status_columns() {
        let base = perf_json(&[("ReadFile", 10.0)], 50.0);
        let cur = perf_json(&[("ReadFile", 12.0)], 50.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        let table = render_table(&diff, Some("linux-x86_64"));
        assert!(table.contains("linux-x86_64"));
        assert!(table.contains("ReadFile"));
        assert!(table.contains("REGRESSED"));
    }

    #[test]
    fn render_markdown_outputs_pipe_table() {
        let base = perf_json(&[("ReadFile", 10.0)], 50.0);
        let cur = perf_json(&[("ReadFile", 10.5)], 50.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        let md = render_markdown(&diff, Some("macos-aarch64"));
        assert!(md.contains("### startup-budget — macos-aarch64"));
        assert!(md.contains("| Metric |"));
        assert!(md.contains("`ReadFile`"));
        assert!(md.contains("All metrics within threshold."));
    }

    #[test]
    fn render_markdown_says_broken_on_corrupt_input() {
        // Codex round 3 MEDIUM: the PR comment used to claim "All
        // metrics within threshold." even for fully corrupt JSON
        // because the verdict line only checked `any_regressed()`.
        // Now the broken case takes precedence.
        let baseline = perf_json(&[("ReadFile", 10.0)], 50.0);
        let current = serde_json::json!({ "runs": 0, "phases": [] });
        let diff = build_diff(&baseline, &current, 0.15, 0.5);
        let md = render_markdown(&diff, Some("macos-aarch64"));
        assert!(
            md.contains("**BROKEN**"),
            "expected BROKEN verdict, got: {md}"
        );
        assert!(!md.contains("All metrics within threshold."));
    }

    #[test]
    fn render_table_says_broken_on_corrupt_input() {
        // Same defense for the human-readable table.
        let baseline = perf_json(&[("ReadFile", 10.0)], 50.0);
        let current = serde_json::json!({ "runs": 0, "phases": [] });
        let diff = build_diff(&baseline, &current, 0.15, 0.5);
        let table = render_table(&diff, Some("linux-x86_64"));
        assert!(
            table.contains("BROKEN"),
            "expected BROKEN verdict, got: {table}"
        );
        assert!(!table.contains("OK — all metrics"));
    }

    #[test]
    fn render_json_emits_rows_and_verdict() {
        let base = perf_json(&[("ReadFile", 10.0)], 50.0);
        let cur = perf_json(&[("ReadFile", 12.0)], 50.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        let v = render_json(&diff, Some("linux-x86_64"));
        assert_eq!(v["label"], "linux-x86_64");
        assert_eq!(v["threshold"], 0.15);
        assert_eq!(v["verdict"], "regressed");
        assert_eq!(v["regressed"], true);
        assert_eq!(v["structurally_broken"], false);
        let rows = v["rows"].as_array().unwrap();
        assert!(rows
            .iter()
            .any(|r| r["metric"] == "ReadFile" && r["regressed"] == true));
    }

    #[test]
    fn render_json_verdict_is_broken_on_corrupt_input() {
        // Codex round 4 NIT: a JSON consumer must not be able to
        // accidentally treat broken input as passing by inspecting
        // only the `regressed` boolean. The authoritative `verdict`
        // string folds both gating dimensions into one value.
        let baseline = perf_json(&[("ReadFile", 10.0)], 50.0);
        let current = serde_json::json!({ "runs": 0, "phases": [] });
        let diff = build_diff(&baseline, &current, 0.15, 0.5);
        let v = render_json(&diff, None);
        assert_eq!(v["verdict"], "broken");
        assert_eq!(v["structurally_broken"], true);
        // `regressed` stays false because the input is broken before
        // any threshold check could fire — the verdict string is the
        // authoritative gate signal.
        assert_eq!(v["regressed"], false);
    }

    #[test]
    fn render_json_verdict_is_ok_on_well_formed_input_within_threshold() {
        let base = perf_json(&[("ReadFile", 10.0)], 50.0);
        let cur = perf_json(&[("ReadFile", 10.5)], 50.0);
        let diff = build_diff(&base, &cur, 0.15, 0.5);
        let v = render_json(&diff, None);
        assert_eq!(v["verdict"], "ok");
        assert_eq!(v["regressed"], false);
        assert_eq!(v["structurally_broken"], false);
    }
}
