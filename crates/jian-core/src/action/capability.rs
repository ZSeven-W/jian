//! Backwards-compatible shim. The real module lives in
//! `crate::capability` (Plan 6). Existing `action::capability` imports
//! continue to work via these re-exports.

pub use crate::capability::{
    AuditEntry, AuditLog, AutomationLevel, Capability, CapabilityGate, DeclaredCapabilityGate,
    DummyCapabilityGate, NullPermissionBroker, PermissionBroker, PermissionStatus, Verdict,
};
