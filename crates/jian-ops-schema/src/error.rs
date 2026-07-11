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
    ResponsiveBelowMinor {
        declared: String,
    },
    LogicModulesSkipped {
        reason: &'static str,
    },
    InvalidExpression {
        path: String,
        expr: String,
        reason: String,
    },
    ViewportWrite {
        path: String,
    },
    /// A legacy frame carrying an explicit widget role marker was
    /// promoted in-memory to a first-class widget node (the source
    /// file is NOT rewritten — see promote.rs).
    LegacyRolePromoted {
        path: String,
        from_role: String,
        to: &'static str,
    },
}

pub struct LoadResult<T> {
    pub value: T,
    pub warnings: Vec<LoadWarning>,
}
