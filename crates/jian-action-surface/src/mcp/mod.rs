//! MCP transport — stdio rmcp server bridged into the in-process
//! `ActionSurface` via typed mpsc + oneshot channels.
//!
//! Gated behind `cfg(feature = "mcp")`. The default build drops the
//! whole module. See plan
//! `openpencil-docs/superpowers/plans/2026-04-25-jian-action-surface-mcp.md`.

pub mod bridge;
