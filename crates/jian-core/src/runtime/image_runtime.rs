use super::{ImageCompletion, ImageRequest, Runtime};
use crate::action::ExecOutcome;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

impl Runtime {
    pub fn prepare_frame(
        &mut self,
        backend: &mut impl crate::render::RenderBackend,
        backend_generation: u64,
    ) {
        let changed = self.image_store.has_pending_work();
        for warning in self.image_store.prepare_frame(backend, backend_generation) {
            self.load_warnings.push(warning);
        }
        if changed {
            self.mark_dirty();
        }
    }

    pub fn set_image_document_dir(&mut self, directory: impl Into<PathBuf>) {
        self.image_document_dir = directory.into();
        self.state.clear_image_keys();
        self.admit_document_images();
    }

    pub fn admit_document_images(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let mut found = Vec::new();
        for (_, node) in document.tree.nodes.iter() {
            if let jian_ops_schema::node::PenNode::Image(image) = &node.schema {
                found.push(image.src.as_ref().to_owned());
            }
            if let Ok(json) = serde_json::to_value(&node.schema) {
                if let Some(fills) = json.get("fill").cloned().and_then(|value| {
                    serde_json::from_value::<Vec<jian_ops_schema::style::PenFill>>(value).ok()
                }) {
                    for fill in fills {
                        if let jian_ops_schema::style::PenFill::Image(image) = fill {
                            found.push(image.url.as_ref().to_owned());
                        }
                    }
                }
            }
        }
        for source in found {
            if !source.starts_with("data:") {
                match self.image_resolver.admission(&source) {
                    Ok(Some(admission)) => {
                        self.state.set_image_key(&source, &admission.key);
                        self.image_request_sources
                            .insert(admission.key.clone(), admission.request_source);
                        if !admission.requires_network
                            || self.capabilities.check(
                                crate::action::Capability::Network,
                                "image_resolve",
                                self.now_ms,
                            )
                        {
                            self.image_store
                                .admit_resolver(&admission.key, 64 * 1024 * 1024);
                        } else {
                            self.image_store.admit_resolver(&admission.key, 0);
                            if self.image_store.state(&admission.key)
                                != Some(crate::render::image_store::ImageState::Registered)
                            {
                                self.image_store
                                    .fail(&admission.key, "network capability denied");
                                self.load_warnings.push(format!(
                                    "image `{}`: network capability denied",
                                    admission.key
                                ));
                            }
                        }
                        continue;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.load_warnings
                            .push(format!("image `{source}`: {error}"));
                        continue;
                    }
                }
            }
            let key = match crate::render::image_store::canonical_url_key(
                &source,
                &self.image_document_dir,
            ) {
                Ok(key) => key,
                Err(error) => {
                    self.load_warnings.push(error);
                    continue;
                }
            };
            self.state.set_image_key(&source, &key);
            if source.starts_with("data:") {
                match crate::render::image_store::decode_data_url(&source) {
                    Ok(bytes) => self.image_store.admit_inline(&key, bytes),
                    Err(error) => {
                        self.image_store.admit_resolver(&key, 0);
                        self.image_store.fail(&key, &error);
                        self.load_warnings.push(format!("image `{key}`: {error}"));
                    }
                }
            } else if source.starts_with("http://") || source.starts_with("https://") {
                if self.capabilities.check(
                    crate::action::Capability::Network,
                    "image_resolve",
                    self.now_ms,
                ) {
                    self.image_store.admit_resolver(&key, 64 * 1024 * 1024);
                } else {
                    self.image_store.admit_resolver(&key, 0);
                    if self.image_store.state(&key)
                        != Some(crate::render::image_store::ImageState::Registered)
                    {
                        self.image_store.fail(&key, "network capability denied");
                        self.load_warnings
                            .push(format!("image `{key}`: network capability denied"));
                    }
                }
            } else {
                let path = Path::new(&key);
                match crate::render::image_store::read_confined_local(
                    path,
                    &self.image_document_dir,
                ) {
                    Ok(bytes) => self.image_store.admit_inline(&key, bytes),
                    Err(error) => {
                        self.image_store.admit_resolver(&key, 0);
                        self.image_store.fail(&key, &error);
                        self.load_warnings.push(format!("image `{key}`: {error}"));
                    }
                }
            }
        }
    }

    pub(crate) fn dispatch_image_requests(&mut self) {
        for key in self.image_store.pending_keys() {
            if self.image_requests.contains_key(&key) {
                continue;
            }
            let resolver = self.image_resolver.clone();
            let completions = self.image_completions.clone();
            let request_key = key.clone();
            let owner_generation = Rc::new(Cell::new(self.document_generation));
            let completion_owner = owner_generation.clone();
            let request_source = self
                .image_request_sources
                .get(&key)
                .cloned()
                .unwrap_or_else(|| key.clone());
            let task_id = self.task_queue.spawn_future(
                async move {
                    let result = resolver.resolve(&request_source).await;
                    completions.borrow_mut().push(ImageCompletion {
                        key: request_key,
                        owner_generation: completion_owner,
                        result,
                    });
                    ExecOutcome {
                        result: Ok(()),
                        warnings: Vec::new(),
                    }
                },
                self.document_generation,
                Some(format!("image:{key}")),
            );
            self.image_requests.insert(
                key,
                ImageRequest {
                    task_id,
                    owner_generation,
                },
            );
        }
    }
}
