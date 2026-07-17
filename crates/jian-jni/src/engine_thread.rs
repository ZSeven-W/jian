//! The ONE dedicated engine thread (spec §6.7): every engine call dispatches
//! onto it; lifecycle calls block on a post-and-wait barrier; teardown drains
//! the queue on the engine thread, completes every parked waiter with
//! [`Dispatch::Closing`], runs the final job (destroy → window release →
//! global-ref deletion) strictly before the thread quits, and joins from a
//! reaper thread when close() originates on the engine thread itself.
//!
//! Panic policy: EVERY closure handed to this queue — engine jobs, drained
//! cleanups, and the final teardown job — runs under `catch_unwind`. A
//! panic in one closure can therefore never unwind the engine thread, skip
//! the rest of the drain, or drop the destroy. A panic in a blocking
//! `call()` job is captured and re-raised on the CALLING thread (matching
//! `JoinHandle::join`), so the caller is never left parked. The engine
//! thread only ever exits through the normal Close/deferred-final path.
//!
//! Pure `std` — host-testable; the JNI/NDK edges live in the Android-only
//! modules and only hand this queue closures.

use std::any::Any;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle, ThreadId};

/// Shell-side dispatch-rejected status (never produced by the C ABI).
pub const STATUS_CLOSING: i32 = -1;

/// Best-effort stderr log that cannot panic on a WRITE failure (unlike
/// `eprintln!`, which does). Every use here is on a teardown path where an
/// unwinding panic would drop a live payload unguarded or lose the destroy,
/// so the `write_fmt` `Err` is ignored instead of panicked on. The format
/// arguments are only literals, `&str`, and an OS-backed `io::Error`, whose
/// `Display` impls do not panic.
macro_rules! log_teardown {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

/// Result of dispatching onto the engine thread.
#[derive(Debug, PartialEq, Eq)]
pub enum Dispatch<R> {
    /// The job ran; its owned result.
    Done(R),
    /// The queue is closing (or closed): the job did NOT run.
    Closing,
}

impl<R> Dispatch<R> {
    pub fn is_closing(&self) -> bool {
        matches!(self, Dispatch::Closing)
    }
}

/// Runs a teardown-side closure so a panic can never skip the work that
/// follows it. On the drain and final-job paths there is no caller to
/// receive the panic, and losing one buggy cleanup's panic is strictly
/// better than losing the destroy.
///
/// The caught payload is dropped through [`drop_guarded`]: a `panic_any`
/// payload can carry a value whose own `Drop` panics — arbitrarily deep —
/// and any such drop happening outside a guard would unwind the engine
/// thread, the exact failure this function exists to prevent.
fn run_guarded(what: &str, f: impl FnOnce()) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(f)) {
        log_teardown!("jian-jni: {what} panicked; continuing teardown");
        drop_guarded(payload);
    }
}

/// Drops a caught panic payload without any possibility of unwinding. The
/// common case drops it normally (no leak); ONLY if the payload's own `Drop`
/// panics is the resulting payload FORGOTTEN rather than dropped — its
/// `Drop` never runs, so an arbitrarily deep panicking-`Drop` chain cannot
/// re-enter and unwind. `mem::forget` runs no destructor, so this is the
/// absorbing terminal state and cannot recurse. The leak is bounded to that
/// pathological case (a caught payload whose destructor panics); the
/// forgotten value may own arbitrary resources, but never losing the destroy
/// is worth that trade on a teardown path that should never be hit.
pub(crate) fn drop_guarded(payload: Box<dyn Any + Send + 'static>) {
    if let Err(poison) = catch_unwind(AssertUnwindSafe(move || drop(payload))) {
        std::mem::forget(poison);
    }
}

/// Joins the engine thread, routing any panic payload through
/// [`drop_guarded`] so a panicking payload `Drop` cannot unwind the joiner
/// (a closer, the reaper, or `Drop`). The engine thread does not panic in
/// normal operation — every closure is guarded and all teardown logging is
/// non-panicking — so this only matters for an abort-class unwind.
fn join_guarded(handle: JoinHandle<()>) {
    if let Err(payload) = handle.join() {
        drop_guarded(payload);
    }
}

/// Drops a value inside [`run_guarded`] so a panicking `Drop` on discarded
/// closure state (e.g. a job's unexecuted `run` body, or a completed job's
/// now-unused `cleanup`) can never unwind the engine thread.
fn guard_drop<T>(what: &str, value: T) {
    run_guarded(what, move || drop(value));
}

/// Runs `job` for the NORMAL (non-closing) path: execute its body guarded,
/// then guard-drop the now-unused cleanup.
fn run_job(job: Job) {
    let Job { run, cleanup } = job;
    run_guarded("engine job", run);
    guard_drop("spent cleanup", cleanup);
}

/// DRAINS `job` on a close path: run its cleanup guarded (waiter completion,
/// global-ref retirement), then guard-drop the unexecuted body.
fn drain_job(job: Job) {
    let Job { run, cleanup } = job;
    if let Some(cleanup) = cleanup {
        run_guarded("drained cleanup", cleanup);
    }
    guard_drop("drained job body", run);
}

/// A queued unit of work. `cleanup` runs ON the engine thread when the job is
/// DRAINED without execution (close path) — dropped jobs may own JNI global
/// refs, which may only be retired with the engine thread's `JNIEnv`.
struct Job {
    run: Box<dyn FnOnce() + Send + 'static>,
    cleanup: Option<Box<dyn FnOnce() + Send + 'static>>,
}

enum Message {
    Job(Job),
    /// Drain marker: everything queued after it is completed with `Closing`
    /// without running; the final job then executes, and the loop exits.
    Close(Job),
}

struct Shared {
    /// Admission and close are ONE mutex-guarded transition: a producer
    /// admitted just before `closing` can never enqueue after the drain.
    queue: Mutex<QueueState>,
    ready: Condvar,
}

struct QueueState {
    messages: VecDeque<Message>,
    /// Deferred final teardown (callback-origin destroy): runs after the
    /// CURRENT engine job returns. Guarded by the same lock as admission so
    /// closing/admission/deferral are one atomic transition.
    deferred_close: Option<Job>,
    closing: bool,
}

thread_local! {
    /// Depth of C-callback frames on the CURRENT thread (set by the Android
    /// callback trampolines). A destroy initiated inside a callback frame
    /// must defer, never block (the no-re-entry rule).
    static CALLBACK_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Bracket a C callback frame (used by the Android trampolines; public for
/// the host-triple tests).
pub fn enter_callback_frame() {
    CALLBACK_DEPTH.with(|depth| depth.set(depth.get() + 1));
}

pub fn exit_callback_frame() {
    CALLBACK_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
}

pub fn in_callback_frame() -> bool {
    CALLBACK_DEPTH.with(|depth| depth.get() > 0)
}

/// Teardown-completion latch, set ON THE ENGINE THREAD as `run_loop` returns
/// — i.e. strictly after the drain and the final job (destroy → window
/// release → global-ref deletion) have completed. Signaling from the engine
/// thread itself (not from whoever joins) makes the latch mean "teardown
/// done" on EVERY path — winner join, reaper join, reaper-spawn failure,
/// Drop. A losing close() that finds the `JoinHandle` already taken parks
/// here; the winner's `join()` is strictly stronger (thread fully exited),
/// so both uphold the synchronous guarantee.
struct JoinSync {
    finished: Mutex<bool>,
    done: Condvar,
    /// Closers currently parked in [`JoinSync::wait_finished`] (test seam).
    waiters: AtomicUsize,
}

/// Sets the teardown latch when the engine thread's stack unwinds — i.e.
/// when `run_loop` returns. On the normal path `run_loop` has already
/// drained the queue and run the final job under `run_guarded`; nothing is
/// left to do, and the guard's only job is to release the latch.
///
/// It also performs a best-effort salvage in case an UNEXPECTED unwind (not
/// from a user closure — those are all caught — but e.g. an allocation
/// failure in the queue machinery) reaches it with work still queued: set
/// closing, drain queued cleanups, run any queued/deferred final job, each
/// guarded, and only THEN release the latch, so a concurrent close() parked
/// on the latch never observes completion before the teardown work ran.
struct FinishGuard {
    shared: Arc<Shared>,
    sync: Arc<JoinSync>,
}

impl Drop for FinishGuard {
    fn drop(&mut self) {
        if thread::panicking() {
            log_teardown!("jian-jni: engine thread unwinding unexpectedly; salvaging teardown");
            salvage_teardown(&self.shared);
        }
        self.sync.mark_finished();
    }
}

/// Best-effort completion of teardown work still parked in the queue after
/// an unexpected unwind. Closes admission first, then runs every remaining
/// cleanup and final job under `run_guarded`. Called ONLY from the panic arm
/// of [`FinishGuard::drop`]; the normal path leaves the queue already empty.
fn salvage_teardown(shared: &Shared) {
    let (drained, deferred) = {
        let mut queue = shared.queue.lock().unwrap();
        queue.closing = true;
        let drained: Vec<Message> = queue.messages.drain(..).collect();
        (drained, queue.deferred_close.take())
    };
    for message in drained {
        match message {
            Message::Job(job) => drain_job(job),
            Message::Close(final_job) => run_guarded("salvaged final job", final_job.run),
        }
    }
    if let Some(final_job) = deferred {
        run_guarded("salvaged deferred final job", final_job.run);
    }
}

impl JoinSync {
    fn mark_finished(&self) {
        *self.finished.lock().unwrap() = true;
        self.done.notify_all();
    }

    fn wait_finished(&self) {
        let mut finished = self.finished.lock().unwrap();
        // The count is bumped under the latch lock so a test can observe
        // "this closer is genuinely parked here" without a timing guess.
        self.waiters.fetch_add(1, Ordering::SeqCst);
        while !*finished {
            finished = self.done.wait(finished).unwrap();
        }
        self.waiters.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Completion of a blocking `call()`: the job ran (owned value), was drained
/// unexecuted (closing), or panicked (payload re-raised on the caller).
enum CallOutcome<R> {
    Ran(R),
    Drained,
    Panicked(Box<dyn Any + Send + 'static>),
}

pub struct EngineThread {
    shared: Arc<Shared>,
    thread_id: ThreadId,
    handle: Mutex<Option<JoinHandle<()>>>,
    join_sync: Arc<JoinSync>,
}

impl EngineThread {
    pub fn spawn(name: &str) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(QueueState {
                messages: VecDeque::new(),
                deferred_close: None,
                closing: false,
            }),
            ready: Condvar::new(),
        });
        let join_sync = Arc::new(JoinSync {
            finished: Mutex::new(false),
            done: Condvar::new(),
            waiters: AtomicUsize::new(0),
        });
        let loop_shared = shared.clone();
        let guard_shared = shared.clone();
        let loop_sync = join_sync.clone();
        let (id_tx, id_rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                id_tx.send(thread::current().id()).ok();
                let _finished = FinishGuard {
                    shared: guard_shared,
                    sync: loop_sync,
                };
                run_loop(loop_shared);
            })
            .expect("spawn engine thread");
        let thread_id = id_rx.recv().expect("engine thread id");

        Self {
            shared,
            thread_id,
            handle: Mutex::new(Some(handle)),
            join_sync,
        }
    }

    /// TEST SEAM (host tests only): whether some closer has already claimed
    /// the `JoinHandle`. Lets a test deterministically construct the
    /// handle-less losing-close() path.
    #[doc(hidden)]
    pub fn teardown_join_claimed(&self) -> bool {
        self.handle.lock().unwrap().is_none()
    }

    /// TEST SEAM (host tests only): closers currently parked on the teardown
    /// latch. Lets a test prove a losing close() actually reached the wait
    /// before releasing the teardown.
    #[doc(hidden)]
    pub fn teardown_waiters(&self) -> usize {
        self.join_sync.waiters.load(Ordering::SeqCst)
    }

    /// TEST SEAM (host tests only): messages currently admitted and queued.
    /// Lets a test prove a blocked `call()`'s job was ADMITTED (enqueued)
    /// before a close, making its `Closing` completion provably a drain.
    #[doc(hidden)]
    pub fn queued_jobs(&self) -> usize {
        self.shared.queue.lock().unwrap().messages.len()
    }

    pub fn is_engine_thread(&self) -> bool {
        thread::current().id() == self.thread_id
    }

    /// Non-blocking post. `Closing` when the queue no longer admits work.
    pub fn post(&self, job: impl FnOnce() + Send + 'static) -> Dispatch<()> {
        self.post_with_cleanup(job, None::<fn()>)
    }

    /// Non-blocking post carrying an engine-thread cleanup for the drained
    /// case (e.g. deleting a JNI global ref owned by the closure).
    pub fn post_with_cleanup(
        &self,
        job: impl FnOnce() + Send + 'static,
        cleanup: Option<impl FnOnce() + Send + 'static>,
    ) -> Dispatch<()> {
        let mut queue = self.shared.queue.lock().unwrap();
        if queue.closing {
            return Dispatch::Closing;
        }
        queue.messages.push_back(Message::Job(Job {
            run: Box::new(job),
            cleanup: cleanup.map(|f| Box::new(f) as Box<dyn FnOnce() + Send>),
        }));
        drop(queue);
        self.shared.ready.notify_one();
        Dispatch::Done(())
    }

    /// Blocking post-and-wait returning the job's OWNED result.
    ///
    /// Runs the job directly when already on the engine thread — INCLUDING
    /// from inside an FFI callback frame: the job executes and the C ABI
    /// itself reports `WrongThread` for synchronous re-entry (the queue must
    /// not mask that as `Closing`). Only callback-origin destroy is special
    /// (`close_deferred`).
    ///
    /// A panic in `job` is captured on the engine thread (the engine keeps
    /// running) and re-raised on THIS thread, exactly like
    /// [`std::thread::JoinHandle::join`] — the caller is never left parked.
    pub fn call<R: Send + 'static>(&self, job: impl FnOnce() -> R + Send + 'static) -> Dispatch<R> {
        if self.is_engine_thread() {
            return Dispatch::Done(job());
        }
        let result: Arc<(Mutex<Option<CallOutcome<R>>>, Condvar)> =
            Arc::new((Mutex::new(None), Condvar::new()));
        let job_result = result.clone();
        let cleanup_result = result.clone();
        let posted = self.post_with_cleanup(
            move || {
                // Capture a panic so it neither kills the engine thread nor
                // leaves the caller parked; the caller re-raises it.
                let outcome = catch_unwind(AssertUnwindSafe(job));
                let (lock, signal) = &*job_result;
                *lock.lock().unwrap() = Some(match outcome {
                    Ok(value) => CallOutcome::Ran(value),
                    Err(payload) => CallOutcome::Panicked(payload),
                });
                signal.notify_one();
            },
            Some(move || {
                // Drained without execution: complete the waiter with a
                // Drained outcome so no caller is left parked on the latch.
                let (lock, signal) = &*cleanup_result;
                let mut slot = lock.lock().unwrap();
                if slot.is_none() {
                    *slot = Some(CallOutcome::Drained);
                }
                signal.notify_one();
            }),
        );
        if posted.is_closing() {
            return Dispatch::Closing;
        }
        let (lock, signal) = &*result;
        let mut slot = lock.lock().unwrap();
        while slot.is_none() {
            slot = signal.wait(slot).unwrap();
        }
        match slot.take().expect("completed dispatch") {
            CallOutcome::Ran(value) => Dispatch::Done(value),
            CallOutcome::Drained => Dispatch::Closing,
            CallOutcome::Panicked(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Callback-origin destroy (§6.7): callable only on the engine thread,
    /// from inside an FFI callback frame. In ONE guarded transition it sets
    /// the closing flag — every subsequent dispatch is rejected immediately —
    /// and parks `final_job` to run right after the CURRENT engine job (the
    /// outer FFI call) returns, honoring the no-re-entry rule. The loop then
    /// drains the queue cleanup-only, runs `final_job`, and exits.
    pub fn close_deferred(&self, final_job: impl FnOnce() + Send + 'static) -> Dispatch<()> {
        if !self.is_engine_thread() {
            return Dispatch::Closing;
        }
        let mut queue = self.shared.queue.lock().unwrap();
        if queue.closing {
            return Dispatch::Closing;
        }
        queue.closing = true;
        queue.deferred_close = Some(Job {
            run: Box::new(final_job),
            cleanup: None,
        });
        Dispatch::Done(())
    }

    /// §6.7 teardown. Sets closing (atomically with admission), drains every
    /// queued message ON the engine thread — running cleanups, never jobs —
    /// runs `final_job` strictly last (jian_destroy → window release →
    /// global-ref deletion), then the loop exits. The join runs here, or on a
    /// freshly spawned reaper thread when close() is invoked from the engine
    /// thread itself (a thread cannot join itself). See [`Self::close_deferred`]
    /// for the callback-origin variant.
    ///
    /// A concurrent close() that arrives after the winning transition drops
    /// its own `final_job` (exactly one destroy runs) but still blocks until
    /// the active teardown has fully completed — the synchronous guarantee
    /// holds for every off-thread closer.
    pub fn close(&self, final_job: impl FnOnce() + Send + 'static) {
        let queued = {
            let mut queue = self.shared.queue.lock().unwrap();
            if queue.closing {
                // A teardown is already active (concurrent close or a
                // callback-origin deferred close won). This close's final_job
                // is dropped — exactly one destroy runs — but the SYNCHRONOUS
                // guarantee still holds: fall through to the join/latch below
                // and wait for the active teardown to finish.
                false
            } else {
                queue.closing = true;
                queue.messages.push_back(Message::Close(Job {
                    run: Box::new(final_job),
                    cleanup: None,
                }));
                true
            }
        };
        if queued {
            self.shared.ready.notify_one();
        }

        let handle = self.handle.lock().unwrap().take();
        if self.is_engine_thread() {
            // The loop will exit once the current job (us) returns; a reaper
            // owns the join (a thread cannot join itself). The engine thread
            // itself never waits on the latch — it IS the teardown.
            if let Some(handle) = handle {
                if let Err(error) = thread::Builder::new()
                    .name("jian-engine-reaper".into())
                    .spawn(move || join_guarded(handle))
                {
                    // Never unwind the engine job: detaching leaks one joinable
                    // handle under resource exhaustion, which is strictly
                    // better than skipping the drain + final teardown. The
                    // teardown latch is untouched here — the engine thread
                    // itself sets it when the drain + final job complete, so
                    // a parked concurrent closer still wakes at the right
                    // time.
                    log_teardown!(
                        "jian-jni: reaper spawn failed ({error}); detaching engine thread"
                    );
                }
            }
        } else if let Some(handle) = handle {
            if let Err(payload) = handle.join() {
                // An unexpected unwind: the FinishGuard has already salvaged
                // the drain + final job; surface the panic (non-panicking
                // log) and route the payload through the guard so a panicking
                // payload Drop cannot unwind this closer.
                log_teardown!("jian-jni: engine thread panicked during teardown");
                drop_guarded(payload);
            }
        } else {
            // Losing close(): another closer already owns the join (or a
            // callback-origin close_deferred won and Drop took the handle).
            // The synchronous guarantee still holds — park on the latch,
            // which the ENGINE THREAD sets only after the drain and the
            // final job have completed.
            self.join_sync.wait_finished();
        }
    }
}

impl Drop for EngineThread {
    fn drop(&mut self) {
        // A dropped-without-close queue still terminates: mark closing and
        // wake the loop; queued cleanups run on the engine thread.
        let handle = self.handle.lock().unwrap().take();
        if let Some(handle) = handle {
            {
                let mut queue = self.shared.queue.lock().unwrap();
                queue.closing = true;
                queue.messages.push_back(Message::Close(Job {
                    run: Box::new(|| {}),
                    cleanup: None,
                }));
            }
            self.shared.ready.notify_one();
            if thread::current().id() != self.thread_id {
                join_guarded(handle);
            } else {
                thread::Builder::new()
                    .name("jian-engine-reaper".into())
                    .spawn(move || join_guarded(handle))
                    .ok();
            }
        }
    }
}

fn run_loop(shared: Arc<Shared>) {
    loop {
        let (message, closing) = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if let Some(message) = queue.messages.pop_front() {
                    break (message, queue.closing);
                }
                queue = shared.ready.wait(queue).unwrap();
            }
        };
        match message {
            // Once closing is set, jobs still in the queue — including ones
            // enqueued BEFORE close() that never started — drain without
            // execution; only their cleanups (waiter completion, global-ref
            // retirement) run, on this thread.
            Message::Job(job) if closing => {
                // Drain: run the cleanup guarded and guard-drop the
                // unexecuted body — either could carry a panicking Drop.
                drain_job(job);
                // A callback-origin close parked its teardown while this job
                // was already dequeued-but-drained; honor it now.
                if let Some(final_job) = drain_then_take_deferred(&shared) {
                    run_guarded("final job", final_job.run);
                    return;
                }
            }
            Message::Job(job) => {
                // A user job never unwinds the engine thread: post() jobs are
                // fire-and-forget (a panic is logged and swallowed); call()
                // jobs capture their own panic and re-raise it on the caller.
                // The now-unused cleanup is guard-dropped by run_job.
                run_job(job);
                // A callback-origin close (close_deferred) set closing and
                // parked the final teardown while this job ran: drain the
                // queue cleanup-only, run it strictly last, and exit.
                if let Some(final_job) = drain_then_take_deferred(&shared) {
                    run_guarded("final job", final_job.run);
                    return;
                }
            }
            Message::Close(final_job) => {
                // Drain WITHOUT execution, completing each waiter via its
                // cleanup, all on this thread — each guarded so one panicking
                // cleanup (or a panicking Drop on a discarded body) cannot
                // drop the destroy that follows.
                loop {
                    let drained = shared.queue.lock().unwrap().messages.pop_front();
                    match drained {
                        Some(Message::Job(job)) => drain_job(job),
                        Some(Message::Close(extra)) => {
                            // A second close (e.g. Drop after close) — its
                            // final job is a no-op by construction.
                            run_guarded("extra final job", extra.run);
                        }
                        None => break,
                    }
                }
                run_guarded("final job", final_job.run);
                return;
            }
        }
    }
}

/// When a callback-origin close parked a deferred teardown, drain the queue
/// cleanup-only (under the admission lock, batch-wise) and hand the final job
/// back to the loop. Returns `None` when no deferred close is pending. Each
/// drained cleanup runs guarded so a panic cannot drop the returned final
/// job.
fn drain_then_take_deferred(shared: &Arc<Shared>) -> Option<Job> {
    let final_job = {
        let mut queue = shared.queue.lock().unwrap();
        let final_job = queue.deferred_close.take()?;
        // Everything admitted before closing was set drains here.
        let drained: Vec<Message> = queue.messages.drain(..).collect();
        drop(queue);
        for message in drained {
            match message {
                Message::Job(job) => drain_job(job),
                Message::Close(extra) => run_guarded("extra final job", extra.run),
            }
        }
        final_job
    };
    Some(final_job)
}
