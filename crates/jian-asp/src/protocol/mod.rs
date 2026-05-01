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

#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod result;
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod verbs;

#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
mod request;

#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub use request::{Request, Response};
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub use result::{ActionRow, AuditEntry, DeltaEntry, DetailKind, NodeSummary, OutcomePayload};
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub use verbs::{InspectKind, NavMode, ScrollDir, SnapshotFormat, Verb};
