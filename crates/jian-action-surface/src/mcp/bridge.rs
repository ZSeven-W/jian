//! Bridge between the MCP worker thread (tokio current-thread)
//! and the runtime's main thread.
//!
//! The worker can't touch `ActionSurface` / `Runtime` directly — the
//! signal scheduler and gesture pipeline are `!Send`, and the runtime
//! lives on the main thread for good reason. Instead each rmcp tool
//! handler builds a typed [`Request`], sends it through an
//! [`mpsc::UnboundedSender`], and `await`s the matching
//! [`oneshot::Receiver`] for the reply.
//!
//! The main thread drains the queue once per frame in
//! `RunApp::about_to_wait`. For each [`Request`] it:
//!
//! 1. Checks `reply.is_closed()` — a client that disconnected
//!    mid-call doesn't get the side effect; we drop the request and
//!    keep the surface clean. Spec §10 data-hiding doesn't want
//!    silent state mutations for callers that won't see the result.
//! 2. Calls `ActionSurface::list_with_gate` / `execute_with_gate`
//!    against the payload — the gate-aware variants honour live
//!    `bindings.visible` / `bindings.disabled` and prevent
//!    dynamically-hidden actions from leaking onto the wire (spec
//!    §10). Captures the typed outcome, sends it through the
//!    oneshot. The send may fail if the worker dropped during the
//!    main-thread call — same handling as #1, log + continue.
//!
//! No rmcp types here yet — that's MCP plan Task 2 (rmcp tool defs).
//! This file is pure plumbing so it can be unit-tested without
//! spinning up a full tokio runtime + rmcp server.

use crate::{ExecuteOutcome, ListOptions, ListResponse};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// One MCP RPC the worker wants the main thread to service.
///
/// Each variant carries its own typed `oneshot::Sender` so the worker
/// gets back the exact response shape rmcp will serialise — `list`
/// returns the [`ListResponse`] payload, `execute` returns the
/// `{ ok: true } | { ok: false, error: … }` taxonomy in
/// [`ExecuteOutcome`].
#[derive(Debug)]
pub enum Request {
    List {
        opts: ListOptions,
        reply: oneshot::Sender<ListResponse>,
    },
    Execute {
        name: String,
        params: Option<Value>,
        reply: oneshot::Sender<ExecuteOutcome>,
    },
}

impl Request {
    /// Whether the worker is still listening for the reply. The
    /// drain loop should skip the surface call when this returns
    /// `false` so a disconnected client doesn't trigger state
    /// mutations no one will read.
    pub fn worker_listening(&self) -> bool {
        match self {
            Request::List { reply, .. } => !reply.is_closed(),
            Request::Execute { reply, .. } => !reply.is_closed(),
        }
    }
}

/// Worker-side handle. Each rmcp tool builds a `Bridge::send_*` call,
/// awaits the returned receiver. Cloning the `Sender` is fine — rmcp
/// hands the bridge into multiple async tasks.
#[derive(Debug, Clone)]
pub struct Bridge {
    tx: mpsc::UnboundedSender<Request>,
}

impl Bridge {
    /// Build a fresh bridge pair: worker holds the [`Bridge`], main
    /// thread holds the [`Drain`]. The pair is a single-producer
    /// /single-consumer logical channel even though `mpsc` allows
    /// many senders — rmcp's tool dispatch fans into one runtime.
    pub fn new() -> (Self, Drain) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, Drain { rx })
    }

    /// Build a `tools/list` request and `await` the response. Returns
    /// `None` if the main-thread end has been dropped — the caller
    /// surfaces that as a transport-level error.
    pub async fn list(&self, opts: ListOptions) -> Option<ListResponse> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Request::List { opts, reply }).ok()?;
        rx.await.ok()
    }

    /// Build a `tools/call` request and `await` the response. Same
    /// `None` semantics on dropped consumer.
    pub async fn execute(
        &self,
        name: impl Into<String>,
        params: Option<Value>,
    ) -> Option<ExecuteOutcome> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Request::Execute {
                name: name.into(),
                params,
                reply,
            })
            .ok()?;
        rx.await.ok()
    }
}

/// Main-thread side of the bridge. Held by the host's run loop;
/// drained once per frame in `about_to_wait`.
#[derive(Debug)]
pub struct Drain {
    rx: mpsc::UnboundedReceiver<Request>,
}

impl Drain {
    /// Pop the next pending request without blocking. Returns `None`
    /// when the queue is empty *or* every worker has dropped its
    /// `Bridge` (queue closed). Hosts call this in a loop until
    /// `None` to drain the frame.
    pub fn try_recv(&mut self) -> Option<Request> {
        self.rx.try_recv().ok()
    }

    /// Convenience: drain everything pending into a `Vec`. Most hosts
    /// prefer the loop-`try_recv` shape above so they can react per
    /// request, but tests and integration shims sometimes want the
    /// batched form.
    pub fn drain(&mut self) -> Vec<Request> {
        let mut out = Vec::new();
        while let Some(req) = self.try_recv() {
            out.push(req);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExecuteError;
    use std::time::Duration;
    use tokio::time::timeout;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("tokio current-thread rt")
    }

    #[test]
    fn list_request_round_trips_typed_response() {
        let rt = rt();
        rt.block_on(async {
            let (bridge, mut drain) = Bridge::new();
            let bridge2 = bridge.clone();

            let task = tokio::spawn(async move { bridge2.list(ListOptions::default()).await });

            // Drain on "main thread", reply with a ListResponse.
            let req = loop {
                if let Some(req) = drain.try_recv() {
                    break req;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            };
            match req {
                Request::List { reply, .. } => {
                    let _ = reply.send(ListResponse {
                        actions: Vec::new(),
                        total: 0,
                        page: None,
                    });
                }
                Request::Execute { .. } => panic!("expected List, got Execute"),
            }

            let resp = timeout(Duration::from_millis(500), task)
                .await
                .expect("worker resolves before timeout")
                .expect("task didn't panic")
                .expect("bridge returned Some");
            assert!(resp.actions.is_empty());
        });
    }

    #[test]
    fn execute_request_round_trips_typed_outcome() {
        let rt = rt();
        rt.block_on(async {
            let (bridge, mut drain) = Bridge::new();

            let task =
                tokio::spawn(async move { bridge.execute("home.does_not_exist", None).await });

            let req = loop {
                if let Some(req) = drain.try_recv() {
                    break req;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            };
            match req {
                Request::Execute { name, reply, .. } => {
                    assert_eq!(name, "home.does_not_exist");
                    let _ = reply.send(ExecuteOutcome::Err(ExecuteError::unknown_action()));
                }
                Request::List { .. } => panic!("expected Execute, got List"),
            }

            let resp = timeout(Duration::from_millis(500), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(matches!(resp, ExecuteOutcome::Err(_)));
        });
    }

    #[test]
    fn dropped_receiver_marks_request_unlistening() {
        // Simulates a client that disconnected after enqueueing a
        // request but before the main thread got to drain it. The
        // worker's `oneshot::Receiver` was dropped, so the request's
        // `Sender` is now closed — the host must skip the surface
        // call to honour spec §10 data-hiding.
        let (tx, mut rx) = mpsc::unbounded_channel::<Request>();
        let (reply, worker_rx) = oneshot::channel::<ExecuteOutcome>();
        tx.send(Request::Execute {
            name: "home.x".into(),
            params: None,
            reply,
        })
        .expect("enqueue request");
        // Client drops the receiver — equivalent to disconnect.
        drop(worker_rx);
        let req = rx.try_recv().expect("request still in queue");
        assert!(
            !req.worker_listening(),
            "dropping the oneshot receiver should close its sender"
        );
    }

    #[test]
    fn drain_empty_returns_none() {
        let (_bridge, mut drain) = Bridge::new();
        assert!(drain.try_recv().is_none());
        assert!(drain.drain().is_empty());
    }
}
