//! CapabilityGate trait + Dummy/Declared implementations.
//!
//! Plan 6 extends the Plan 4 stub:
//!
//! - `check` takes an `action` name so the audit log can record which
//!   action tripped the gate.
//! - `DeclaredCapabilityGate` carries an optional `AuditLog`; every check
//!   (allowed *or* denied) is recorded when present.
//! - `AutomationLevel` (graded capability) is ready for Plan 14 Tier-3
//!   logic but plumbed through here so the satisfaction logic is in one
//!   place.

use super::audit::{AuditEntry, AuditLog, Verdict};
use jian_ops_schema::app::Capability as SchemaCapability;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Instant;

/// Map a schema-declared capability to the runtime `Capability` enum.
/// The schema's enum has no `Automation` variant (Tier-3 capability
/// declarations live per-module), so this is a pure lift.
pub fn from_schema_capability(c: SchemaCapability) -> Capability {
    match c {
        SchemaCapability::Storage => Capability::Storage,
        SchemaCapability::Network => Capability::Network,
        SchemaCapability::Camera => Capability::Camera,
        SchemaCapability::Microphone => Capability::Microphone,
        SchemaCapability::Location => Capability::Location,
        SchemaCapability::Notifications => Capability::Notifications,
        SchemaCapability::Clipboard => Capability::Clipboard,
        SchemaCapability::Biometric => Capability::Biometric,
        SchemaCapability::FileSystem => Capability::FileSystem,
        SchemaCapability::Haptic => Capability::Haptic,
    }
}

/// Privilege level for graded capabilities (currently only `Automation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AutomationLevel {
    Observe,
    Act,
    Full,
}

impl AutomationLevel {
    /// `self` covers `other` iff it grants at least as much access.
    /// `Full` ⊇ `Act` ⊇ `Observe`.
    pub fn covers(self, other: AutomationLevel) -> bool {
        use AutomationLevel::*;
        matches!(
            (self, other),
            (Full, _) | (Act, Observe | Act) | (Observe, Observe)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Storage,
    Network,
    Camera,
    Microphone,
    Location,
    Notifications,
    Clipboard,
    Biometric,
    FileSystem,
    Haptic,
    /// Reserved for Tier-3 automation modules (Plan 14+).
    Automation(AutomationLevel),
}

pub trait CapabilityGate {
    /// Return `true` if the action is permitted to proceed. Called
    /// **before** the action's IO side effect runs.
    ///
    /// `action` is the registered action name ("fetch" / "storage_set" /
    /// …) so audit entries record which action tripped the gate.
    fn check(&self, needed: Capability, action: &'static str) -> bool;
}

/// Permissive gate — every check returns true. Used in `Runtime::new()`
/// for tests that don't want to declare capabilities.
pub struct DummyCapabilityGate;

impl CapabilityGate for DummyCapabilityGate {
    fn check(&self, _: Capability, _: &'static str) -> bool {
        true
    }
}

/// Production gate — a whitelist of declared capabilities. Any check
/// against a capability not in the set fails. When an `AuditLog` is
/// provided, every call (allowed or denied) is recorded.
pub struct DeclaredCapabilityGate {
    declared: HashSet<Capability>,
    audit: Option<Rc<AuditLog>>,
}

impl DeclaredCapabilityGate {
    pub fn new(caps: impl IntoIterator<Item = Capability>, audit: Option<Rc<AuditLog>>) -> Self {
        Self {
            declared: caps.into_iter().collect(),
            audit,
        }
    }

    /// Backwards-compatible constructor for pre-Plan-6 call sites.
    #[allow(clippy::should_implement_trait)]
    pub fn from_iter(caps: impl IntoIterator<Item = Capability>) -> Self {
        Self::new(caps, None)
    }

    /// Does the declared set satisfy `needed`? For simple capabilities
    /// this is plain set membership; for graded `Automation` it's
    /// level-aware (Full satisfies Observe, etc.).
    fn satisfies(&self, needed: Capability) -> bool {
        if let Capability::Automation(needed_level) = needed {
            return self.declared.iter().any(|d| match d {
                Capability::Automation(decl_level) => decl_level.covers(needed_level),
                _ => false,
            });
        }
        self.declared.contains(&needed)
    }
}

impl CapabilityGate for DeclaredCapabilityGate {
    fn check(&self, needed: Capability, action: &'static str) -> bool {
        let verdict = if self.satisfies(needed) {
            Verdict::Allowed
        } else {
            Verdict::Denied
        };
        if let Some(ref log) = self.audit {
            log.record(AuditEntry {
                at: Instant::now(),
                action,
                needed,
                verdict,
                node_id: None,
            });
        }
        verdict == Verdict::Allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy_allows_all() {
        assert!(DummyCapabilityGate.check(Capability::Network, "fetch"));
    }

    #[test]
    fn declared_filters_simple() {
        let g = DeclaredCapabilityGate::from_iter([Capability::Network]);
        assert!(g.check(Capability::Network, "fetch"));
        assert!(!g.check(Capability::Storage, "storage_set"));
    }

    #[test]
    fn automation_full_satisfies_lower() {
        let g = DeclaredCapabilityGate::from_iter([Capability::Automation(AutomationLevel::Full)]);
        assert!(g.check(Capability::Automation(AutomationLevel::Observe), "call"));
        assert!(g.check(Capability::Automation(AutomationLevel::Act), "call"));
        assert!(g.check(Capability::Automation(AutomationLevel::Full), "call"));
    }

    #[test]
    fn automation_act_satisfies_observe_not_full() {
        let g = DeclaredCapabilityGate::from_iter([Capability::Automation(AutomationLevel::Act)]);
        assert!(g.check(Capability::Automation(AutomationLevel::Observe), "call"));
        assert!(g.check(Capability::Automation(AutomationLevel::Act), "call"));
        assert!(!g.check(Capability::Automation(AutomationLevel::Full), "call"));
    }

    #[test]
    fn automation_observe_does_not_satisfy_act() {
        let g =
            DeclaredCapabilityGate::from_iter([Capability::Automation(AutomationLevel::Observe)]);
        assert!(!g.check(Capability::Automation(AutomationLevel::Act), "call"));
    }

    #[test]
    fn empty_denies_automation() {
        let g = DeclaredCapabilityGate::from_iter([]);
        assert!(!g.check(Capability::Automation(AutomationLevel::Observe), "call"));
    }

    #[test]
    fn declared_records_audit_when_provided() {
        let log = Rc::new(AuditLog::new(10));
        let g = DeclaredCapabilityGate::new([Capability::Network], Some(log.clone()));
        assert!(g.check(Capability::Network, "fetch"));
        assert!(!g.check(Capability::Storage, "storage_set"));
        let snap = log.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].action, "fetch");
        assert_eq!(snap[0].verdict, Verdict::Allowed);
        assert_eq!(snap[1].action, "storage_set");
        assert_eq!(snap[1].verdict, Verdict::Denied);
    }

    #[test]
    fn declared_without_audit_is_silent() {
        let g = DeclaredCapabilityGate::new([Capability::Network], None);
        assert!(g.check(Capability::Network, "fetch"));
    }
}
