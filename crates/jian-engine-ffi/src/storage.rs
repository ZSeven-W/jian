use async_trait::async_trait;
use jian_core::action::services::{ServiceError, StorageBackend};
use serde_json::Value;
use std::path::PathBuf;

pub(crate) struct DirectoryStorage {
    root: PathBuf,
}

impl DirectoryStorage {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, key: &str) -> PathBuf {
        let mut encoded = String::with_capacity(key.len() * 2 + 5);
        for byte in key.as_bytes() {
            use std::fmt::Write;
            let _ = write!(&mut encoded, "{byte:02x}");
        }
        encoded.push_str(".json");
        self.root.join(encoded)
    }

    fn create_root(&self) -> Result<(), ServiceError> {
        std::fs::create_dir_all(&self.root)
            .map_err(|error| ServiceError(format!("storage directory: {error}")))
    }

    fn values(&self) -> Result<Vec<(String, Value)>, ServiceError> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(ServiceError(format!("storage directory: {error}"))),
        };
        let mut values = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| ServiceError(error.to_string()))?;
            if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes =
                std::fs::read(entry.path()).map_err(|error| ServiceError(error.to_string()))?;
            let envelope: StorageEnvelope = serde_json::from_slice(&bytes)
                .map_err(|error| ServiceError(format!("invalid storage entry: {error}")))?;
            values.push((envelope.key, envelope.value));
        }
        Ok(values)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StorageEnvelope {
    key: String,
    value: Value,
}

#[async_trait(?Send)]
impl StorageBackend for DirectoryStorage {
    async fn get(&self, key: &str) -> Result<Option<Value>, ServiceError> {
        let path = self.path(key);
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(ServiceError(format!("storage read: {error}"))),
        };
        let envelope: StorageEnvelope = serde_json::from_slice(&bytes)
            .map_err(|error| ServiceError(format!("invalid storage entry: {error}")))?;
        Ok((envelope.key == key).then_some(envelope.value))
    }

    async fn set(&self, key: &str, value: Value) -> Result<(), ServiceError> {
        self.create_root()?;
        let bytes = serde_json::to_vec(&StorageEnvelope {
            key: key.into(),
            value,
        })
        .map_err(|error| ServiceError(error.to_string()))?;
        std::fs::write(self.path(key), bytes)
            .map_err(|error| ServiceError(format!("storage write: {error}")))
    }

    async fn delete(&self, key: &str) -> Result<(), ServiceError> {
        match std::fs::remove_file(self.path(key)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ServiceError(format!("storage delete: {error}"))),
        }
    }

    async fn clear(&self) -> Result<(), ServiceError> {
        for entry in self.values()? {
            self.delete(&entry.0).await?;
        }
        Ok(())
    }

    async fn keys(&self) -> Result<Vec<String>, ServiceError> {
        let mut keys: Vec<_> = self.values()?.into_iter().map(|entry| entry.0).collect();
        keys.sort();
        Ok(keys)
    }
}
