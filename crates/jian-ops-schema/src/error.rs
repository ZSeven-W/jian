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
