//! MCP transport — stdio rmcp server bridged into the in-process
//! `ActionSurface` via typed mpsc + oneshot channels.
//!
//! Gated behind `cfg(feature = "mcp")`. The default build drops the
//! whole module. See plan
//! `openpencil-docs/superpowers/plans/2026-04-25-jian-action-surface-mcp.md`.
//!
//! # Host integration (per-frame drain)
//!
//! Hosts call [`spawn_stdio_server`] once at startup and hold the
//! [`Drain`] for the program lifetime. Each frame, drain the queue
//! and dispatch each [`Request`] against the live surface — the
//! main-thread half of the bridge. Skeleton:
//!
//! ```ignore
//! let (mut drain, _handle) = mcp::spawn_stdio_server()?;
//! let mut surface = ActionSurface::from_document(&doc, &salt)
//!     .with_audit(audit_log.clone())
//!     .with_session_id("mcp");
//! // …per frame in the host loop…
//! while let Some(req) = drain.try_recv() {
//!     if !req.worker_listening() { continue; }
//!     match req {
//!         mcp::Request::List { opts, reply } => {
//!             let _ = reply.send(surface.list(opts));
//!         }
//!         mcp::Request::Execute { name, params, reply } => {
//!             let mut dispatcher = RuntimeDispatcher::new(&mut runtime);
//!             let outcome = surface.execute_with_gate(
//!                 &name,
//!                 params.as_ref(),
//!                 &mut dispatcher,
//!                 &state_gate,
//!             );
//!             let _ = reply.send(outcome);
//!         }
//!     }
//! }
//! ```
//!
//! # Audit + data-hiding (spec §8.1, §10)
//!
//! The audit row is recorded by the in-process surface, not by
//! this module — that's why the host construction above wires
//! `with_audit(log).with_session_id("mcp")` *before* draining.
//! Every MCP `execute_action` resolves to one
//! `ActionSurfaceAuditEntry` with that session id.
//!
//! The wire payload `JianToolServer::execute_action` returns is
//! exactly `{ ok: true } | { ok: false, error: { kind, reason } }`
//! — locked down by
//! `mcp::tools::tests::execute_outcome_wire_form_does_not_leak_internal_state`.
//! No source-node ids, no path strings, no internal state.

pub mod bridge;
pub mod server;
pub mod tools;

pub use bridge::{Bridge, Drain, Request};
pub use server::{spawn_stdio_server, ServerHandle};
pub use tools::{ExecuteRequest, JianToolServer, ListRequest, WirePageScope};
