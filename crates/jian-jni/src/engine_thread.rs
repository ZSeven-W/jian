//! The ONE dedicated engine thread (spec §6.7): every engine call dispatches
//! onto it; lifecycle calls block on a post-and-wait barrier; teardown drains
//! the queue on the engine thread, completes every parked waiter with
//! [`Dispatch::Closing`], runs the final job (destroy → window release →
//! global-ref deletion) strictly before the thread quits, and joins from a
//! reaper thread when close() originates on the engine thread itself.
//!
//! Pure `std` — host-testable; the JNI/NDK edges live in the Android-only
//! modules and only hand this queue closures.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle, ThreadId};

/// Shell-side dispatch-rejected status (never produced by the C ABI).
pub const STATUS_CLOSING: i32 = -1;

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
    /// The engine thread died by PANIC (salvage path): no engine job will
    /// ever run again. A later close() must not park its final job in the
    /// queue — it runs it on the calling thread instead (the only thread
    /// left that can).
    dead: bool,
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

/// Teardown-completion latch, set ON THE ENGINE THREAD as `run_loop`
/// returns — i.e. strictly after the drain and the final job (destroy →
/// window release → global-ref deletion) have completed. Signaling from the
/// engine thread itself (not from whoever joins) makes the latch mean
/// "teardown done" on EVERY path — winner join, reaper join, reaper-spawn
/// failure, Drop — and a drop guard sets it even if a job panics, so a
/// parked closer can never hang. A losing close() that finds the
/// `JoinHandle` already taken parks here; the winner's `join()` is strictly
/// stronger (thread fully exited), so both uphold the synchronous
/// guarantee.
struct JoinSync {
    finished: Mutex<bool>,
    done: Condvar,
    /// Closers currently parked in [`JoinSync::wait_finished`] (test seam).
    waiters: AtomicUsize,
}

/// Runs on every engine-thread exit — normal return AND unwind. On the
/// normal path `run_loop` has already drained the queue and run the final
/// job, so the salvage pass finds nothing and only the latch is set. On
/// unwind (a panicking job, cleanup, or final job) the guard still performs
/// the outstanding teardown obligations — set closing, drain queued
/// cleanups and any queued `Close` final job, run a parked deferred close —
/// each under `catch_unwind` so one panicking closure cannot skip the rest.
/// Only THEN is the latch set: a parked closer never observes "finished"
/// before the teardown work was actually attempted.
struct FinishGuard {
    shared: Arc<Shared>,
    sync: Arc<JoinSync>,
}

impl Drop for FinishGuard {
    fn drop(&mut self) {
        if thread::panicking() {
            eprintln!("jian-jni: engine thread unwinding; salvaging teardown");
        }
        salvage_teardown(&self.shared);
        self.sync.mark_finished();
    }
}

/// Completes any teardown work still parked in the queue. Closures run
/// OUTSIDE the admission lock, each under `catch_unwind`.
fn salvage_teardown(shared: &Shared) {
    let (drained, deferred) = {
        let mut queue = shared.queue.lock().unwrap();
        // `dead` is set ONLY when the thread exits with closing still unset —
        // i.e. a mid-life panic with NO destroy in flight anywhere. If
        // closing was already set, the destroy is either queued right here
        // (run in the drain below) or owned by a closer's transition, and a
        // later close() must still drop its final job (exactly-one-destroy).
        if !queue.closing {
            queue.closing = true;
            queue.dead = true;
        }
        let drained: Vec<Message> = queue.messages.drain(..).collect();
        (drained, queue.deferred_close.take())
    };
    let run = |f: Box<dyn FnOnce() + Send>| {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
            eprintln!("jian-jni: teardown closure panicked during salvage");
        }
    };
    for message in drained {
        match message {
            Message::Job(job) => {
                if let Some(cleanup) = job.cleanup {
                    run(cleanup);
                }
            }
            Message::Close(final_job) => run(final_job.run),
        }
    }
    if let Some(final_job) = deferred {
        run(final_job.run);
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
                dead: false,
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

    /// TEST SEAM (host tests only): closers currently parked on the
    /// teardown latch. Lets a test prove a losing close() actually reached
    /// the wait before releasing the teardown.
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
    /// (`post_deferred`).
    pub fn call<R: Send + 'static>(&self, job: impl FnOnce() -> R + Send + 'static) -> Dispatch<R> {
        if self.is_engine_thread() {
            return Dispatch::Done(job());
        }
        let result: Arc<(Mutex<Option<Dispatch<R>>>, Condvar)> =
            Arc::new((Mutex::new(None), Condvar::new()));
        let job_result = result.clone();
        let cleanup_result = result.clone();
        let posted = self.post_with_cleanup(
            move || {
                let value = job();
                let (lock, signal) = &*job_result;
                *lock.lock().unwrap() = Some(Dispatch::Done(value));
                signal.notify_one();
            },
            Some(move || {
                // Drained without execution: complete the waiter with Closing
                // so no caller is left parked on the latch.
                let (lock, signal) = &*cleanup_result;
                *lock.lock().unwrap() = Some(Dispatch::Closing);
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
        slot.take().expect("completed dispatch")
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
    /// If the engine thread previously DIED BY PANIC with no destroy in
    /// flight (`dead`), the final job runs on the calling thread instead —
    /// the engine thread no longer exists, and the JNI operations in a
    /// destroy final job (global-ref deletion, window release) are legal
    /// from any attached thread. Only the FIRST post-death close() does
    /// this; exactly one destroy runs.
    pub fn close(&self, final_job: impl FnOnce() + Send + 'static) {
        enum Route {
            Queued,
            AlreadyClosing,
            DeadFallback,
        }
        let final_job: Box<dyn FnOnce() + Send> = Box::new(final_job);
        let (route, final_job) = {
            let mut queue = self.shared.queue.lock().unwrap();
            if queue.dead {
                queue.dead = false;
                (Route::DeadFallback, Some(final_job))
            } else if queue.closing {
                // A teardown is already active (concurrent close or a
                // callback-origin deferred close won). This close's final_job
                // is dropped — exactly one destroy runs — but the SYNCHRONOUS
                // guarantee still holds: fall through to the join below and
                // wait for the active teardown to finish.
                (Route::AlreadyClosing, None)
            } else {
                queue.closing = true;
                queue.messages.push_back(Message::Close(Job {
                    run: final_job,
                    cleanup: None,
                }));
                (Route::Queued, None)
            }
        };
        match route {
            Route::Queued => self.shared.ready.notify_one(),
            Route::AlreadyClosing => {}
            Route::DeadFallback => {
                eprintln!(
                    "jian-jni: engine thread died by panic; running the final \
                     teardown on the closing thread"
                );
                if let Some(final_job) = final_job {
                    final_job();
                }
            }
        }

        let handle = self.handle.lock().unwrap().take();
        if self.is_engine_thread() {
            // The loop will exit once the current job (us) returns; a reaper
            // owns the join (a thread cannot join itself). The engine thread
            // itself never waits on the latch — it IS the teardown.
            if let Some(handle) = handle {
                if let Err(error) = thread::Builder::new()
                    .name("jian-engine-reaper".into())
                    .spawn(move || {
                        let _ = handle.join();
                    })
                {
                    // Never unwind the engine job: detaching leaks one joinable
                    // handle under resource exhaustion, which is strictly
                    // better than skipping the drain + final teardown. The
                    // teardown latch is untouched here — the engine thread
                    // itself sets it when the drain + final job complete, so
                    // a parked concurrent closer still wakes at the right
                    // time.
                    eprintln!("jian-jni: reaper spawn failed ({error}); detaching engine thread");
                }
            }
        } else if let Some(handle) = handle {
            if handle.join().is_err() {
                // The FinishGuard has already salvaged the drain + final job
                // on the unwind path; surface the panic instead of hiding it.
                eprintln!("jian-jni: engine thread panicked during teardown");
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
                let _ = handle.join();
            } else {
                thread::Builder::new()
                    .name("jian-engine-reaper".into())
                    .spawn(move || {
                        let _ = handle.join();
                    })
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
                if let Some(cleanup) = job.cleanup {
                    cleanup();
                }
                // A callback-origin close parked its teardown while this job
                // was already dequeued-but-drained; honor it now.
                if let Some(final_job) = drain_then_take_deferred(&shared) {
                    (final_job.run)();
                    return;
                }
            }
            Message::Job(job) => {
                (job.run)();
                // A callback-origin close (close_deferred) set closing and
                // parked the final teardown while this job ran: drain the
                // queue cleanup-only, run it strictly last, and exit.
                if let Some(final_job) = drain_then_take_deferred(&shared) {
                    (final_job.run)();
                    return;
                }
            }
            Message::Close(final_job) => {
                // Drain WITHOUT execution, completing each waiter via its
                // cleanup, all on this thread.
                loop {
                    let drained = shared.queue.lock().unwrap().messages.pop_front();
                    match drained {
                        Some(Message::Job(job)) => {
                            if let Some(cleanup) = job.cleanup {
                                cleanup();
                            }
                        }
                        Some(Message::Close(extra)) => {
                            // A second close (e.g. Drop after close) — its
                            // final job is a no-op by construction.
                            (extra.run)();
                        }
                        None => break,
                    }
                }
                (final_job.run)();
                return;
            }
        }
    }
}

/// When a callback-origin close parked a deferred teardown, drain the queue
/// cleanup-only (under the admission lock, batch-wise) and hand the final job
/// back to the loop. Returns `None` when no deferred close is pending.
fn drain_then_take_deferred(shared: &Arc<Shared>) -> Option<Job> {
    let final_job = {
        let mut queue = shared.queue.lock().unwrap();
        let final_job = queue.deferred_close.take()?;
        // Everything admitted before closing was set drains here.
        let drained: Vec<Message> = queue.messages.drain(..).collect();
        drop(queue);
        for message in drained {
            match message {
                Message::Job(job) => {
                    if let Some(cleanup) = job.cleanup {
                        cleanup();
                    }
                }
                Message::Close(extra) => (extra.run)(),
            }
        }
        final_job
    };
    Some(final_job)
}
