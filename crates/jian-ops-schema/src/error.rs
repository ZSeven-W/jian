use thiserror::Error;

#[derive(Debug, Error)]
pub enum OpsSchemaError {
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("unsupported formatVersion: {found} (this crate supports up to {supported})")]
    UnsupportedFormatVersion {
        found: String,
        supported: &'static str,
    },

    #[error("schema validation failed: {0}")]
    Validation(String),
}

pub type OpsResult<T> = std::result::Result<T, OpsSchemaError>;

/// Non-fatal warnings produced while loading a document. Collected in `LoadResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadWarning {
    UnknownField {
        path: String,
        field: String,
    },
    FutureFormatVersion {
        found: String,
        supported_max: &'static str,
    },
    LogicModulesSkipped {
        reason: &'static str,
    },
    InvalidExpression {
        path: String,
        expr: String,
        reason: String,
    },
}

pub struct LoadResult<T> {
    pub value: T,
    pub warnings: Vec<LoadWarning>,
}
