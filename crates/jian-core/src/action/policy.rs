//! Action policy (R3): a guard consulted BEFORE each parsed action's
//! capability gate or side effect, so a Preview runtime can allowlist
//! exactly the safe action vocabulary and reject everything else with a
//! structured, non-fatal diagnostic.
//!
//! Policy runs inside [`ActionChain::run_serial`]: a rejection never
//! aborts the chain — it emits `ActionError::PolicyRejected`, runs the
//! action's optional rejection branch, and the LATER SAFE SIBLINGS still
//! execute. Unknown/invalid syntax stays a parse error (the registry
//! never constructs the action); policy is only about the NAME.

use super::error::ActionError;
use std::collections::BTreeSet;

/// The policy guard an ActionContext consults per action.
pub trait ActionPolicy {
    /// `Ok(())` when `action` may execute; `Err(ActionError::
    /// PolicyRejected)` when the policy forbids it.
    fn check(&self, action: &str) -> Result<(), ActionError>;
}

/// An exact-name allowlist policy.
pub struct AllowListPolicy {
    allowed: BTreeSet<String>,
}

impl AllowListPolicy {
    /// Build from any ordered collection of action names.
    pub fn new<I: IntoIterator<Item = String>>(allowed: I) -> Self {
        Self {
            allowed: allowed.into_iter().collect(),
        }
    }

    pub fn contains(&self, action: &str) -> bool {
        self.allowed.contains(action)
    }
}

impl ActionPolicy for AllowListPolicy {
    fn check(&self, action: &str) -> Result<(), ActionError> {
        if self.allowed.contains(action) {
            Ok(())
        } else {
            Err(ActionError::PolicyRejected {
                action: action.to_owned(),
            })
        }
    }
}

/// The FIXED Preview allowlist (R3): the safe authorable vocabulary.
/// `fetch`, every WebSocket action, storage wipe, `notify`, `paste`,
/// `race`, and `call` are rejected by Preview policy even when their
/// capabilities are declared. R5 adds `dismiss_keyboard`'s siblings to
/// both the registry and this list; `animate` has an authorable
/// descriptor (R5) and receives its runtime factory in R7.
pub struct PreviewActionPolicy;

impl PreviewActionPolicy {
    pub const ALLOWED: &[&str] = &[
        "set",
        "toggle",
        "delete",
        "reset",
        "if",
        "delay",
        "parallel",
        "push",
        "replace",
        "pop",
        "show",
        "hide",
        "toggle_visibility",
        "focus",
        "blur",
        "scroll_to",
        "animate",
        "toast",
        "alert",
        "confirm",
        "open_url",
        "copy",
        "share",
        "haptic",
        "dismiss_keyboard",
    ];

    /// The canonical [`AllowListPolicy`] over [`Self::ALLOWED`].
    pub fn policy() -> AllowListPolicy {
        AllowListPolicy::new(Self::ALLOWED.iter().copied().map(str::to_owned))
    }
}
