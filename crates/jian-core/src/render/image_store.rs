use super::{DecodeError, RenderBackend};
use std::collections::{BTreeMap, VecDeque};

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

    pub fn admit_resolver(&mut self, key: &str, reservation: usize) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.refs += 1;
            return;
        }
        let admitted = reservation <= 64 * 1024 * 1024
            && self.reserved.saturating_add(reservation) <= self.reservation_budget;
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
        let state = if bytes.len() <= self.reservation_budget {
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
                reservation: bytes.len(),
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
        if entry.refs == 0 && entry.state == ImageState::Registered {
            self.releases.push(key.to_owned());
        }
    }

    /// True when the next `prepare_frame` will change store or backend
    /// state (pending releases, un-registered bytes, or budget-deferred
    /// entries awaiting promotion). Drives the caller's dirty marking.
    pub fn has_pending_work(&self) -> bool {
        !self.releases.is_empty()
            || self
                .entries
                .values()
                .any(|e| matches!(e.state, ImageState::Bytes | ImageState::Deferred))
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
