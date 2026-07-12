use super::{DecodeError, RenderBackend};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use sha2::{Digest, Sha256};

pub fn data_url_key(source: &str) -> String {
    format!("data:sha256:{:x}", Sha256::digest(source.as_bytes()))
}

pub fn decode_data_url(source: &str) -> Result<Vec<u8>, String> {
    let (meta, payload) = source.split_once(',').ok_or("invalid data URL")?;
    if !meta.starts_with("data:") || !meta.ends_with(";base64") {
        return Err("image data URL must be base64 encoded".into());
    }
    let decoded_len = payload
        .len()
        .checked_mul(3)
        .ok_or("data URL is too large")?
        / 4;
    if decoded_len > 64 * 1024 * 1024 {
        return Err("image data URL exceeds 64 MiB".into());
    }
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|error| format!("invalid image data URL: {error}"))
}

pub fn canonical_url_key(source: &str, document_dir: &Path) -> Result<String, String> {
    if source.starts_with("data:") {
        return Ok(data_url_key(source));
    }
    if let Ok(url) = url::Url::parse(source) {
        return Ok(url.to_string());
    }
    let path = Path::new(source);
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        document_dir.join(path)
    };
    let normalized = absolute
        .canonicalize()
        .map_err(|error| format!("image path `{}`: {error}", absolute.display()))?;
    Ok(normalized.to_string_lossy().into_owned())
}

#[cfg(unix)]
pub fn read_confined_local(path: &Path, asset_root: &Path) -> Result<Vec<u8>, String> {
    use std::fs::OpenOptions;
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let root = asset_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| error.to_string())?;
    let opened = PathBuf::from(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&file)
    ))
    .canonicalize()
    .or_else(|_| path.canonicalize())
    .map_err(|error| error.to_string())?;
    if !opened.starts_with(&root) {
        return Err("image path escapes asset root".into());
    }
    let mut bytes = Vec::new();
    file.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("image file exceeds 64 MiB".into());
    }
    Ok(bytes)
}

#[async_trait::async_trait(?Send)]
pub trait ImageResolver {
    async fn resolve(&self, url: &str) -> Result<Vec<u8>, String>;
}

pub struct NullImageResolver;
#[async_trait::async_trait(?Send)]
impl ImageResolver for NullImageResolver {
    async fn resolve(&self, _url: &str) -> Result<Vec<u8>, String> {
        Err("image resolver unavailable".into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageState {
    Pending,
    Deferred,
    Bytes,
    Registered,
    Failed,
}

#[derive(Clone)]
struct Entry {
    state: ImageState,
    reservation: usize,
    bytes: Option<Vec<u8>>,
    inline: bool,
    backend_generation: u64,
    refs: usize,
}

pub struct ImageStore {
    entries: BTreeMap<String, Entry>,
    deferred: VecDeque<String>,
    reserved: usize,
    reservation_budget: usize,
    pinned: usize,
    pinned_budget: usize,
    backend_generation: u64,
    releases: Vec<String>,
}

impl Default for ImageStore {
    fn default() -> Self {
        Self::with_budgets(256 * 1024 * 1024, 128 * 1024 * 1024)
    }
}

impl ImageStore {
    pub fn with_budgets(reservation_budget: usize, pinned_budget: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            deferred: VecDeque::new(),
            reserved: 0,
            reservation_budget,
            pinned: 0,
            pinned_budget,
            backend_generation: 0,
            releases: Vec::new(),
        }
    }

    pub fn state(&self, key: &str) -> Option<ImageState> {
        self.entries.get(key).map(|entry| entry.state)
    }

    pub fn pending_keys(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.state == ImageState::Pending && !entry.inline)
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub fn revoke_network(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        for (key, entry) in &mut self.entries {
            if !entry.inline
                && matches!(
                    entry.state,
                    ImageState::Pending | ImageState::Deferred | ImageState::Bytes
                )
            {
                if entry.state == ImageState::Pending {
                    self.reserved = self.reserved.saturating_sub(entry.reservation);
                }
                entry.bytes = None;
                entry.state = ImageState::Failed;
                warnings.push(format!(
                    "image `{key}` denied after network capability revocation"
                ));
            }
        }
        self.deferred.retain(|key| {
            self.entries
                .get(key)
                .is_some_and(|entry| entry.state == ImageState::Deferred)
        });
        warnings
    }

    pub fn admit_resolver(&mut self, key: &str, reservation: usize) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.refs += 1;
            if entry.state == ImageState::Failed {
                entry.reservation = reservation;
                if self.reserved.saturating_add(reservation) <= self.reservation_budget {
                    self.reserved += reservation;
                    entry.state = ImageState::Pending;
                } else {
                    entry.state = ImageState::Deferred;
                    self.deferred.push_back(key.to_owned());
                }
            }
            return;
        }
        if reservation > 64 * 1024 * 1024 {
            self.entries.insert(
                key.to_owned(),
                Entry {
                    state: ImageState::Failed,
                    reservation: 0,
                    bytes: None,
                    inline: false,
                    backend_generation: self.backend_generation,
                    refs: 1,
                },
            );
            return;
        }
        let admitted = self.reserved.saturating_add(reservation) <= self.reservation_budget;
        if admitted {
            self.reserved += reservation;
        } else {
            self.deferred.push_back(key.to_owned());
        }
        self.entries.insert(
            key.to_owned(),
            Entry {
                state: if admitted {
                    ImageState::Pending
                } else {
                    ImageState::Deferred
                },
                reservation,
                bytes: None,
                inline: false,
                backend_generation: self.backend_generation,
                refs: 1,
            },
        );
    }

    pub fn begin_reload_ownership(&mut self) {
        for entry in self.entries.values_mut() {
            entry.refs = 0;
        }
    }

    pub fn finish_reload_ownership(&mut self) {
        let stale: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.refs == 0)
            .map(|(key, _)| key.clone())
            .collect();
        for key in stale {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.refs = 1;
            }
            self.release_ref(&key);
        }
    }

    pub fn resolve(&mut self, key: &str, bytes: Vec<u8>) {
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        if entry.state != ImageState::Pending {
            return;
        }
        self.reserved = self.reserved.saturating_sub(entry.reservation);
        if bytes.len() > 64 * 1024 * 1024 {
            entry.state = ImageState::Failed;
        } else {
            entry.bytes = Some(bytes);
            entry.state = ImageState::Bytes;
        }
        self.promote();
    }

    pub fn admit_inline(&mut self, key: &str, bytes: Vec<u8>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.refs += 1;
            if entry.state == ImageState::Failed {
                entry.bytes = Some(bytes);
                entry.reservation = entry.bytes.as_ref().map_or(0, Vec::len);
                entry.state = ImageState::Bytes;
            }
            return;
        }
        let reservation = bytes.len();
        let state = if self.reserved.saturating_add(reservation) <= self.reservation_budget {
            ImageState::Bytes
        } else {
            ImageState::Deferred
        };
        if state == ImageState::Deferred {
            self.deferred.push_back(key.to_owned());
        }
        self.entries.insert(
            key.to_owned(),
            Entry {
                state,
                reservation,
                bytes: Some(bytes),
                inline: true,
                backend_generation: self.backend_generation,
                refs: 1,
            },
        );
    }

    pub fn fail(&mut self, key: &str, _reason: &str) {
        if let Some(entry) = self.entries.get_mut(key) {
            if entry.state == ImageState::Pending {
                self.reserved = self.reserved.saturating_sub(entry.reservation);
            }
            entry.state = ImageState::Failed;
        }
        self.promote();
    }

    fn promote(&mut self) {
        let mut remaining = VecDeque::new();
        while let Some(key) = self.deferred.pop_front() {
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            if self.reserved.saturating_add(entry.reservation) <= self.reservation_budget {
                if entry.inline {
                    entry.state = ImageState::Bytes;
                } else {
                    self.reserved += entry.reservation;
                    entry.state = ImageState::Pending;
                }
            } else {
                remaining.push_back(key);
            }
        }
        self.deferred = remaining;
    }

    pub fn mark_registered(&mut self, key: &str, generation: u64) -> Result<(), DecodeError> {
        let entry = self
            .entries
            .get_mut(key)
            .ok_or_else(|| DecodeError("unknown image".into()))?;
        let bytes = entry.bytes.as_ref().map_or(0, Vec::len);
        if self.pinned.saturating_add(bytes) > self.pinned_budget {
            entry.state = ImageState::Failed;
            return Err(DecodeError("pinned image budget exceeded".into()));
        }
        self.pinned += bytes;
        entry.state = ImageState::Registered;
        entry.backend_generation = generation;
        Ok(())
    }

    pub fn backend_generation_changed(&mut self, generation: u64) {
        if generation == self.backend_generation {
            return;
        }
        self.backend_generation = generation;
        self.pinned = 0;
        self.releases.clear();
        for entry in self.entries.values_mut() {
            if entry.state == ImageState::Registered {
                entry.state = if entry.inline {
                    ImageState::Bytes
                } else {
                    ImageState::Pending
                };
                if !entry.inline {
                    self.reserved = self.reserved.saturating_add(entry.reservation);
                }
            }
        }
    }

    pub fn release_ref(&mut self, key: &str) {
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        entry.refs = entry.refs.saturating_sub(1);
        if entry.refs != 0 {
            return;
        }
        let entry = self.entries.remove(key).expect("entry still present");
        if entry.state == ImageState::Pending {
            self.reserved = self.reserved.saturating_sub(entry.reservation);
        }
        if entry.state == ImageState::Registered {
            self.pinned = self
                .pinned
                .saturating_sub(entry.bytes.as_ref().map_or(0, Vec::len));
            self.releases.push(key.to_owned());
        }
        self.deferred.retain(|candidate| candidate != key);
        self.promote();
    }

    /// True when the next `prepare_frame` will change store or backend
    /// state (pending releases, un-registered bytes, or budget-deferred
    /// entries awaiting promotion). Drives the caller's dirty marking.
    pub fn has_pending_work(&self) -> bool {
        !self.releases.is_empty()
            || self.entries.values().any(|e| {
                e.state == ImageState::Bytes
                    || (e.state == ImageState::Deferred
                        && self.reserved.saturating_add(e.reservation) <= self.reservation_budget)
            })
    }

    pub fn prepare_frame(
        &mut self,
        backend: &mut impl RenderBackend,
        generation: u64,
    ) -> Vec<String> {
        self.backend_generation_changed(generation);
        for key in std::mem::take(&mut self.releases) {
            backend.release_image(&key);
        }
        let pending: Vec<(String, Vec<u8>)> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.state == ImageState::Bytes)
            .map(|(key, entry)| (key.clone(), entry.bytes.clone().unwrap_or_default()))
            .collect();
        let mut warnings = Vec::new();
        for (key, bytes) in pending {
            match backend
                .register_image(&key, &bytes)
                .and_then(|_| self.mark_registered(&key, generation))
            {
                Ok(()) => {}
                Err(error) => {
                    self.fail(&key, &error.to_string());
                    warnings.push(format!("image `{key}`: {error}"));
                }
            }
        }
        warnings
    }
}
