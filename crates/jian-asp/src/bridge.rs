//! ASP listener ↔ runtime bridge (Plan 18 ASP prod mode / C4
//! follow-up).
//!
//! Wires a multi-threaded transport listener (Unix socket / Named
//! Pipe) to the single-threaded `Runtime` the host's event loop
//! owns. The ASP listener thread parks on per-connection
//! [`crate::transport::Transport`] reads; for every parsed verb it
//! sends a [`DispatchRequest`] to the main thread via the
//! [`AspBridge`] handle and blocks on a oneshot [`mpsc::SyncSender`]
//! reply. The main thread drains pending requests once per event
//! loop tick (mirrors the existing MCP `Drain` integration in
//! `jian-host-desktop::run`), dispatches via
//! [`crate::verb_impls::dispatch_with_mode`], and writes the
//! outcome back through the reply channel.
//!
//! The bridge is synchronous-only on purpose:
//!
//! - The transport is read-line-then-dispatch-then-write-line, no
//!   pipelining → mpsc + sync reply matches that shape.
//! - The runtime (`jian_core::Runtime`) is `!Send`, so we cannot
//!   move it across threads. The bridge pattern keeps it parked on
//!   the winit thread and threads verbs through it.
//! - There's no tokio dep; std `mpsc` is enough and keeps `jian-asp`
//!   buildable in a `--no-default-features` workspace tier.

use std::sync::mpsc;

use crate::protocol::{OutcomePayload, Verb};
use crate::verb_impls::DispatchControl;

/// One verb dispatch ferried from a listener thread to the main
/// runtime thread. `reply` is a `sync_channel(0)` so the listener
/// blocks until the runtime hands back the outcome — the protocol's
/// strict request → response shape doesn't allow concurrent
/// in-flight verbs on a single connection.
#[derive(Debug)]
pub struct DispatchRequest {
    /// The parsed verb the listener wants the runtime to handle.
    pub verb: Verb,
    /// Reply slot. The runtime writes one [`DispatchResponse`] into
    /// it; the listener thread blocks on receive.
    pub reply: mpsc::SyncSender<DispatchResponse>,
}

/// The runtime's reply for a dispatch request. Wraps the payload
/// the listener writes back to the agent plus the control flag the
/// listener uses to decide whether to keep the session open.
#[derive(Debug)]
pub struct DispatchResponse {
    pub payload: OutcomePayload,
    pub control: DispatchControl,
}

/// Listener-side handle. `Clone` so a single bridge can fan-out to
/// multiple connection-handler threads if the listener accepts
/// concurrently. Sending on a closed bridge (the runtime thread
/// dropped its [`AspDrain`]) returns `None` from
/// [`AspBridge::dispatch_blocking`] — the listener treats that the
/// same as an EOF.
#[derive(Debug, Clone)]
pub struct AspBridge {
    tx: mpsc::Sender<DispatchRequest>,
}

/// Runtime-side handle. Drained once per event-loop tick in the
/// host's `about_to_wait`. Not `Clone`: the `Receiver` must stay on
/// one thread (the winit thread) so dispatch ordering is consistent
/// with the rest of the runtime's mutations.
#[derive(Debug)]
pub struct AspDrain {
    rx: mpsc::Receiver<DispatchRequest>,
}

/// Pair an [`AspBridge`] with its [`AspDrain`]. The host installs
/// the drain on its `DesktopHost` (or whatever event-loop owner it
/// uses) and hands the bridge to the listener thread.
pub fn channel() -> (AspBridge, AspDrain) {
    let (tx, rx) = mpsc::channel();
    (AspBridge { tx }, AspDrain { rx })
}

impl AspBridge {
    /// Send `verb` to the runtime thread and block until it replies.
    /// Returns `None` if the bridge or the reply channel was
    /// dropped before the round-trip completed — caller treats that
    /// as a transport-level disconnect.
    pub fn dispatch_blocking(&self, verb: Verb) -> Option<DispatchResponse> {
        // `sync_channel(0)` is rendezvous-style: the runtime's
        // `send` and the listener's `recv` synchronise without an
        // intermediate slot. Buffer-of-1 would also work; we pick 0
        // because the request flow is serial per session and we
        // want any future "queued more than one" misuse to surface
        // as a deadlock during testing rather than mask itself.
        let (reply_tx, reply_rx) = mpsc::sync_channel(0);
        let req = DispatchRequest {
            verb,
            reply: reply_tx,
        };
        self.tx.send(req).ok()?;
        reply_rx.recv().ok()
    }
}

impl AspDrain {
    /// Pop the next pending dispatch without blocking. Returns
    /// `None` when the queue is empty *or* every listener has
    /// dropped its [`AspBridge`] (queue closed). The host's event
    /// loop calls this in a `while let Some(...)` until exhausted.
    pub fn try_recv(&self) -> Option<DispatchRequest> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Verb;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn dispatch_round_trips_through_the_bridge() {
        let (bridge, drain) = channel();
        // Worker thread: sends one dispatch request, blocks on
        // reply, asserts on the result.
        let worker = thread::spawn(move || {
            let resp = bridge.dispatch_blocking(Verb::Exit).expect("response");
            assert!(resp.payload.ok);
            assert_eq!(resp.payload.verb, "exit");
        });

        // Main-thread side: spin until the request arrives, then
        // reply with a synthetic exit outcome.
        let req = loop {
            if let Some(r) = drain.try_recv() {
                break r;
            }
            thread::sleep(Duration::from_millis(1));
        };
        assert!(matches!(req.verb, Verb::Exit));
        let payload = OutcomePayload::ok("exit", None, "session ended");
        req.reply
            .send(DispatchResponse {
                payload,
                control: DispatchControl::Exit,
            })
            .expect("reply send");

        worker.join().expect("worker join");
    }

    #[test]
    fn dispatch_returns_none_when_drain_dropped() {
        let (bridge, drain) = channel();
        drop(drain);
        // No drain → send fails → method returns None rather than
        // panicking. This is the listener's signal to tear the
        // connection down.
        let resp = bridge.dispatch_blocking(Verb::Exit);
        assert!(resp.is_none());
    }

    #[test]
    fn try_recv_is_non_blocking_when_empty() {
        let (_bridge, drain) = channel();
        // No worker has sent yet — try_recv must not park.
        assert!(drain.try_recv().is_none());
    }
}
