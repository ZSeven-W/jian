//! Cold-start budget regression guard (Plan 19 Task 9 / D3).
//!
//! Two layers of assertion live here:
//!
//! 1. **Framework overhead** (`startup_driver_overhead_*`,
//!    `startup_driver_per_phase_*`, `startup_driver_runs_every_*`):
//!    every phase registered as `futures::future::ready(Ok(()))`. The
//!    test catches scheduler regressions — a wakeup leak, an
//!    accidental `block_in_place`, a dropped phase — that would 10×
//!    the no-op driver's wall clock or break the per-phase invariants
//!    Plan 19 leans on downstream (font preload, splash dismissal,
//!    spatial fill-rest). The 200 ms ceiling stays generous on
//!    purpose to survive macOS aarch64 → Linux x86_64 → Windows VM
//!    variance.
//!
//! 2. **Real DataPath bootstrap** (`startup_budget_counter_doc_*`,
//!    `startup_budget_500_node_doc_*`): drives
//!    `HostAgnosticBootstrap::install_data_path` with no-op Visual /
//!    Background phases — the same shape `jian perf startup` uses,
//!    minus the visual presentation. This measures the actual cold-
//!    start cost the user pays before the first frame and gates the
//!    Plan 19 §Gate desktop budget (< 400 ms total). The ceiling is
//!    set per fixture: counter.op (3 nodes, ~200 LOC of schema) gets
//!    a 100 ms ceiling — measured p95 on macOS aarch64 sits near
//!    1 ms, the headroom covers Linux/Windows CI noise. The 500-node
//!    fixture gets 200 ms — `Runtime::new_from_document` and
//!    `build_layout` both walk the tree once. The total Plan 19
//!    desktop budget (400 ms) is split conceptually as DataPath ≤
//!    200 ms, Visual ≤ 150 ms, framework / process-launch overhead ≤
//!    50 ms; these tests guard the first slice. AOT (`.op.pack`) and
//!    full-process measurements are deferred to D1 / D4 respectively.

use jian_core::startup::{
    BootstrapSource, HostAgnosticBootstrap, PhaseResult, StartupConfig, StartupDriver,
    StartupPhase, StartupStage,
};
use std::time::Instant;

/// Generous overhead ceiling for the no-op driver, picked so the
/// test stays green across the GitHub Actions matrix (linux x86_64,
/// linux aarch64, macos aarch64, windows x86_64). Tighten when Plan
/// 19 Tasks 2-7 turn the phase impls into real work.
const FRAMEWORK_CEILING_MS: f64 = 200.0;

fn run_driver_once() -> f64 {
    let mut driver = StartupDriver::new();
    for phase in StartupPhase::ALL {
        driver.register(*phase, || async { PhaseResult::Ok(()) });
    }
    let started = Instant::now();
    let report = futures::executor::block_on(driver.run(StartupConfig::default()))
        .expect("driver.run(no-op) returns Ok");
    let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
    // Sanity: the driver's own report should match wall-clock to
    // within a few millis.
    let report_total = report.total_wall_clock_ms();
    assert!(
        (elapsed - report_total).abs() < 50.0,
        "report.total_wall_clock_ms ({:.2}) drifted from wall-clock ({:.2})",
        report_total,
        elapsed,
    );
    elapsed
}

#[test]
fn startup_driver_overhead_under_framework_ceiling() {
    // First run is warmup (allocator / branch predictor / cache);
    // measurement comes from a steady-state second pass. This pattern
    // matches `jian perf startup`'s aggregator which discards no
    // samples but uses median / p95 — a single hot-path run is
    // representative for a regression guard.
    let _ = run_driver_once();
    let elapsed = run_driver_once();
    assert!(
        elapsed < FRAMEWORK_CEILING_MS,
        "startup framework overhead exceeded {:.0} ms ceiling: {:.2} ms",
        FRAMEWORK_CEILING_MS,
        elapsed,
    );
}

#[test]
fn startup_driver_per_phase_end_time_within_total() {
    // The actual API contract: `critical_path_ms` is a *serial sum*
    // of `on_critical` phase durations (not a longest-path metric),
    // so it's not bounded by `total_wall_clock_ms` when multiple
    // critical phases run in parallel. The genuine invariant the
    // scheduler must maintain is that **every individual phase
    // finished no later than the wall-clock total**. A regression
    // that drifts a phase's `ended_at_ms` past the rolled-up total
    // (e.g. a wakeup-after-shutdown bug) trips this.
    let mut driver = StartupDriver::new();
    for phase in StartupPhase::ALL {
        driver.register(*phase, || async { PhaseResult::Ok(()) });
    }
    let report = futures::executor::block_on(driver.run(StartupConfig::default()))
        .expect("driver.run returns Ok");
    let total = report.total_wall_clock_ms();
    for timing in &report.phases {
        assert!(
            timing.ended_at_ms() <= total + 0.001,
            "phase {:?} ended_at_ms {:.4} > total wall-clock {:.4}",
            timing.phase,
            timing.ended_at_ms(),
            total,
        );
    }
}

#[test]
fn startup_driver_runs_every_phase_exactly_once() {
    // Foundational invariant — each declared StartupPhase must fire
    // exactly once per driver run. A regression in the scheduler
    // that drops a phase (or fires it twice) trips this. Plan 19's
    // overall correctness leans on this assumption everywhere
    // downstream (font preload, spatial index, splash dismissal).
    let mut driver = StartupDriver::new();
    for phase in StartupPhase::ALL {
        driver.register(*phase, || async { PhaseResult::Ok(()) });
    }
    let report = futures::executor::block_on(driver.run(StartupConfig::default()))
        .expect("driver.run returns Ok");
    for phase in StartupPhase::ALL {
        let timings: Vec<_> = report.phases.iter().filter(|t| t.phase == *phase).collect();
        assert_eq!(
            timings.len(),
            1,
            "phase {:?} fired {} times; expected exactly 1",
            phase,
            timings.len(),
        );
    }
}

// ──────────────────────────────────────────────────────────────────
// Real DataPath bootstrap budgets (Plan 19 §Gate, D3)
// ──────────────────────────────────────────────────────────────────

/// Per-fixture ceiling for the counter-shaped doc (3 nodes). Local
/// p95 on macOS aarch64 is sub-millisecond; the 100 ms ceiling buys
/// ~100× headroom for slower CI runners and warm-cache variance.
const COUNTER_BUDGET_MS: f64 = 100.0;

/// Per-fixture ceiling for a 500-node doc — `Runtime::new_from_document`
/// and `build_layout` both walk the tree once, and visible-spatial
/// fills the off-viewport set in the same pass. 200 ms is half the
/// Plan 19 desktop total (400 ms), leaving 200 ms for the visual
/// stage and process-launch overhead D4 will gate end-to-end.
const LARGE_DOC_BUDGET_MS: f64 = 200.0;

/// Drive a single bootstrap iteration: `HostAgnosticBootstrap` for
/// the DataPath stage, no-op closures for Visual + Background so the
/// driver still completes its full phase set (matches `jian perf
/// startup`'s shape — the desktop host attaches real Visual impls in
/// a second `run_stage` call after the window opens). Returns the
/// elapsed wall clock in milliseconds.
fn run_bootstrap_once(source: BootstrapSource, viewport: (f32, f32)) -> f64 {
    let mut driver = StartupDriver::new();
    let _handles = HostAgnosticBootstrap::install_data_path(&mut driver, source, viewport);
    for phase in StartupPhase::ALL {
        if phase.stage() != StartupStage::DataPath {
            driver.register(*phase, || async { PhaseResult::Ok(()) });
        }
    }
    let started = Instant::now();
    let _report = futures::executor::block_on(driver.run(StartupConfig::default()))
        .expect("driver.run(bootstrap) returns Ok");
    started.elapsed().as_secs_f64() * 1_000.0
}

/// 500-node fixture — a single root frame with 500 sibling rectangles.
/// Stress-tests the *quantity* of work `Runtime::new_from_document`,
/// `build_layout`, and `BuildVisibleSpatial` do, not their depth.
/// Codex review note: a deep tree would mostly stress recursion limits
/// — a flat fan-out is the realistic large-doc shape (gallery /
/// dashboard layouts).
fn synth_500_node_doc() -> String {
    let mut children = String::with_capacity(500 * 80);
    for i in 0..500 {
        if i > 0 {
            children.push(',');
        }
        // `r##` lets the JSON's `"#0066ff"` literal pass through
        // without colliding with the raw-string terminator.
        children.push_str(&format!(
            r##"{{"type":"rectangle","id":"r{i}","width":40,"height":40,"fill":[{{"type":"solid","color":"#0066ff"}}]}}"##
        ));
    }
    format!(
        r##"{{
  "formatVersion": "1.0",
  "version": "1.0.0",
  "id": "large-doc",
  "app": {{ "name": "Large", "version": "1.0.0", "id": "com.example.large" }},
  "children": [{{
    "type": "frame",
    "id": "root",
    "width": 2400,
    "height": 2400,
    "layout": "horizontal",
    "children": [{children}]
  }}]
}}"##
    )
}

#[test]
fn startup_budget_counter_doc_via_bootstrap() {
    // counter.op (3 nodes) — the canonical small-doc fixture every
    // other Plan 19 test uses. Two runs: warmup (allocator / cache)
    // + measurement (steady-state). Mirrors the framework-overhead
    // pattern above so the same flake characteristics apply.
    let src = include_str!("../../jian-core/tests/counter.op").to_owned();
    let viewport = (400.0, 200.0);
    let _warmup = run_bootstrap_once(BootstrapSource::String(src.clone()), viewport);
    let elapsed = run_bootstrap_once(BootstrapSource::String(src), viewport);
    assert!(
        elapsed < COUNTER_BUDGET_MS,
        "counter.op DataPath bootstrap exceeded {:.0} ms ceiling: {:.2} ms",
        COUNTER_BUDGET_MS,
        elapsed,
    );
}

#[test]
fn startup_budget_500_node_doc_via_bootstrap() {
    // 500-node synthetic large-doc. The body is built once outside
    // the timed region — schema construction is not part of the
    // startup pipeline (the user pays for *.op* parsing inside
    // ParseSchema, and BootstrapSource::String already feeds the
    // pipeline raw text). Warmup pass shakes out cold-allocator
    // jitter the same as the small-doc test.
    let src = synth_500_node_doc();
    let viewport = (2400.0, 2400.0);
    let _warmup = run_bootstrap_once(BootstrapSource::String(src.clone()), viewport);
    let elapsed = run_bootstrap_once(BootstrapSource::String(src), viewport);
    assert!(
        elapsed < LARGE_DOC_BUDGET_MS,
        "500-node DataPath bootstrap exceeded {:.0} ms ceiling: {:.2} ms",
        LARGE_DOC_BUDGET_MS,
        elapsed,
    );
}

#[test]
fn startup_budget_bootstrap_completes_every_datapath_phase() {
    // Sanity guard for the budget tests above: the bootstrap must
    // actually execute every DataPath phase — a regression that
    // silently skips one (e.g. a dep-graph mis-wire) would make the
    // wall clock falsely look fast. Runs once and asserts the
    // DataPath-stage phase set fully populates the report.
    let src = include_str!("../../jian-core/tests/counter.op").to_owned();
    let mut driver = StartupDriver::new();
    let _handles = HostAgnosticBootstrap::install_data_path(
        &mut driver,
        BootstrapSource::String(src),
        (400.0, 200.0),
    );
    for phase in StartupPhase::ALL {
        if phase.stage() != StartupStage::DataPath {
            driver.register(*phase, || async { PhaseResult::Ok(()) });
        }
    }
    let report = futures::executor::block_on(driver.run(StartupConfig::default()))
        .expect("driver.run(bootstrap) returns Ok");
    for phase in StartupPhase::ALL
        .iter()
        .filter(|p| p.stage() == StartupStage::DataPath)
    {
        assert!(
            report.phases.iter().any(|t| t.phase == *phase),
            "DataPath phase {phase:?} missing from bootstrap report"
        );
    }
}
