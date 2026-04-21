//! Action execution errors and warnings.

use super::capability::Capability;
use crate::expression::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ActionError {
    #[error("unknown action `{0}`")]
    UnknownAction(String),

    #[error("action `{name}`: missing required field `{field}`")]
    MissingField {
        name: &'static str,
        field: &'static str,
    },

    #[error("action `{name}`: type error in `{field}`: {message}")]
    FieldType {
        name: &'static str,
        field: &'static str,
        message: String,
    },

    #[error("expression compile failed: {0}")]
    Expression(#[from] Diagnostic),

    #[error("capability denied: `{needed:?}` for action `{action}`")]
    CapabilityDenied {
        action: &'static str,
        needed: Capability,
    },

    #[error("action aborted")]
    Aborted,

    #[error("network: {0}")]
    Network(String),

    #[error("storage: {0}")]
    Storage(String),

    #[error("logic provider: {0}")]
    Logic(String),

    #[error("custom: {0}")]
    Custom(String),
}

pub type ActionResult = Result<(), ActionError>;
