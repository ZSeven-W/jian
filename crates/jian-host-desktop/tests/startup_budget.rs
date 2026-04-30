//! Cold-start budget regression guard (Plan 19 Task 9).
//!
//! What this test measures **today**: the `StartupDriver` framework's
//! own overhead — the cost of building a layered `FuturesUnordered`
//! dispatch over every variant in `StartupPhase::ALL`, the per-phase
//! `PhaseTiming` allocation, and the `Instant::now()` deltas the
//! report rolls up. Phase impls are currently
//! `futures::future::ready(Ok(()))` no-ops; Plan 19 Tasks 2-7 land
//! the real Runtime-coupled implementations (eager GPU init, lazy
//! expression compile, font subset, visible-only spatial, …) over
//! several follow-ups.
//!
//! What this test catches: a regression in the scheduler that
//! suddenly makes the framework take 10× longer (e.g. a wakeup leak
//! or an accidental `block_in_place` inside a phase). The ceiling
//! is intentionally generous — the no-op driver should run in well
//! under 50 ms on every CI machine; the assertion uses 200 ms to
//! survive macOS aarch64 → Linux x86_64 → Windows VM variance.
//!
//! Once the real phase impls land, this test's ceiling tightens
//! toward the C19 desktop budget (400 ms). Until then a tighter
//! ceiling here would falsely fail when the framework is the entire
//! thing being measured.

use jian_core::startup::{PhaseResult, StartupConfig, StartupDriver, StartupPhase};
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
