//! PermissionBroker — OS-level permission UX (the second half of C14).
//!
//! Where `CapabilityGate` enforces whether the app is *allowed* to ask,
//! `PermissionBroker` handles the OS-level "user, grant/deny this?"
//! dialog for capabilities that need runtime consent (camera, mic,
//! location, notifications, biometric, …).
//!
//! Only the trait + a Null implementation live in `jian-core`. Real
//! brokers ship with host crates (jian-host-desktop, jian-host-mobile)
//! in Plan 14+.

use super::gate::Capability;
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    NotDetermined,
    Granted,
    Denied,
    Restricted,
}

#[async_trait(?Send)]
pub trait PermissionBroker {
    async fn request(&self, cap: Capability) -> PermissionStatus;
    fn status(&self, cap: Capability) -> PermissionStatus;
}

/// Stub broker that always reports `NotDetermined`. Used in
/// `Runtime::new()` and in any host that hasn't wired up a real broker
/// yet.
pub struct NullPermissionBroker;

#[async_trait(?Send)]
impl PermissionBroker for NullPermissionBroker {
    async fn request(&self, _: Capability) -> PermissionStatus {
        PermissionStatus::NotDetermined
    }
    fn status(&self, _: Capability) -> PermissionStatus {
        PermissionStatus::NotDetermined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_status_is_not_determined() {
        let b = NullPermissionBroker;
        assert_eq!(
            b.status(Capability::Camera),
            PermissionStatus::NotDetermined
        );
    }

    #[test]
    fn null_request_is_not_determined() {
        let b = NullPermissionBroker;
        let r = futures::executor::block_on(b.request(Capability::Microphone));
        assert_eq!(r, PermissionStatus::NotDetermined);
    }
}
