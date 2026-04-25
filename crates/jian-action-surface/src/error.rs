//! Four-tier error taxonomy from `2026-04-24-ai-action-surface.md` §5.3.
//!
//! Reasons are **fixed enums** — no dynamic detail strings, no leaked
//! state paths or handler names, no internal error messages. Detailed
//! context only goes to the `AuditLog` (spec §8). Runtime callers
//! produce `ExecuteError`; both `kind` and `reason` round-trip
//! through serde so the JSON wire form matches §5.3 exactly.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationReason {
    MissingRequired,
    TypeMismatch,
    OutOfRange,
    SchemaViolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotAvailableReason {
    UnknownAction,
    StaticHidden,
    StateGated,
    ConfirmGated,
    RateLimited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusyReason {
    AlreadyRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionReason {
    CapabilityDenied,
    HandlerError,
    Timeout,
    Unknown,
}

/// Fully-typed execute-side error. Always one of four kinds with a
/// matching reason enum — no free-form strings, no PII.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum ExecuteError {
    ValidationFailed { reason: ValidationReason },
    NotAvailable { reason: NotAvailableReason },
    Busy { reason: BusyReason },
    ExecutionFailed { reason: ExecutionReason },
}

impl ExecuteError {
    /// Convenience constructors so handler code reads as
    /// `ExecuteError::not_available_static_hidden()` rather than
    /// the wordy struct form.
    pub fn unknown_action() -> Self {
        Self::NotAvailable {
            reason: NotAvailableReason::UnknownAction,
        }
    }
    pub fn static_hidden() -> Self {
        Self::NotAvailable {
            reason: NotAvailableReason::StaticHidden,
        }
    }
    pub fn state_gated() -> Self {
        Self::NotAvailable {
            reason: NotAvailableReason::StateGated,
        }
    }
    pub fn confirm_gated() -> Self {
        Self::NotAvailable {
            reason: NotAvailableReason::ConfirmGated,
        }
    }
    pub fn rate_limited() -> Self {
        Self::NotAvailable {
            reason: NotAvailableReason::RateLimited,
        }
    }
    pub fn already_running() -> Self {
        Self::Busy {
            reason: BusyReason::AlreadyRunning,
        }
    }
    pub fn capability_denied() -> Self {
        Self::ExecutionFailed {
            reason: ExecutionReason::CapabilityDenied,
        }
    }
    pub fn handler_error() -> Self {
        Self::ExecutionFailed {
            reason: ExecutionReason::HandlerError,
        }
    }
    pub fn missing_required() -> Self {
        Self::ValidationFailed {
            reason: ValidationReason::MissingRequired,
        }
    }
    pub fn type_mismatch() -> Self {
        Self::ValidationFailed {
            reason: ValidationReason::TypeMismatch,
        }
    }
    pub fn schema_violation() -> Self {
        Self::ValidationFailed {
            reason: ValidationReason::SchemaViolation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_error_serialises_to_spec_shape() {
        // `{ kind: "NotAvailable", reason: "state_gated" }` per §5.3.
        let err = ExecuteError::state_gated();
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["kind"], "NotAvailable");
        assert_eq!(v["reason"], "state_gated");
    }

    #[test]
    fn execute_error_round_trip() {
        let cases = [
            ExecuteError::unknown_action(),
            ExecuteError::confirm_gated(),
            ExecuteError::rate_limited(),
            ExecuteError::already_running(),
            ExecuteError::capability_denied(),
            ExecuteError::handler_error(),
            ExecuteError::missing_required(),
            ExecuteError::type_mismatch(),
        ];
        for original in cases {
            let json = serde_json::to_string(&original).unwrap();
            let parsed: ExecuteError = serde_json::from_str(&json).unwrap();
            assert_eq!(original, parsed);
        }
    }
}
