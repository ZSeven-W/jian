//! Host-triple tests for the §6.7 queue core — one test per contract clause
//! (M4 plan Task 4 Step 2, cases (a)–(p)).

use jian_jni::engine_thread::{enter_callback_frame, exit_callback_frame, Dispatch, EngineThread};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

fn with_timeout(test: impl FnOnce() + Send + 'static) {
    // Every case is deadlock-sensitive; a wedged queue must fail, not hang CI.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        test();
        tx.send(()).ok();
    });
    rx.recv_timeout(Duration::from_secs(30))
        .expect("test wedged (deadlock or parked waiter)");
}

// (a) post/call round-trips off-thread.
#[test]
fn post_and_call_round_trip() {
    with_timeout(|| {
        let engine = EngineThread::spawn("t-a");
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        assert_eq!(
            engine.post(move || {
                h.fetch_add(1, Ordering::SeqCst);
            }),
            Dispatch::Done(())
        );
        let value = engine.call(|| 41 + 1);
        assert_eq!(value, Dispatch::Done(42));
        assert_eq!(hits.load(Ordering::SeqCst), 1); // FIFO: post ran first
        engine.close(|| {});
    });
}

// (b) call on the engine thread runs direct (no self-deadlock).
#[test]
fn call_on_engine_thread_runs_direct() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-b"));
        let inner = engine.clone();
        let result = engine.call(move || {
            assert!(inner.is_engine_thread());
            match inner.call(|| 7) {
                Dispatch::Done(v) => v,
                Dispatch::Closing => panic!("direct call must run"),
            }
        });
        assert_eq!(result, Dispatch::Done(7));
        engine.close(|| {});
    });
}

// (c) closing rejects new dispatches with Closing.
#[test]
fn closing_rejects_new_dispatches() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-c"));
        let entered = Arc::new(Barrier::new(2));
        let gate = Arc::new(Barrier::new(2));
        let e = entered.clone();
        let g = gate.clone();
        // Park the engine thread so close() queues behind a running job.
        let _ = engine.post(move || {
            e.wait();
            g.wait();
        });
        // Entry handshake: without it, a fast close() could land before the
        // loop dequeues this job — the job would DRAIN (g.wait() never runs)
        // and the release below would wedge.
        entered.wait();
        let closer_engine = engine.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let closed_flag = closed.clone();
        let closer = std::thread::spawn(move || {
            closer_engine.close(move || closed_flag.store(true, Ordering::SeqCst));
        });
        // Wait until close() has marked the queue closing, then verify
        // rejection while the loop is still parked on the barrier.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if engine.post(|| {}).is_closing() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "closing never took effect"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(engine.call(|| 1).is_closing());
        gate.wait();
        closer.join().unwrap();
        assert!(closed.load(Ordering::SeqCst), "final job ran");
    });
}

// (d) drained messages complete their waiters with Closing (no parked
// caller) AND their cleanups ran on the engine thread.
#[test]
fn drain_completes_waiters_and_runs_cleanups_on_engine_thread() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-d"));
        let engine_id = match engine.call(std::thread::current) {
            Dispatch::Done(t) => t.id(),
            Dispatch::Closing => panic!("engine alive"),
        };

        let entered = Arc::new(Barrier::new(2));
        let gate = Arc::new(Barrier::new(2));
        let e = entered.clone();
        let g = gate.clone();
        let _ = engine.post(move || {
            e.wait();
            g.wait(); // hold the loop so later messages stay queued
        });
        entered.wait(); // the loop is provably INSIDE the held job

        // A blocking caller queued behind the held job. Admission handshake:
        // the loop is parked inside the gate job, so the queue length rising
        // to 1 (test seam, read under the admission lock) proves the call()'s
        // job was ENQUEUED — and closing cannot be set yet (close() comes
        // later), so its Closing completion below is provably a DRAIN of an
        // admitted, parked call(), not a rejection.
        let waiter_engine = engine.clone();
        let waiter = std::thread::spawn(move || waiter_engine.call(|| 5));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while engine.queued_jobs() < 1 {
            assert!(
                std::time::Instant::now() < deadline,
                "the call() waiter never enqueued"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        // A posted job with a cleanup that records its thread; its Done ack
        // proves admission before closing.
        let cleanup_thread = Arc::new(Mutex::new(None));
        let ct = cleanup_thread.clone();
        assert_eq!(
            engine.post_with_cleanup(
                || panic!("drained job must not run"),
                Some(move || {
                    *ct.lock().unwrap() = Some(std::thread::current().id());
                }),
            ),
            Dispatch::Done(())
        );

        let closer_engine = engine.clone();
        let closer = std::thread::spawn(move || closer_engine.close(|| {}));
        // Wait until the closer's guarded transition has landed before
        // releasing the held job.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !engine.post(|| {}).is_closing() {
            assert!(
                std::time::Instant::now() < deadline,
                "closing never took effect"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        gate.wait(); // release the held job → drain begins

        assert_eq!(waiter.join().unwrap(), Dispatch::Closing);
        closer.join().unwrap();
        assert_eq!(
            *cleanup_thread.lock().unwrap(),
            Some(engine_id),
            "drained cleanup must run on the engine thread"
        );
    });
}

// (e) close from the engine thread joins via the reaper (no deadlock).
#[test]
fn close_from_engine_thread_uses_reaper() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-e"));
        let done = Arc::new(AtomicBool::new(false));
        let done_flag = done.clone();
        let inner = engine.clone();
        let _ = engine.post(move || {
            inner.close(move || done_flag.store(true, Ordering::SeqCst));
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done.load(Ordering::SeqCst) {
            assert!(std::time::Instant::now() < deadline, "final job never ran");
            std::thread::sleep(Duration::from_millis(20));
        }
    });
}

// (f) callback-origin deferred close: runs after the outer job returns
// without blocking the caller, and closing takes effect IMMEDIATELY (the
// same guarded transition), rejecting later dispatches.
#[test]
fn callback_frame_deferral_runs_after_outer_job() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-f"));
        let log = Arc::new(Mutex::new(Vec::new()));
        let l1 = log.clone();
        let l2 = log.clone();
        let inner = engine.clone();
        let result = engine.call(move || {
            enter_callback_frame();
            let deferred = inner.close_deferred(move || l2.lock().unwrap().push("final"));
            assert_eq!(deferred, Dispatch::Done(()));
            // Closing already took effect: a dispatch from THIS point on is
            // rejected, even before the outer job returns.
            assert!(inner.post(|| {}).is_closing());
            exit_callback_frame();
            l1.lock().unwrap().push("outer");
            "returned"
        });
        assert_eq!(result, Dispatch::Done("returned"));
        // Deferred teardown ran strictly after the outer job.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if *log.lock().unwrap() == vec!["outer", "final"] {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "deferred close never ran"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(engine.call(|| 1).is_closing());
    });
}

// (g) failed-create teardown (close with a no-op final job) is clean.
#[test]
fn failed_create_teardown_is_clean() {
    with_timeout(|| {
        let engine = EngineThread::spawn("t-g");
        engine.close(|| {});
    });
}

// (h) final_job runs strictly after the drain and strictly before quit.
#[test]
fn final_job_runs_after_drain_before_quit() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-h"));
        let entered = Arc::new(Barrier::new(2));
        let gate = Arc::new(Barrier::new(2));
        let e = entered.clone();
        let g = gate.clone();
        let _ = engine.post(move || {
            e.wait();
            g.wait();
        });
        entered.wait(); // the loop is provably INSIDE the held job
        let log = Arc::new(Mutex::new(Vec::new()));
        let lc = log.clone();
        let _ = engine.post_with_cleanup(
            || panic!("drained"),
            Some(move || lc.lock().unwrap().push("cleanup")),
        );
        // Admission handshake: the cleanup-job must be ENQUEUED before the
        // close, so it drains (cleanup fires) instead of running its job.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while engine.queued_jobs() < 1 {
            assert!(
                std::time::Instant::now() < deadline,
                "the cleanup job never enqueued"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let lf = log.clone();
        let closer_engine = engine.clone();
        let closer = std::thread::spawn(move || {
            closer_engine.close(move || lf.lock().unwrap().push("final"));
        });
        // Close-landed handshake: releasing the gate before closing is
        // observed could let the cleanup job RUN instead of drain.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !engine.post(|| {}).is_closing() {
            assert!(
                std::time::Instant::now() < deadline,
                "closing never took effect"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        gate.wait();
        closer.join().unwrap();
        assert_eq!(*log.lock().unwrap(), vec!["cleanup", "final"]);
    });
}

// (i) destroy-during-frame concurrency: an in-flight job completes, queued
// jobs drain with Closing, no waiter parks.
#[test]
fn destroy_during_inflight_job() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-i"));
        let inflight_done = Arc::new(AtomicBool::new(false));
        let flag = inflight_done.clone();
        let entered = Arc::new(Barrier::new(2));
        let gate = Arc::new(Barrier::new(2));
        let e = entered.clone();
        let g = gate.clone();
        let _ = engine.post(move || {
            e.wait();
            g.wait();
            std::thread::sleep(Duration::from_millis(120));
            flag.store(true, Ordering::SeqCst);
        });
        entered.wait(); // the loop is provably INSIDE the in-flight job
        let waiter_engine = engine.clone();
        let waiter = std::thread::spawn(move || waiter_engine.call(|| 9));
        // Admission handshake: the waiter's job is ENQUEUED behind the
        // in-flight job before the close, so its Closing is provably a drain.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while engine.queued_jobs() < 1 {
            assert!(
                std::time::Instant::now() < deadline,
                "the call() waiter never enqueued"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let closer_engine = engine.clone();
        let closer = std::thread::spawn(move || closer_engine.close(|| {}));
        // Close-landed handshake: the close is pending (closing observed)
        // while the in-flight job is still held.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !engine.post(|| {}).is_closing() {
            assert!(
                std::time::Instant::now() < deadline,
                "closing never took effect"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        gate.wait(); // in-flight job proceeds while close is pending
        closer.join().unwrap();
        assert!(
            inflight_done.load(Ordering::SeqCst),
            "in-flight job completed"
        );
        assert_eq!(waiter.join().unwrap(), Dispatch::Closing);
    });
}

// (j) late-enqueue race (barrier-paced): a racer either lands BEFORE the
// drain (its cleanup fires / it is rejected at admission) — it never
// executes after the final job.
#[test]
fn late_enqueue_race_never_lands_after_final_job() {
    with_timeout(|| {
        for _ in 0..100 {
            let engine = Arc::new(EngineThread::spawn("t-j"));
            let final_ran = Arc::new(AtomicBool::new(false));
            let ran_after_final = Arc::new(AtomicBool::new(false));
            let start = Arc::new(Barrier::new(2));
            let start_racer = start.clone();

            let racer_engine = engine.clone();
            let ff = final_ran.clone();
            let jf = ran_after_final.clone();
            let ran = Arc::new(AtomicBool::new(false));
            let drained = Arc::new(AtomicBool::new(false));
            let ran_flag = ran.clone();
            let drained_flag = drained.clone();
            let racer = std::thread::spawn(move || {
                start_racer.wait();
                racer_engine.post_with_cleanup(
                    move || {
                        ran_flag.store(true, Ordering::SeqCst);
                        if ff.load(Ordering::SeqCst) {
                            jf.store(true, Ordering::SeqCst);
                        }
                    },
                    Some(move || drained_flag.store(true, Ordering::SeqCst)),
                )
            });
            let fr = final_ran.clone();
            start.wait();
            engine.close(move || fr.store(true, Ordering::SeqCst));
            let admitted = !racer.join().unwrap().is_closing();
            assert!(
                !ran_after_final.load(Ordering::SeqCst),
                "an admitted job executed after the final job"
            );
            // Total accounting: an admitted job either RAN (before final) or
            // was DRAINED (cleanup fired) — never silently lost.
            if admitted {
                let ran = ran.load(Ordering::SeqCst);
                let drained = drained.load(Ordering::SeqCst);
                assert!(
                    ran ^ drained,
                    "admitted job must EITHER run OR drain (ran={ran}, drained={drained})"
                );
            }
        }
        // Deterministic admitted case (the 100 racing rounds above may, on an
        // adversarial scheduler, all reject at admission): a post BEFORE
        // close() is GUARANTEED admitted, and must still obey the xor
        // accounting — run or drain, never lost, never after the final job.
        let engine = Arc::new(EngineThread::spawn("t-j-det"));
        let final_ran = Arc::new(AtomicBool::new(false));
        let ran_after_final = Arc::new(AtomicBool::new(false));
        let ran = Arc::new(AtomicBool::new(false));
        let drained = Arc::new(AtomicBool::new(false));
        let ff = final_ran.clone();
        let jf = ran_after_final.clone();
        let ran_flag = ran.clone();
        let drained_flag = drained.clone();
        let admitted = engine.post_with_cleanup(
            move || {
                ran_flag.store(true, Ordering::SeqCst);
                if ff.load(Ordering::SeqCst) {
                    jf.store(true, Ordering::SeqCst);
                }
            },
            Some(move || drained_flag.store(true, Ordering::SeqCst)),
        );
        assert_eq!(admitted, Dispatch::Done(()), "pre-close post is admitted");
        let fr = final_ran.clone();
        engine.close(move || fr.store(true, Ordering::SeqCst));
        assert!(!ran_after_final.load(Ordering::SeqCst));
        let ran = ran.load(Ordering::SeqCst);
        let drained = drained.load(Ordering::SeqCst);
        assert!(
            ran ^ drained,
            "admitted job must EITHER run OR drain (ran={ran}, drained={drained})"
        );
    });
}

// (l) concurrent double-close: the LOSING close() (closing already set,
// JoinHandle already taken by the winner) still blocks until the active
// teardown — drain + final job + engine-thread exit — has completed. The
// synchronous guarantee holds for every off-thread closer, not just the
// winner.
#[test]
fn concurrent_close_loser_waits_for_active_teardown() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-l"));
        let entered = Arc::new(Barrier::new(2));
        let gate = Arc::new(Barrier::new(2));
        let e = entered.clone();
        let g = gate.clone();
        // Park the loop so the winner's join blocks and teardown stays active.
        let _ = engine.post(move || {
            e.wait();
            g.wait();
        });
        // Entry handshake: the winner's close() must not be able to land
        // before the loop is inside this job — otherwise the job DRAINS,
        // teardown completes immediately, and the loser legitimately returns.
        entered.wait();

        let final_ran = Arc::new(AtomicBool::new(false));
        let winner_engine = engine.clone();
        let fr = final_ran.clone();
        let winner = std::thread::spawn(move || {
            winner_engine.close(move || fr.store(true, Ordering::SeqCst));
        });
        // Deterministic handle-less setup: wait until the winner has BOTH
        // landed the guarded closing transition AND claimed the JoinHandle
        // (test seam) — only then can the loser start, so it is guaranteed
        // to find the handle gone and take the latch path.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !(engine.post(|| {}).is_closing() && engine.teardown_join_claimed()) {
            assert!(
                std::time::Instant::now() < deadline,
                "winner never claimed the teardown join"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        let loser_engine = engine.clone();
        let fr_seen = final_ran.clone();
        let loser = std::thread::spawn(move || {
            loser_engine.close(|| panic!("losing final job must never run"));
            // The synchronous guarantee: when the losing close() returns, the
            // winner's final job has already run.
            assert!(
                fr_seen.load(Ordering::SeqCst),
                "losing close() returned before the active teardown finished"
            );
        });
        // Latch-wait handshake: the loser is provably PARKED on the teardown
        // latch (waiter count, bumped under the latch lock) before the loop
        // is released — an early-returning wait_finished() can never reach
        // this state and fails the deadline below.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while engine.teardown_waiters() == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "losing close() never parked on the teardown latch"
            );
            assert!(!loser.is_finished(), "losing close() returned early");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!loser.is_finished(), "losing close() returned early");

        gate.wait(); // release the loop → drain → final job → exit
        winner.join().unwrap();
        loser.join().unwrap();
        assert!(final_ran.load(Ordering::SeqCst));
    });
}

// (m) a panicking post() job must NOT kill the engine thread: the panic is
// caught and swallowed (fire-and-forget has no caller), and the engine keeps
// processing — a subsequent call() returns normally, and close() still tears
// down cleanly.
#[test]
fn panicking_post_job_does_not_kill_the_engine() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-m"));
        assert_eq!(
            engine.post(|| panic!("boom (deliberate test panic)")),
            Dispatch::Done(())
        );
        // The engine survives: a job queued AFTER the panicking one runs.
        assert_eq!(engine.call(|| 21 + 21), Dispatch::Done(42));
        let final_ran = Arc::new(AtomicBool::new(false));
        let fr = final_ran.clone();
        engine.close(move || fr.store(true, Ordering::SeqCst));
        assert!(
            final_ran.load(Ordering::SeqCst),
            "the destroy must run after a survived panic"
        );
    });
}

// (n) a panicking call() job must neither deadlock the caller nor kill the
// engine: the panic is captured on the engine thread and RE-RAISED on the
// calling thread (JoinHandle::join semantics), while the engine keeps
// serving later calls.
#[test]
fn call_job_panic_propagates_without_deadlock() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-n"));
        let caller_engine = engine.clone();
        let caller = std::thread::spawn(move || {
            caller_engine.call(|| -> i32 { panic!("boom (deliberate test panic)") })
        });
        // The panic surfaces on the caller (Err), NOT a park.
        assert!(
            caller.join().is_err(),
            "the engine job's panic must re-raise on the caller, not park it"
        );
        // The engine is still alive and serving.
        assert_eq!(engine.call(|| 7 * 6), Dispatch::Done(42));
        engine.close(|| {});
    });
}

// (o) a panicking cleanup on the callback-origin (deferred) drain must not
// lose the destroy. This is the load-bearing case: in
// `drain_then_take_deferred` the final job is a LOCAL taken out of shared
// state before the cleanups run, so an unguarded cleanup panic would drop it
// with no salvage able to recover it. Guarded, the remaining cleanup still
// runs and the deferred final job still executes.
#[test]
fn panicking_cleanup_on_deferred_drain_keeps_the_destroy() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-o"));
        let final_ran = Arc::new(AtomicBool::new(false));
        let second_cleanup = Arc::new(AtomicBool::new(false));

        let inner = engine.clone();
        let fr = final_ran.clone();
        let gate = Arc::new(Barrier::new(2));
        let admitted_gate = Arc::new(Barrier::new(2));
        let g = gate.clone();
        let ag = admitted_gate.clone();
        let driver_engine = engine.clone();
        let driver = std::thread::spawn(move || {
            driver_engine.call(move || {
                g.wait();
                // Hold the close until the drain-bound jobs are admitted.
                ag.wait();
                enter_callback_frame();
                let deferred = inner.close_deferred(move || fr.store(true, Ordering::SeqCst));
                assert_eq!(deferred, Dispatch::Done(()));
                exit_callback_frame();
            })
        });
        gate.wait();
        // Two jobs admitted BEFORE close_deferred, so they drain through
        // drain_then_take_deferred: the FIRST cleanup panics; the second
        // must still run, and the deferred final must still execute.
        assert_eq!(
            engine.post_with_cleanup(
                || panic!("admitted-before-close job must drain, not run"),
                Some(|| panic!("boom (deliberate cleanup panic)")),
            ),
            Dispatch::Done(())
        );
        let sc = second_cleanup.clone();
        assert_eq!(
            engine.post_with_cleanup(
                || panic!("admitted-before-close job must drain, not run"),
                Some(move || sc.store(true, Ordering::SeqCst)),
            ),
            Dispatch::Done(())
        );
        admitted_gate.wait(); // now the outer job may close_deferred

        assert_eq!(driver.join().unwrap(), Dispatch::Done(()));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !final_ran.load(Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "the deferred destroy must survive a panicking cleanup"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            second_cleanup.load(Ordering::SeqCst),
            "the cleanup AFTER the panicking one must still run"
        );
    });
}

// (p) a panicking Drop on discarded closure state — or a `panic_any`
// payload whose Drop itself panics with ANOTHER panicking-Drop payload
// (arbitrarily deep) — must not unwind the engine and lose the deferred
// destroy. A drained job whose UNEXECUTED body captures a panicking-Drop
// value, plus a cleanup that panics with a NESTED panicking-Drop payload,
// is drained on the callback-origin path; the deferred final job must still
// run. The nested payload is what exercises drop_guarded's forget-on-poison
// arm: catching the cleanup panic yields a payload whose own drop panics
// again.
#[test]
fn panicking_drop_on_discarded_state_keeps_the_destroy() {
    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("boom (deliberate panicking Drop)");
        }
    }

    // Its own Drop panics WITH a further panicking-Drop payload — so the
    // payload the queue catches from this one ALSO panics when dropped.
    struct NestedPanicOnDrop;
    impl Drop for NestedPanicOnDrop {
        fn drop(&mut self) {
            std::panic::panic_any(PanicOnDrop);
        }
    }

    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-p"));
        let final_ran = Arc::new(AtomicBool::new(false));

        let inner = engine.clone();
        let fr = final_ran.clone();
        let gate = Arc::new(Barrier::new(2));
        let admitted_gate = Arc::new(Barrier::new(2));
        let g = gate.clone();
        let ag = admitted_gate.clone();
        let driver_engine = engine.clone();
        let driver = std::thread::spawn(move || {
            driver_engine.call(move || {
                g.wait();
                ag.wait();
                enter_callback_frame();
                let deferred = inner.close_deferred(move || fr.store(true, Ordering::SeqCst));
                assert_eq!(deferred, Dispatch::Done(()));
                exit_callback_frame();
            })
        });
        gate.wait();
        // A drained-bound job whose UNEXECUTED body captures a panicking-Drop
        // value — the body is never called, only dropped.
        let poison = PanicOnDrop;
        assert_eq!(
            engine.post(move || {
                let _hold = poison;
                unreachable!("drained job body must not run");
            }),
            Dispatch::Done(())
        );
        // A second drained job whose CLEANUP panics with a NESTED
        // panicking-Drop payload: catching it yields a payload whose own
        // drop panics again — drop_guarded must forget it, never re-drop.
        assert_eq!(
            engine.post_with_cleanup(
                || unreachable!("drained job must not run"),
                Some(|| std::panic::panic_any(NestedPanicOnDrop)),
            ),
            Dispatch::Done(())
        );
        admitted_gate.wait();

        assert_eq!(driver.join().unwrap(), Dispatch::Done(()));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !final_ran.load(Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "the deferred destroy must survive a panicking Drop on discarded state"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    });
}

// (k) callback-origin destroy with a populated queue: jobs admitted BEFORE
// the deferred close drain with Closing (never execute), the final job runs
// right after the outer job, and admission is rejected from the moment
// close_deferred returns. Fully deterministic: the outer job parks on a
// second barrier until BOTH queue-populating posts have been acknowledged
// with Dispatch::Done — the ack under the admission lock PROVES they were
// admitted before closing; only then does close_deferred run. A subsequent
// Closing is therefore always drain, never rejection. (Test (d) covers a
// real blocked `call()` waiter; the waiter here is call()'s exact internal
// composition — post_with_cleanup + completion flag — made observable.)
#[test]
fn callback_origin_destroy_with_populated_queue() {
    with_timeout(|| {
        let engine = Arc::new(EngineThread::spawn("t-k"));
        let log = Arc::new(Mutex::new(Vec::new()));

        let inner = engine.clone();
        let l_outer = log.clone();
        let l_final = log.clone();
        let gate = Arc::new(Barrier::new(2));
        let admitted_gate = Arc::new(Barrier::new(2));
        let g = gate.clone();
        let ag = admitted_gate.clone();
        let driver_engine = engine.clone();
        let driver = std::thread::spawn(move || {
            driver_engine.call(move || {
                g.wait();
                // Hold the close until the main thread has enqueued (and had
                // acknowledged) everything that must drain.
                ag.wait();
                enter_callback_frame();
                let deferred = inner.close_deferred(move || {
                    l_final.lock().unwrap().push("final");
                });
                assert_eq!(deferred, Dispatch::Done(()));
                exit_callback_frame();
                l_outer.lock().unwrap().push("outer");
            })
        });
        gate.wait();
        // Both posts land while the outer job is parked on admitted_gate —
        // closing CANNOT be set yet, and each Done ack proves admission.
        let drained = Arc::new(AtomicBool::new(false));
        let drained_flag = drained.clone();
        let admitted = engine.post_with_cleanup(
            || panic!("admitted-before-close job must drain, not run"),
            Some(move || drained_flag.store(true, Ordering::SeqCst)),
        );
        assert_eq!(admitted, Dispatch::Done(()), "admitted before closing");
        let waiter_drained = Arc::new(AtomicBool::new(false));
        let waiter_flag = waiter_drained.clone();
        let waiter_admitted = engine.post_with_cleanup(
            || panic!("admitted-before-close waiter must drain, not run"),
            Some(move || waiter_flag.store(true, Ordering::SeqCst)),
        );
        assert_eq!(
            waiter_admitted,
            Dispatch::Done(()),
            "waiter admitted before closing"
        );
        admitted_gate.wait(); // now the outer job may close

        assert_eq!(driver.join().unwrap(), Dispatch::Done(()));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if *log.lock().unwrap() == vec!["outer", "final"] {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "final never ran");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            drained.load(Ordering::SeqCst),
            "the admitted-before-close job must have been drained"
        );
        assert!(
            waiter_drained.load(Ordering::SeqCst),
            "the admitted-before-close waiter must have been drained (its \
             Closing completion is drain, not rejection)"
        );
        assert!(engine.post(|| {}).is_closing());
    });
}
