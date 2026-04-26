//! Backwards-compatible shim. The real module lives in
//! `crate::capability` (Plan 6). Existing `action::capability` imports
//! continue to work via these re-exports.

#[cfg(feature = "dev-asp")]
pub use crate::capability::AutomationLevel;
pub use crate::capability::{
    AuditEntry, AuditLog, Capability, CapabilityGate, DeclaredCapabilityGate, DummyCapabilityGate,
    NullPermissionBroker, PermissionBroker, PermissionStatus, Verdict,
};
