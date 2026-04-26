//! `spawn_stdio_server` — wire `JianToolServer` to rmcp's stdio
//! transport on a tokio current-thread runtime.
//!
//! The host calls this once at startup. It returns:
//!
//! - [`Drain`]: the main-thread side of the bridge. Pump it once
//!   per frame (`RunApp::about_to_wait` per the plan) — for each
//!   pending [`Request`] check `worker_listening()`, then service
//!   the call against the live `ActionSurface`.
//! - [`ServerHandle`]: opaque RAII wrapper. Dropping it cancels
//!   the rmcp service (via `CancellationToken`) and joins the
//!   worker thread. The host typically holds it for the program
//!   lifetime; explicit `shutdown()` is available for tests.
//!
//! The worker thread is `std::thread::spawn`ed (not tokio::spawn)
//! because the runtime's signal scheduler is `!Send` — we keep the
//! main-thread tokio-free by giving the rmcp server its own
//! current-thread runtime in a worker.

use crate::mcp::bridge::{Bridge, Drain};
use crate::mcp::tools::JianToolServer;
use rmcp::transport::io::stdio;
use rmcp::ServiceExt;
use std::io;
use std::thread::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Spawn the rmcp stdio server on a worker thread. Returns the
/// main-thread [`Drain`] + an RAII [`ServerHandle`].
///
/// Errors propagate from `tokio::runtime::Builder::build()` only
/// (worker thread panics surface lazily through the JoinHandle —
/// `ServerHandle::shutdown()` exposes them).
pub fn spawn_stdio_server() -> io::Result<(Drain, ServerHandle)> {
    let (bridge, drain) = Bridge::new();
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    let join = std::thread::Builder::new()
        .name("jian-mcp-stdio".into())
        .spawn(move || run_worker(bridge, cancel_clone))?;

    Ok((
        drain,
        ServerHandle {
            cancel,
            join: Some(join),
        },
    ))
}

fn run_worker(bridge: Bridge, cancel: CancellationToken) {
    // current-thread is enough — rmcp's stdio transport is single-
    // pollable, and a multi-thread runtime would just add overhead
    // for a single client.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("jian-mcp: failed to build tokio runtime: {e}");
            return;
        }
    };
    let handler = JianToolServer::new(bridge);
    rt.block_on(async move {
        let transport = stdio();
        let service = match handler.serve_with_ct(transport, cancel.clone()).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("jian-mcp: failed to start service: {e}");
                return;
            }
        };
        // `waiting()` resolves when either the cancel token fires or
        // the peer disconnects. Either way the worker thread exits.
        if let Err(e) = service.waiting().await {
            eprintln!("jian-mcp: service stopped with error: {e}");
        }
    });
}

/// RAII wrapper around the worker thread + its cancel token.
/// Dropping the handle cancels the service and joins the thread.
pub struct ServerHandle {
    cancel: CancellationToken,
    join: Option<JoinHandle<()>>,
}

impl ServerHandle {
    /// Explicit shutdown — cancels the service and joins the worker.
    /// Returns the worker's JoinHandle result so the caller can
    /// inspect a panic. Idempotent.
    pub fn shutdown(mut self) -> std::thread::Result<()> {
        self.cancel.cancel();
        match self.join.take() {
            Some(h) => h.join(),
            None => Ok(()),
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(h) = self.join.take() {
            // Best-effort — surface a panic in tests via stderr
            // rather than re-panic in Drop.
            if let Err(e) = h.join() {
                eprintln!("jian-mcp: worker thread panicked: {e:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn handle_drop_cancels_worker() {
        // We can't drive stdin/stdout from a unit test, so just
        // verify the lifecycle: spawn, wait briefly, drop. Worker
        // thread should exit cleanly.
        let (_drain, handle) = spawn_stdio_server().expect("spawn worker");
        std::thread::sleep(Duration::from_millis(50));
        drop(handle);
        // If the worker didn't honour the cancel token the test
        // would hang indefinitely; cargo's per-test 60s timeout
        // would catch it.
    }

    #[test]
    fn explicit_shutdown_returns_join_result() {
        let (_drain, handle) = spawn_stdio_server().expect("spawn worker");
        std::thread::sleep(Duration::from_millis(50));
        let res = handle.shutdown();
        assert!(res.is_ok(), "worker thread panicked: {res:?}");
    }
}
