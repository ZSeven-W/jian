use super::document_prepare::{
    copy_layout_scopes, normalized_route_values, prepare_document, route_values,
};
use super::{Runtime, AUDIT_LOG_CAPACITY};
use crate::action::services::RouteState;
use crate::action::ExecOutcome;
use crate::capability::{from_schema_capability, AuditLog, DeclaredCapabilityGate};
use crate::document::loader;
use crate::error::CoreResult;
use crate::gesture::collect_focus_chain;
use crate::signal::scheduler::Scheduler;
use crate::state::StateGraph;
use jian_ops_schema::{document::PenDocument, load_str};
use std::collections::BTreeMap;
use std::rc::Rc;

impl Runtime {
    pub fn load_str(&mut self, src: &str) -> CoreResult<()> {
        let schema = load_str(src)?.value;
        self.replace_document(schema)
    }

    /// Atomically hot-reload a document together with the layout/spatial data
    /// the host will immediately consume. All fallible parsing, loading,
    /// measurement, and constraint work completes against detached state
    /// before the live document, tasks, or image ownership are changed.
    pub fn load_str_and_relayout(&mut self, src: &str) -> CoreResult<()> {
        let schema = load_str(src)?.value;
        let preferred_path = self.active_screen_path.clone();
        self.replace_document_for_path_mode(schema, preferred_path.as_deref(), None, true, true)
    }

    /// Swap the runtime's document tree for `schema`, reusing the
    /// existing StateGraph + services. Used by `jian dev` hot-reload
    /// so app state (e.g. `$state.count`) survives a `.op` edit.
    ///
    /// Refreshes the capability gate from the new schema's
    /// `app.capabilities` (additions become available immediately,
    /// removals start denying), and reuses an existing `AuditLog` so
    /// rolling history is preserved across reloads.
    ///
    /// State seeding uses `SeedMode::PreserveExisting` — keys that
    /// already hold a value keep that value; only newly-introduced
    /// keys get their schema default.
    pub fn replace_document(&mut self, schema: PenDocument) -> CoreResult<()> {
        let preferred_path = self.active_screen_path.clone();
        self.replace_document_for_path_mode(schema, preferred_path.as_deref(), None, true, false)
    }

    pub(crate) fn replace_document_for_path(
        &mut self,
        schema: PenDocument,
        preferred_path: Option<&str>,
        route: &RouteState,
    ) -> CoreResult<()> {
        self.replace_document_for_path_mode(schema, preferred_path, Some(route), false, false)
    }

    pub(crate) fn replace_document_for_path_and_relayout(
        &mut self,
        schema: PenDocument,
        preferred_path: Option<&str>,
        route: &RouteState,
    ) -> CoreResult<()> {
        self.replace_document_for_path_mode(schema, preferred_path, Some(route), false, true)
    }

    pub(super) fn replace_document_for_path_mode(
        &mut self,
        mut schema: PenDocument,
        preferred_path: Option<&str>,
        candidate_route: Option<&RouteState>,
        conform_reload: bool,
        install_layout: bool,
    ) -> CoreResult<()> {
        let route_snapshot = conform_reload.then(|| self.nav.current());
        let reload_declaration_schema = schema.clone();
        let prepared = prepare_document(
            schema,
            (self.viewport.size.width, self.viewport.size.height),
            preferred_path,
        );
        let declaration_source = &reload_declaration_schema;
        let page_declarations: BTreeMap<String, jian_ops_schema::state::StateSchema> =
            declaration_source
                .pages
                .as_ref()
                .into_iter()
                .flatten()
                .filter_map(|page| page.state.clone().map(|state| (page.id.clone(), state)))
                .collect();
        fn collect_self_declarations(
            value: &serde_json::Value,
            page_key: &str,
            output: &mut BTreeMap<(String, String), jian_ops_schema::state::StateSchema>,
        ) {
            match value {
                serde_json::Value::Object(map) => {
                    if map.get("type").and_then(|value| value.as_str()).is_some() {
                        if let (Some(id), Some(state)) =
                            (map.get("id").and_then(|v| v.as_str()), map.get("state"))
                        {
                            if let Ok(schema) = serde_json::from_value(state.clone()) {
                                output.insert((page_key.to_owned(), id.to_owned()), schema);
                            }
                        }
                    }
                    if let Some(children) = map.get("children") {
                        collect_self_declarations(children, page_key, output);
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        collect_self_declarations(value, page_key, output);
                    }
                }
                _ => {}
            }
        }
        let mut self_declarations = BTreeMap::new();
        if let Ok(children) = serde_json::to_value(&declaration_source.children) {
            collect_self_declarations(&children, "", &mut self_declarations);
        }
        if let Some(pages) = declaration_source.pages.as_ref() {
            for page in pages {
                if let Ok(children) = serde_json::to_value(&page.children) {
                    collect_self_declarations(&children, &page.id, &mut self_declarations);
                }
            }
        }
        schema = prepared.mounted;
        let responsive = schema.is_responsive();
        let valid_paths: Vec<String> = schema.routes.as_ref().map_or_else(Vec::new, |routes| {
            std::iter::once(routes.entry.clone())
                .chain(
                    routes
                        .routes
                        .keys()
                        .filter(|path| *path != &routes.entry)
                        .cloned(),
                )
                .collect()
        });
        let declared_state = schema.state.clone().unwrap_or_default();
        let staged_defaults: BTreeMap<String, serde_json::Value> = declared_state
            .iter()
            .map(|(key, entry)| {
                (
                    key.clone(),
                    entry.default.clone().unwrap_or(serde_json::Value::Null),
                )
            })
            .collect();
        // Rebuild the capability gate from the new schema. Reuse the
        // existing AuditLog so the rolling history isn't truncated on
        // every save. If the original Runtime was constructed via
        // `Runtime::new` (no audit), allocate one now so newly
        // declared capabilities can record entries.
        let declared = schema
            .app
            .as_ref()
            .and_then(|a| a.capabilities.as_ref())
            .map(|list| {
                list.iter()
                    .copied()
                    .map(from_schema_capability)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let audit = self
            .audit
            .clone()
            .unwrap_or_else(|| Rc::new(AuditLog::new(AUDIT_LOG_CAPACITY)));
        let storage_allowed = declared.contains(&crate::action::Capability::Storage);
        let capabilities = Rc::new(DeclaredCapabilityGate::new(declared, Some(audit.clone())));

        // Loader seeding is fallible and mutating. Build against a detached
        // graph so failure cannot alter responsive mode or any live scope.
        let staged_state = Rc::new(StateGraph::new(Rc::new(Scheduler::new())));
        staged_state.set_responsive(responsive);
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_loader) {
            return Err(crate::error::CoreError::Layout(
                "injected loader failure".into(),
            ));
        }
        let doc = loader::build_with(schema, &staged_state, loader::SeedMode::Initial)?;
        let staged_vars = staged_state.vars_snapshot();
        let page_key = if responsive {
            prepared.selected_page_id.as_deref().unwrap_or_default()
        } else {
            ""
        };
        // Preview every registered cancellation compensation against a
        // detached copy. The one fallible geometry build then sees exactly
        // the state that task cancellation will commit, while failed reloads
        // leave live futures and their authored loading flags untouched.
        let cancellation_state = Rc::new(StateGraph::new(Rc::new(Scheduler::new())));
        cancellation_state.set_responsive(responsive);
        copy_layout_scopes(&self.state, &cancellation_state, storage_allowed);
        if conform_reload {
            let retained = self.reload_retained_task_ids();
            self.task_queue
                .preview_cancel_compensations_except(&retained, &cancellation_state);
        }
        let live_state = cancellation_state.app_snapshot();
        let (merged_state, mut conformance_warnings) =
            crate::state::conformance::merge_scope(&live_state, &staged_defaults, &declared_state);
        let mut page_merges = Vec::new();
        let mut self_merges = Vec::new();
        if conform_reload {
            // Union of newly declared keys and RETAINED live keys: a page
            // whose `state` declaration disappeared must still be merged —
            // against an empty declaration, which prunes its stale fields.
            let empty_page_schema = jian_ops_schema::state::StateSchema::default();
            let mut page_keys: Vec<String> = page_declarations.keys().cloned().collect();
            for key in self.state.page_keys() {
                if !page_declarations.contains_key(&key) {
                    page_keys.push(key);
                }
            }
            for page_key in &page_keys {
                let page_schema = page_declarations
                    .get(page_key)
                    .unwrap_or(&empty_page_schema);
                let staged: BTreeMap<String, serde_json::Value> = page_schema
                    .iter()
                    .map(|(name, entry)| {
                        (
                            name.clone(),
                            entry.default.clone().unwrap_or(serde_json::Value::Null),
                        )
                    })
                    .collect();
                let (merged, warnings) = crate::state::conformance::merge_scope(
                    &cancellation_state.page_snapshot(page_key),
                    &staged,
                    page_schema,
                );
                conformance_warnings.extend(
                    warnings
                        .into_iter()
                        .map(|warning| format!("$page[{page_key}]: {warning}")),
                );
                page_merges.push((page_key.clone(), merged));
            }
            let empty_self_schema = jian_ops_schema::state::StateSchema::default();
            let mut self_keys: Vec<(String, String)> = self_declarations.keys().cloned().collect();
            for key in self.state.self_keys() {
                if !self_declarations.contains_key(&key) {
                    self_keys.push(key);
                }
            }
            for (page_key, node_id) in &self_keys {
                let declared = self_declarations
                    .get(&(page_key.clone(), node_id.clone()))
                    .unwrap_or(&empty_self_schema);
                let staged: BTreeMap<String, serde_json::Value> = declared
                    .iter()
                    .map(|(name, entry)| {
                        (
                            name.clone(),
                            entry.default.clone().unwrap_or(serde_json::Value::Null),
                        )
                    })
                    .collect();
                let (merged, warnings) = crate::state::conformance::merge_scope(
                    &cancellation_state.self_snapshot(page_key, node_id),
                    &staged,
                    declared,
                );
                conformance_warnings.extend(
                    warnings
                        .into_iter()
                        .map(|warning| format!("$self[{page_key}/{node_id}]: {warning}")),
                );
                self_merges.push((page_key.clone(), node_id.clone(), merged));
            }
        }

        let committed_route = candidate_route.map_or_else(
            || {
                route_snapshot.as_ref().map_or_else(
                    || self.state.route_snapshot(),
                    |route| normalized_route_values(route, &valid_paths),
                )
            },
            route_values,
        );
        if conform_reload {
            copy_layout_scopes(&cancellation_state, &staged_state, storage_allowed);
            staged_state.replace_app(&merged_state);
            staged_state.replace_vars(&staged_vars);
            for (page_key, values) in &page_merges {
                staged_state.replace_page(page_key, values);
            }
            for (page_key, node_id, values) in &self_merges {
                staged_state.replace_self(page_key, node_id, values);
            }
            staged_state.replace_route(&committed_route);
        } else {
            copy_layout_scopes(&self.state, &staged_state, storage_allowed);
            staged_state.replace_route(&committed_route);
        }
        let staged_geometry = if install_layout {
            Some(self.stage_document_geometry(
                &doc,
                &staged_state,
                page_key,
                (self.viewport.size.width, self.viewport.size.height),
            )?)
        } else {
            None
        };

        if conform_reload {
            let closing_sessions: Vec<_> = self
                .ws_sessions
                .borrow_mut()
                .drain()
                .map(|(_, handle)| handle.session)
                .collect();
            self.cancel_non_image_tasks_for_reload();
            for session in closing_sessions {
                self.task_queue.spawn_future(
                    async move {
                        let result = session
                            .close()
                            .await
                            .map_err(crate::action::ActionError::Custom);
                        ExecOutcome {
                            result,
                            warnings: Vec::new(),
                        }
                    },
                    self.document_generation,
                    Some("websocket:reload-close".into()),
                );
            }
        }

        if conform_reload {
            self.state.replace_app(&merged_state);
            self.state.replace_vars(&staged_vars);
            for (page_key, values) in &page_merges {
                self.state.replace_page(page_key, values);
            }
            for (page_key, node_id, values) in &self_merges {
                self.state.replace_self(page_key, node_id, values);
            }
            if !storage_allowed {
                self.state.replace_storage(&BTreeMap::new());
                self.state.storage_cache.purge();
            }
        }
        self.state.replace_route(&committed_route);
        self.state.set_responsive(responsive);
        let action_surface_inputs =
            crate::action_surface::derive_actions(&doc.schema, &crate::action_surface::BUILD_SALT);
        let focus_chain = collect_focus_chain(&doc);
        self.audit = Some(audit);
        self.capabilities = capabilities;
        if let Some(route_snapshot) = route_snapshot {
            self.nav.restore(route_snapshot, &valid_paths);
        }
        self.action_surface_inputs = action_surface_inputs;
        self.action_surface_generation = self.action_surface_generation.wrapping_add(1);
        self.load_warnings = prepared.warnings;
        if conform_reload {
            self.load_warnings.extend(conformance_warnings);
        }
        self.variant_source = prepared.source;
        self.variant_table = prepared.variants;
        self.active_screen_path = prepared.path;
        self.active_variant_page_id = prepared.selected_page_id.clone();
        self.active_page_key = if doc.schema.is_responsive() {
            prepared.selected_page_id.unwrap_or_default()
        } else {
            String::new()
        };
        self.widget_states
            .set_page_key(self.active_page_key.clone());
        self.image_store.begin_reload_ownership();
        self.document = Some(doc);
        if let Some((layout, spatial, layout_warnings)) = staged_geometry {
            self.layout.install(layout);
            self.spatial = spatial;
            for warning in layout_warnings {
                if !self.load_warnings.contains(&warning) {
                    self.load_warnings.push(warning);
                }
            }
            self.layout_mutation_seen = self.mutation_counter.get();
            self.mark_dirty();
        }
        // Preserve widget runtime state for ids that still exist in the
        // swapped-in tree; drop state for nodes that vanished.
        if let Some(doc) = self.document.as_ref() {
            self.widget_states
                .retain_ids(&|id| doc.tree.get(id).is_some());
            self.widget_states.revalidate(doc, &self.state);
        }
        // Hot-reload swaps the SlotMap underneath. SlotMap keys are
        // *not* unique across different SlotMaps — both the old and
        // new tree start their version counter at 1, so the first
        // insert into each map yields equal keys. Any cached
        // `NodeKey` from the pre-swap tree could silently dispatch
        // the next event to an unrelated new node, so blow away
        // every gesture-pipeline cache that holds one:
        //
        // - `focus.current` — cleared first (`set_chain` alone can't
        //   tell stale-but-equal apart from "really still in the
        //   chain"). Authors who want focus preserved across reload
        //   re-issue `focus_request` post-swap from
        //   `lifecycle.on_load`.
        // - `gestures` (PointerRouter): `raw_roots`,
        //   `last_hover_target`, `last_tap`, `multi_instances` —
        //   reset wholesale; in-flight pointer / hover sequences
        //   are torn down on hot-reload. Without this, the next
        //   hover after a `.op` edit could fire `HoverLeave`
        //   against a stale-but-equal key that now points to a
        //   different node in the new tree.
        self.focus.clear();
        self.focus.set_chain(focus_chain);
        self.gestures.reset();
        // Plan 19 D1 codex round 2 MEDIUM: a stale preload from a
        // prior `.op.pack` load survives the doc swap and `node_rect`
        // would serve rects keyed against the OLD slot keys whenever
        // the new tree happens to fill matching SecondaryMap slots.
        // Drop the cache unconditionally — hosts that hot-reload to
        // a doc with a fresh `.op.pack` re-call `preload_initial_layout`
        // explicitly.
        self.layout.drop_preload();
        self.state.clear_image_keys();
        self.admit_document_images();
        self.image_store.finish_reload_ownership();
        if conform_reload {
            self.transfer_reload_image_requests();
        }
        self.image_request_sources
            .retain(|key, _| self.image_store.state(key).is_some());
        Ok(())
    }
}
