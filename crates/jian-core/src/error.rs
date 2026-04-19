use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("schema error: {0}")]
    Schema(#[from] jian_ops_schema::OpsSchemaError),

    #[error("signal: target disposed")]
    SignalDisposed,

    #[error("state: unknown scope `{0}`")]
    UnknownScope(String),

    #[error("state: path `{0}` could not be resolved: {1}")]
    PathUnresolvable(String, &'static str),

    #[error("layout: taffy computation failed: {0}")]
    Layout(String),

    #[error("scene: node id not found: {0:?}")]
    NodeNotFound(String),
}

pub type CoreResult<T> = std::result::Result<T, CoreError>;
