//! Capability gate interface — placeholder. Plan 5 replaces with real enforcement.

use std::collections::HashSet;

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
}

pub trait CapabilityGate {
    fn check(&self, needed: Capability) -> bool;
}

pub struct DummyCapabilityGate;

impl CapabilityGate for DummyCapabilityGate {
    fn check(&self, _: Capability) -> bool {
        true
    }
}

pub struct DeclaredCapabilityGate {
    declared: HashSet<Capability>,
}

impl DeclaredCapabilityGate {
    pub fn from_iter(caps: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            declared: caps.into_iter().collect(),
        }
    }
}

impl CapabilityGate for DeclaredCapabilityGate {
    fn check(&self, needed: Capability) -> bool {
        self.declared.contains(&needed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy_allows_all() {
        assert!(DummyCapabilityGate.check(Capability::Network));
    }

    #[test]
    fn declared_filters() {
        let g = DeclaredCapabilityGate::from_iter([Capability::Network]);
        assert!(g.check(Capability::Network));
        assert!(!g.check(Capability::Storage));
    }
}
