//! `StartupDriver` — runs registered async phase implementations respecting
//! the dependency graph from [`StartupPhase::deps`].
//!
//! ### Design
//!
//! - Phases are registered as `FnOnce() -> Future<Output = Result<(), String>>`
//!   closures. Task 1 keeps the signature decoupled from `Runtime`; later
//!   tasks (Plan 19 §2-§7) wrap a Runtime-aware facade that captures `&mut`
//!   handles via channels / Arc / interior mutability.
//! - The driver schedules **per-completion**, not per-layer: it dispatches
//!   every initially-ready phase into a [`FuturesUnordered`] pool, then on
//!   each completion marks the phase done and **immediately** dispatches any
//!   newly-ready phase. A late, non-critical sibling never blocks a
//!   downstream critical-path phase whose own dependencies are already
//!   satisfied. (Codex review round 1, HIGH.)
//! - Per-phase timings are stamped as: `started_at_ms` is the offset from
//!   `run()` entry just before the future is created; `duration_ms` is the
//!   wall-clock time that future took to resolve.
//!
//! ### Cancellation
//!
//! When a phase returns `Err`, the driver **stops dispatching new phases**
//! but still drains the pool — every in-flight phase runs to its natural
//! completion. This avoids the cancellation-by-drop footgun that
//! `try_join_all` introduces (a sibling holding a oneshot sender or a GPU
//! context would be dropped mid-init). The first error encountered is the
//! one returned; subsequent failures are discarded.
//!
//! ### What this module does NOT do (yet)
//!
//! - It does not pin GPU init to a dedicated OS thread. Plan 19 Task 2 wires
//!   that up — the host crate spawns the GPU init *before* calling `run()`
//!   and registers a phase that just `await`s the resulting oneshot.
//! - It does not yet enforce a critical-path priority scheduler. Per-completion
//!   advance is fair and correct; Task 12 introduces a `SpawnHandle`
//!   abstraction to let hosts plug in tokio / async-std / single-threaded
//!   executors with their own priority semantics.

use crate::startup::phase::StartupPhase;
use crate::startup::report::{PhaseTiming, StartupReport};
use futures::future::{FutureExt, LocalBoxFuture};
use futures::stream::{FuturesUnordered, StreamExt};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// Configuration passed to [`StartupDriver::run`]. Empty for Task 1; reserved
/// for viewport / budget hints / spawn policy in later Plan 19 tasks.
#[derive(Debug, Default, Clone)]
pub struct StartupConfig {}

/// Driver-level error: a phase reported failure, or the dependency graph
/// could not make forward progress.
#[derive(Debug)]
pub enum StartupError {
    /// A phase whose deps were satisfied had no implementation registered.
    Unregistered(StartupPhase),
    /// One or more phases remain pending after the in-flight pool drained
    /// without errors. Defensive guard: the static graph is acyclic, so
    /// reaching this branch means a phase whose deps are all `done` was
    /// never dispatched (driver bug) or an error short-circuited dispatch.
    NoProgress { remaining: Vec<StartupPhase> },
    /// A registered phase returned `Err`. The string is supplied by the
    /// phase impl; the driver does not interpret it.
    PhaseFailed {
        phase: StartupPhase,
        message: String,
    },
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartupError::Unregistered(p) => {
                write!(f, "startup phase {:?} has no registered implementation", p)
            }
            StartupError::NoProgress { remaining } => {
                write!(
                    f,
                    "startup driver could not make progress; remaining phases: {:?}",
                    remaining
                )
            }
            StartupError::PhaseFailed { phase, message } => {
                write!(f, "startup phase {:?} failed: {}", phase, message)
            }
        }
    }
}

impl std::error::Error for StartupError {}

/// Outcome a registered phase yields back to the driver.
pub type PhaseResult = Result<(), String>;

type PhaseFuture = LocalBoxFuture<'static, PhaseResult>;
type PhaseFn = Box<dyn FnOnce() -> PhaseFuture + 'static>;

/// Runs the cold-start phase graph.
///
/// Construct with [`StartupDriver::new`], register an implementation for
/// every phase via [`StartupDriver::register`], then drive to completion
/// with [`StartupDriver::run`].
pub struct StartupDriver {
    impls: HashMap<StartupPhase, PhaseFn>,
}

impl Default for StartupDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl StartupDriver {
    pub fn new() -> Self {
        Self {
            impls: HashMap::with_capacity(StartupPhase::ALL.len()),
        }
    }

    /// Register an implementation for `phase`. Re-registering replaces the
    /// previous closure (last-writer-wins) — useful when host crates layer
    /// platform-specific overrides on top of a default config.
    ///
    /// The future is **not** required to be `Send`: phase impls commonly
    /// hold non-`Send` runtime handles (`Rc`, `RefCell`, GPU context bound
    /// to the calling thread) across `.await` points. A pluggable spawn
    /// policy that demands `Send` will be added by Plan 19 Task 12.
    pub fn register<F, Fut>(&mut self, phase: StartupPhase, f: F)
    where
        F: FnOnce() -> Fut + 'static,
        Fut: std::future::Future<Output = PhaseResult> + 'static,
    {
        self.impls
            .insert(phase, Box::new(move || f().boxed_local()));
    }

    /// Returns true once every phase in [`StartupPhase::ALL`] has a
    /// registered implementation.
    pub fn is_fully_registered(&self) -> bool {
        StartupPhase::ALL
            .iter()
            .all(|p| self.impls.contains_key(p))
    }

    /// Drive the pipeline. Per-completion topological execution; siblings
    /// run concurrently and a slow sibling never blocks a downstream phase
    /// whose own deps are already satisfied.
    pub async fn run(mut self, _config: StartupConfig) -> Result<StartupReport, StartupError> {
        let t0 = Instant::now();
        let mut report = StartupReport::default();
        let mut pending: HashSet<StartupPhase> = StartupPhase::ALL.iter().copied().collect();
        let mut done: HashSet<StartupPhase> = HashSet::with_capacity(StartupPhase::ALL.len());
        let mut in_flight: FuturesUnordered<LocalBoxFuture<'static, Result<PhaseTiming, StartupError>>> =
            FuturesUnordered::new();
        let mut first_error: Option<StartupError> = None;

        // Initial dispatch: every phase whose deps are already satisfied.
        // If `dispatch_phase` errors here we can NOT short-circuit with `?`
        // — that would drop `in_flight` while it still holds successfully-
        // dispatched root futures, recreating the cancellation-by-drop
        // footgun we just removed from the main loop. Stash the error and
        // fall through to the drain loop instead.
        for phase in ready_phases(&pending, &done) {
            match dispatch_phase(&mut self.impls, phase, t0, &mut in_flight) {
                Ok(()) => {
                    pending.remove(&phase);
                }
                Err(e) => {
                    first_error = Some(e);
                    break;
                }
            }
        }

        while let Some(result) = in_flight.next().await {
            match result {
                Ok(timing) => {
                    let phase = timing.phase;
                    done.insert(phase);
                    report.phases.push(timing);

                    // Only dispatch follow-ups while the run is healthy. After an
                    // error we drain the pool to natural completion without
                    // starting anything new — see the Cancellation section in
                    // the module docs.
                    if first_error.is_none() {
                        for phase in ready_phases(&pending, &done) {
                            if let Err(e) =
                                dispatch_phase(&mut self.impls, phase, t0, &mut in_flight)
                            {
                                first_error = Some(e);
                                break;
                            }
                            pending.remove(&phase);
                        }
                    }
                }
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }

        if let Some(e) = first_error {
            return Err(e);
        }

        if !pending.is_empty() {
            let mut remaining: Vec<_> = pending.into_iter().collect();
            remaining.sort_by_key(|p| {
                StartupPhase::ALL
                    .iter()
                    .position(|q| q == p)
                    .unwrap_or(usize::MAX)
            });
            return Err(StartupError::NoProgress { remaining });
        }

        if let Some(t) = report
            .phases
            .iter()
            .find(|t| t.phase == StartupPhase::EventPumpReady)
        {
            report.first_interactive_ms = t.ended_at_ms();
        }

        Ok(report)
    }
}

/// Phases in `pending` whose deps are all in `done`, returned in
/// `StartupPhase::ALL` declaration order so test assertions stay stable.
fn ready_phases(
    pending: &HashSet<StartupPhase>,
    done: &HashSet<StartupPhase>,
) -> Vec<StartupPhase> {
    let mut out: Vec<StartupPhase> = pending
        .iter()
        .copied()
        .filter(|p| p.deps().iter().all(|d| done.contains(d)))
        .collect();
    out.sort_by_key(|p| {
        StartupPhase::ALL
            .iter()
            .position(|q| q == p)
            .unwrap_or(usize::MAX)
    });
    out
}

/// Pull `phase`'s impl out of `impls`, stamp `started_at_ms`, and push the
/// timed wrapper future into `in_flight`. Returns `Unregistered` if the
/// impl was missing.
fn dispatch_phase(
    impls: &mut HashMap<StartupPhase, PhaseFn>,
    phase: StartupPhase,
    t0: Instant,
    in_flight: &mut FuturesUnordered<LocalBoxFuture<'static, Result<PhaseTiming, StartupError>>>,
) -> Result<(), StartupError> {
    let f = impls
        .remove(&phase)
        .ok_or(StartupError::Unregistered(phase))?;
    let started_at_ms = elapsed_ms(t0);
    let fut = f();
    let wrapped = async move {
        let phase_t0 = Instant::now();
        match fut.await {
            Ok(()) => Ok(PhaseTiming {
                phase,
                started_at_ms,
                duration_ms: elapsed_ms(phase_t0),
                on_critical: phase.is_critical(),
                notes: None,
            }),
            Err(message) => Err(StartupError::PhaseFailed { phase, message }),
        }
    }
    .boxed_local();
    in_flight.push(wrapped);
    Ok(())
}

fn elapsed_ms(t0: Instant) -> f64 {
    t0.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use futures::future;
    use std::cell::{Cell, RefCell};
    use std::pin::Pin;
    use std::rc::Rc;
    use std::task::{Context, Poll};

    fn ok_phase() -> impl FnOnce() -> future::Ready<PhaseResult> + 'static {
        || future::ready(Ok(()))
    }

    fn register_all_noop(driver: &mut StartupDriver) {
        for p in StartupPhase::ALL {
            driver.register(*p, ok_phase());
        }
    }

    /// Future that returns `Pending` `count` times before resolving. Used to
    /// simulate a slow phase under `block_on` without any timer dependency.
    struct YieldN {
        remaining: usize,
    }
    impl std::future::Future for YieldN {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.remaining == 0 {
                Poll::Ready(())
            } else {
                self.remaining -= 1;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
    fn yield_n(n: usize) -> YieldN {
        YieldN { remaining: n }
    }

    #[test]
    fn fully_registered_after_registering_all_phases() {
        let mut d = StartupDriver::new();
        assert!(!d.is_fully_registered());
        register_all_noop(&mut d);
        assert!(d.is_fully_registered());
    }

    #[test]
    fn run_completes_with_all_phases_recorded() {
        let mut d = StartupDriver::new();
        register_all_noop(&mut d);
        let report = block_on(d.run(StartupConfig::default())).expect("run ok");
        assert_eq!(report.phases.len(), StartupPhase::ALL.len());
        assert!(report.first_interactive_ms >= 0.0);
        let mut names: Vec<_> = report.phases.iter().map(|t| t.phase).collect();
        names.sort_by_key(|p| {
            StartupPhase::ALL.iter().position(|q| q == p).unwrap()
        });
        let mut expected: Vec<_> = StartupPhase::ALL.to_vec();
        expected.sort_by_key(|p| {
            StartupPhase::ALL.iter().position(|q| q == p).unwrap()
        });
        assert_eq!(names, expected);
    }

    #[test]
    fn run_observes_topological_order() {
        let log: Rc<RefCell<Vec<StartupPhase>>> = Rc::new(RefCell::new(Vec::new()));
        let mut d = StartupDriver::new();
        for p in StartupPhase::ALL {
            let log = Rc::clone(&log);
            let phase = *p;
            d.register(*p, move || {
                log.borrow_mut().push(phase);
                future::ready(Ok(()))
            });
        }
        block_on(d.run(StartupConfig::default())).expect("run ok");
        let log = log.borrow();
        for (i, p) in log.iter().enumerate() {
            for d in p.deps() {
                let dep_idx = log.iter().position(|q| q == d).expect("dep ran");
                assert!(
                    dep_idx < i,
                    "{p:?} ran at {i} before its dep {d:?} at {dep_idx}"
                );
            }
        }
    }

    #[test]
    fn unregistered_phase_returns_error() {
        let mut d = StartupDriver::new();
        for p in StartupPhase::ALL {
            if *p != StartupPhase::ReadFile {
                d.register(*p, ok_phase());
            }
        }
        let err = block_on(d.run(StartupConfig::default())).unwrap_err();
        match err {
            StartupError::Unregistered(StartupPhase::ReadFile) => {}
            other => panic!("expected Unregistered(ReadFile), got {other:?}"),
        }
    }

    #[test]
    fn phase_failure_propagates() {
        let mut d = StartupDriver::new();
        for p in StartupPhase::ALL {
            if *p == StartupPhase::ParseSchema {
                d.register(*p, || {
                    future::ready(Err::<(), _>("bad schema".to_string()))
                });
            } else {
                d.register(*p, ok_phase());
            }
        }
        let err = block_on(d.run(StartupConfig::default())).unwrap_err();
        match err {
            StartupError::PhaseFailed { phase, message } => {
                assert_eq!(phase, StartupPhase::ParseSchema);
                assert_eq!(message, "bad schema");
            }
            other => panic!("expected PhaseFailed, got {other:?}"),
        }
    }

    #[test]
    fn timings_are_monotonically_non_decreasing_within_critical_path() {
        let mut d = StartupDriver::new();
        register_all_noop(&mut d);
        let report = block_on(d.run(StartupConfig::default())).expect("run ok");
        let by_phase: std::collections::HashMap<_, _> = report
            .phases
            .iter()
            .map(|t| (t.phase, t.clone()))
            .collect();
        for t in &report.phases {
            for dep in t.phase.deps() {
                let dt = by_phase.get(dep).expect("dep timing recorded");
                assert!(
                    t.started_at_ms + 1e-6 >= dt.ended_at_ms(),
                    "{phase:?} started at {start} before dep {dep:?} ended at {dep_end}",
                    phase = t.phase,
                    start = t.started_at_ms,
                    dep = dep,
                    dep_end = dt.ended_at_ms()
                );
            }
        }
    }

    /// Codex review round 1 HIGH: per-completion scheduling means a slow
    /// non-critical (or critical-but-independent) sibling must not block a
    /// downstream phase whose own deps are already satisfied.
    ///
    /// `InitGpuContext` and `ReadFile` are both in the initial layer (no
    /// deps). `ParseSchema` depends only on `ReadFile`. With a layer
    /// barrier, `ParseSchema` would have to wait for `InitGpuContext`. Per-
    /// completion, it shouldn't.
    #[test]
    fn fast_critical_path_does_not_wait_for_slow_independent_sibling() {
        let order: Rc<RefCell<Vec<StartupPhase>>> = Rc::new(RefCell::new(Vec::new()));
        let mut d = StartupDriver::new();
        for p in StartupPhase::ALL {
            let order = Rc::clone(&order);
            let phase = *p;
            if *p == StartupPhase::InitGpuContext {
                d.register(*p, move || async move {
                    yield_n(50).await;
                    order.borrow_mut().push(phase);
                    Ok(())
                });
            } else {
                d.register(*p, move || {
                    let order = Rc::clone(&order);
                    async move {
                        order.borrow_mut().push(phase);
                        Ok(())
                    }
                });
            }
        }
        block_on(d.run(StartupConfig::default())).expect("run ok");
        let order = order.borrow();
        let parse_idx = order
            .iter()
            .position(|p| *p == StartupPhase::ParseSchema)
            .expect("ParseSchema ran");
        let init_idx = order
            .iter()
            .position(|p| *p == StartupPhase::InitGpuContext)
            .expect("InitGpuContext ran");
        assert!(
            parse_idx < init_idx,
            "ParseSchema (idx {parse_idx}) must finish before slow InitGpuContext (idx {init_idx}) — \
             order = {:?}",
            *order
        );
    }

    /// Codex review round 1 MEDIUM: when a phase fails, in-flight siblings
    /// must run to natural completion (no `try_join_all`-style cancellation).
    #[test]
    fn sibling_phases_complete_before_error_returns() {
        // Both ReadFile and InitGpuContext are root phases and run concurrently.
        // Make ReadFile fail; InitGpuContext must still complete its body.
        let init_completed: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let mut d = StartupDriver::new();
        for p in StartupPhase::ALL {
            if *p == StartupPhase::ReadFile {
                d.register(*p, || {
                    future::ready(Err::<(), _>("disk gone".to_string()))
                });
            } else if *p == StartupPhase::InitGpuContext {
                let flag = Rc::clone(&init_completed);
                d.register(*p, move || async move {
                    yield_n(20).await;
                    flag.set(true);
                    Ok(())
                });
            } else {
                d.register(*p, ok_phase());
            }
        }
        let err = block_on(d.run(StartupConfig::default())).unwrap_err();
        assert!(
            matches!(err, StartupError::PhaseFailed { phase: StartupPhase::ReadFile, .. }),
            "expected ReadFile failure, got {err:?}"
        );
        assert!(
            init_completed.get(),
            "InitGpuContext must have completed naturally despite ReadFile error"
        );
    }

    /// Codex review round 2 MEDIUM: the initial-dispatch path also has to
    /// drain in-flight futures on error. Specifically: if a phase is
    /// successfully dispatched into the pool and a *later* root phase is
    /// missing its impl, returning `Err` immediately would drop the live
    /// future from the first phase. The driver must fall through to the
    /// drain loop instead.
    ///
    /// Setup: `ReadFile` is registered with a yield_n delay that flips a
    /// flag at completion; `InitGpuContext` (the next root in `ALL`) has
    /// no impl. We assert the flag is set, proving `ReadFile` ran to
    /// natural completion before the `Unregistered` error returned.
    #[test]
    fn initial_dispatch_drains_in_flight_on_unregistered() {
        let read_completed: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let mut d = StartupDriver::new();
        for p in StartupPhase::ALL {
            if *p == StartupPhase::ReadFile {
                let flag = Rc::clone(&read_completed);
                d.register(*p, move || async move {
                    yield_n(20).await;
                    flag.set(true);
                    Ok(())
                });
            } else if *p == StartupPhase::InitGpuContext {
                // Intentionally NOT registered.
                continue;
            } else {
                d.register(*p, ok_phase());
            }
        }
        let err = block_on(d.run(StartupConfig::default())).unwrap_err();
        assert!(
            matches!(err, StartupError::Unregistered(StartupPhase::InitGpuContext)),
            "expected Unregistered(InitGpuContext), got {err:?}"
        );
        assert!(
            read_completed.get(),
            "ReadFile must have run to natural completion before initial-dispatch error returned"
        );
    }

    /// Codex review round 1 MEDIUM: register accepts !Send futures (an `Rc`
    /// captured across an `.await` makes the resulting future `!Send`). If
    /// the bound were still `+ Send`, this test would fail to compile.
    #[test]
    fn register_accepts_non_send_futures() {
        let local: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let mut d = StartupDriver::new();
        for p in StartupPhase::ALL {
            let local = Rc::clone(&local);
            let phase = *p;
            d.register(*p, move || async move {
                // Hold `local` (Rc, !Send) across an await so the resulting
                // future itself is !Send.
                yield_n(1).await;
                if phase == StartupPhase::EventPumpReady {
                    local.set(true);
                }
                Ok(())
            });
        }
        block_on(d.run(StartupConfig::default())).expect("run ok");
        assert!(local.get(), "EventPumpReady should have flipped the flag");
    }
}
