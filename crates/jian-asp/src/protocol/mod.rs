//! ASP protocol surface (Plan 18 Task 1).
//!
//! The wire format is line-delimited JSON (NDJSON): each request OR
//! response is exactly one JSON object on its own line. The protocol
//! deliberately does NOT layer JSON-RPC 2.0 — `verb` is the
//! discriminator and `id` is the correlation key, so adding the
//! `jsonrpc` / `method` / `params` envelope would only inflate the
//! wire bytes.
//!
//! Phase 1 (this module) ships the **types** — `Request`, `Response`,
//! the `Verb` tagged enum, and the `OutcomePayload` semantic-result
//! shape. The verb-dispatch implementation, the transport layer, and
//! the session / handshake live in Plan 18 Task 3+ behind the
//! `dev-asp` feature gate.

#[cfg(feature = "dev-asp")]
pub mod result;
#[cfg(feature = "dev-asp")]
pub mod verbs;

#[cfg(feature = "dev-asp")]
mod request;

#[cfg(feature = "dev-asp")]
pub use request::{Request, Response};
#[cfg(feature = "dev-asp")]
pub use result::{AuditEntry, DeltaEntry, DetailKind, NodeSummary, OutcomePayload};
#[cfg(feature = "dev-asp")]
pub use verbs::{InspectKind, NavMode, ScrollDir, SnapshotFormat, Verb};
