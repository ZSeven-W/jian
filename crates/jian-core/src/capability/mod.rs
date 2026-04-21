//! Plan 6 — Capability Gate (full enforcement).
//!
//! Two-layer model (C14):
//!
//! 1. **CapabilityGate** — Runtime-level: every IO action consults
//!    `ctx.capabilities.check(needed, action_name)` before executing its
//!    side effect. Undeclared capabilities are denied and the check is
//!    written to the `AuditLog`.
//! 2. **PermissionBroker** — OS-level UX for permissions like camera /
//!    microphone / location. Trait is defined here; Null implementation
//!    returns `NotDetermined`; real brokers land in Plan 14+.
//!
//! This module replaces the Plan 4 placeholder that lived in
//! `action::capability`. The old import paths still work through a
//! re-export shim in `action::capability`.

pub mod audit;
pub mod broker;
pub mod gate;
pub mod map;

pub use audit::{AuditEntry, AuditLog, Verdict};
pub use broker::{NullPermissionBroker, PermissionBroker, PermissionStatus};
pub use gate::{
    from_schema_capability, AutomationLevel, Capability, CapabilityGate, DeclaredCapabilityGate,
    DummyCapabilityGate,
};
pub use map::required_capabilities;
