use super::layout_runtime::{root_has_limits, select_viewport_root};
use super::Runtime;
use crate::action_surface::{derive_actions, ActionDefinition, BUILD_SALT};
use crate::document::{loader, RuntimeDocument};
use crate::error::{CoreError, CoreResult};
use crate::gesture::collect_focus_chain;
use crate::layout::LayoutEngine;
use crate::signal::scheduler::Scheduler;
use crate::spatial::{NodeBBox, SpatialIndex};
use crate::state::StateGraph;
use crate::widget_state::WidgetStateStore;
use jian_ops_schema::PenDocument;
use std::cell::Cell;
use std::rc::Rc;

/// A complete, non-live candidate. Construction performs schema loading,
/// layout, spatial indexing, widget seeding and action-surface derivation.
pub struct ParkedBuild {
    pub target_page_id: String,
    pub document: RuntimeDocument,
    pub layout: LayoutEngine,
    pub spatial: SpatialIndex,
    pub widget_states: WidgetStateStore,
    pub action_surface_inputs: Vec<ActionDefinition>,
    pub warnings: Vec<String>,
    staged_state: Rc<StateGraph>,
    mutation_counter_at_build: u64,
    font_generation_at_build: u64,
    viewport_at_build: (f32, f32),
    build_count: usize,
    pub(crate) started_at_ms: u64,
}

#[derive(Default)]
pub enum SwapState {
    #[default]
    Idle,
    AwaitingIme {
        request_id: u64,
        parked: Box<ParkedBuild>,
    },
}

impl Runtime {
    pub fn configure_variants(
        &mut self,
        path: impl Into<String>,
        table: jian_ops_schema::screen_projection::ScreenVariantTable,
    ) {
        self.active_screen_path = Some(path.into());
        self.variant_table = table;
    }

    pub fn configure_variant_source(
        &mut self,
        source: PenDocument,
        path: impl Into<String>,
        table: jian_ops_schema::screen_projection::ScreenVariantTable,
    ) {
        self.variant_source = Some(source);
        self.configure_variants(path, table);
        if let Some(page_id) = self
            .document
            .as_ref()
            .and_then(|document| document.active_page.clone())
        {
            self.active_variant_page_id = Some(page_id.clone());
            self.active_page_key = page_id.clone();
            self.widget_states.set_page_key(page_id);
        }
    }

    pub fn selected_variant(&self) -> Option<&str> {
        self.active_variant_page_id.as_deref()
    }

    pub fn active_page_key(&self) -> &str {
        &self.active_page_key
    }

    /// Current projected screen path, when the document defines screens.
    pub fn active_screen_path(&self) -> Option<&str> {
        self.active_screen_path.as_deref()
    }

    /// Clone the projected route table for a host-owned router.
    pub fn screen_table(&self) -> Option<crate::screens::ScreenTable> {
        if let Some(source) = self.variant_source.clone() {
            crate::screens::ScreenTable::from_projected(source, self.variant_table.clone())
        } else {
            crate::screens::ScreenTable::from_document(self.document.as_ref()?.schema.clone())
        }
    }

    /// Changes whenever the mounted document's derived action set changes.
    pub fn action_surface_generation(&self) -> u64 {
        self.action_surface_generation
    }

    pub fn needs_variant_swap(&self, new_width: f32) -> Option<String> {
        let path = self.active_screen_path.as_deref()?;
        let variants = self.variant_table.0.get(path)?;
        let selected = variants
            .ranged
            .iter()
            .find(|entry| {
                entry.range.min_width.unwrap_or(0.0) as f32 <= new_width
                    && new_width <= entry.range.max_width.unwrap_or(f64::INFINITY) as f32
            })
            .map_or(variants.default_page_id.as_str(), |entry| {
                entry.page_id.as_str()
            });
        // While a swap is parked on an IME handshake, the pending target — not
        // the still-live variant — is what the confirmation will commit.
        // Compare against it so a second resize that crosses back re-parks the
        // now-correct target instead of silently leaving the stale one to
        // commit later.
        let current = match &self.swap_state {
            SwapState::AwaitingIme { parked, .. } => Some(parked.target_page_id.as_str()),
            SwapState::Idle => self.active_variant_page_id.as_deref(),
        };
        (current != Some(selected)).then(|| selected.to_owned())
    }

    pub fn input_frozen(&self) -> bool {
        matches!(self.swap_state, SwapState::AwaitingIme { .. })
    }

    /// Request id that must be resolved before a parked responsive swap can
    /// commit. Hosts surface this through their IME-control boundary.
    pub fn pending_variant_ime_request(&self) -> Option<u64> {
        match self.swap_state {
            SwapState::AwaitingIme { request_id, .. } => Some(request_id),
            SwapState::Idle => None,
        }
    }

    pub fn abandon_variant_swap(&mut self) {
        if let SwapState::AwaitingIme { request_id, .. } = self.swap_state {
            self.ime_registry.detach(request_id);
        }
        self.swap_state = SwapState::Idle;
    }

    pub fn mutation_counter(&self) -> u64 {
        self.mutation_counter.get()
    }

    pub fn last_variant_build_count(&self) -> usize {
        self.last_variant_build_count
    }

    /// Debug-only fault injection for the M1 transactional-swap acceptance
    /// test. Release builds do not expose or branch on this seam.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn inject_staged_variant_build_failure(&self) {
        self.fail_next_variant_build.set(true);
    }

    pub fn switch_variant(&mut self, target_page_id: &str) -> CoreResult<bool> {
        if matches!(self.swap_state, SwapState::Idle)
            && self.active_variant_page_id.as_deref() == Some(target_page_id)
        {
            return Ok(false);
        }
        let (started_at_ms, build_count) = match &self.swap_state {
            SwapState::AwaitingIme { parked, .. } => (parked.started_at_ms, parked.build_count + 1),
            SwapState::Idle => (self.now_ms, 1),
        };
        // The only fallible work happens before live metadata changes. A failed
        // rebuild of an already-parked request must detach that request and
        // lift freeze rather than leave the stale artifact active.
        let was_awaiting = matches!(self.swap_state, SwapState::AwaitingIme { .. });
        let parked = match self.build_parked(target_page_id, started_at_ms, build_count) {
            Ok(parked) => parked,
            Err(error) => {
                if was_awaiting {
                    self.abandon_variant_swap();
                    self.push_layout_error(format!("parked variant rebuild failed: {error}"));
                }
                return Err(error);
            }
        };
        if let SwapState::AwaitingIme {
            parked: current, ..
        } = &mut self.swap_state
        {
            **current = parked;
            return Ok(false);
        }
        if let Some(snapshot) = self.active_ime_snapshot() {
            let request_id = self.begin_ime_handshake(snapshot);
            self.swap_state = SwapState::AwaitingIme {
                request_id,
                parked: Box::new(parked),
            };
            return Ok(false);
        }
        self.commit_parked(parked)?;
        Ok(true)
    }

    fn build_parked(
        &self,
        target_page_id: &str,
        started_at_ms: u64,
        build_count: usize,
    ) -> CoreResult<ParkedBuild> {
        let font_generation_at_build = self.layout.measure.font_generation();
        let source = self
            .variant_source
            .as_ref()
            .ok_or_else(|| CoreError::Layout("variant source is not configured".into()))?;
        let page = source
            .pages
            .as_ref()
            .and_then(|pages| pages.iter().find(|page| page.id == target_page_id))
            .cloned()
            .ok_or_else(|| CoreError::Layout(format!("unknown variant page `{target_page_id}`")))?;
        #[cfg(debug_assertions)]
        if self.fail_next_variant_build.replace(false) {
            return Err(CoreError::Layout(
                "injected staged variant build failure".into(),
            ));
        }
        let mut schema: PenDocument = source.clone();
        schema.pages = Some(vec![page]);

        let staging_counter = Rc::new(Cell::new(0));
        let staging_state = Rc::new(StateGraph::new_with_counter(
            Rc::new(Scheduler::new()),
            staging_counter.clone(),
        ));
        copy_live_seed_state(&self.state, &staging_state);
        let document =
            loader::build_with(schema, &staging_state, loader::SeedMode::PreserveExisting)?;

        // Mirror `build_layout`: responsive geometry is computed from a
        // materialized clone so layout-affecting bindings (e.g. a width bound to
        // `$viewport.width`) resolve against the staged state for this variant,
        // while the committed live document keeps its raw bindings for later
        // re-materialization. Without this the swapped-in variant would lay out
        // with authored placeholder values.
        let mut materialized = document.clone();
        for (_, node) in materialized.tree.nodes.iter_mut() {
            crate::binding::materialize_layout_bindings(
                &mut node.schema,
                &staging_state,
                Some(target_page_id),
            );
        }

        let viewport = (self.viewport.size.width, self.viewport.size.height);
        let mut layout = LayoutEngine::with_backend(self.layout.measure.clone());
        let roots = layout.build_responsive(&materialized.tree, true)?;
        let mut warnings = layout.constraint_lints().to_vec();
        if let Some(root) = select_viewport_root(&materialized.tree, &mut warnings) {
            if root_has_limits(&materialized.tree.nodes[root].schema) {
                warnings.push("responsive viewport root min/max bounds are ignored".to_owned());
            }
            layout.override_root_for_viewport(root, viewport)?;
        }
        for root in roots {
            layout.compute_responsive(root, viewport)?;
        }
        let mut spatial = SpatialIndex::new();
        spatial.rebuild(
            materialized
                .tree
                .nodes
                .iter()
                .filter(|(_, node)| {
                    // Mirror the normal layout path: `visible: false` nodes are
                    // not drawn and must not be hit-testable either.
                    serde_json::to_value(&node.schema)
                        .ok()
                        .and_then(|json| json.get("visible").and_then(|value| value.as_bool()))
                        .unwrap_or(true)
                })
                .filter_map(|(key, _)| {
                    layout
                        .node_scene_rect(&materialized, key)
                        .map(|rect| NodeBBox { key, rect })
                }),
        );

        let mut widget_states = self
            .widget_states
            .clone_with_counter(staging_counter.clone());
        widget_states.set_page_key(target_page_id);
        for (_, node) in document.tree.nodes.iter() {
            let _ = widget_states.get_or_init(&node.schema, &staging_state);
        }
        let action_surface_inputs = derive_actions(&document.schema, &BUILD_SALT);

        Ok(ParkedBuild {
            target_page_id: target_page_id.to_owned(),
            document,
            layout,
            spatial,
            widget_states,
            action_surface_inputs,
            warnings,
            staged_state: staging_state,
            mutation_counter_at_build: self.mutation_counter(),
            font_generation_at_build,
            viewport_at_build: viewport,
            build_count,
            started_at_ms,
        })
    }

    pub(crate) fn commit_parked(&mut self, mut parked: ParkedBuild) -> CoreResult<()> {
        // The viewport is compared directly rather than through the mutation
        // counter: a host that re-lays out between park and commit (e.g. the
        // desktop resize path calls `build_layout` after parking) moves
        // `self.viewport` without bumping the counter, and its later
        // `set_viewport_size` becomes a no-op that never bumps either.
        if parked.mutation_counter_at_build != self.mutation_counter()
            || parked.font_generation_at_build != self.layout.measure.font_generation()
            || parked.viewport_at_build != (self.viewport.size.width, self.viewport.size.height)
        {
            parked = self.build_parked(
                &parked.target_page_id,
                parked.started_at_ms,
                parked.build_count + 1,
            )?;
        }
        merge_staged_defaults(&parked.staged_state, &self.state);
        let focus_chain = collect_focus_chain(&parked.document);
        let target = parked.target_page_id.clone();
        self.last_variant_build_count = parked.build_count;

        // Reset all retained transient state, including inactive page entries,
        // before the atomic field installation. Durable values remain intact.
        parked.widget_states.reset_transients();
        parked.widget_states.set_page_key(target.clone());
        parked
            .widget_states
            .set_mutation_counter(self.mutation_counter.clone());
        // Rotate image ownership across the document swap exactly like a normal
        // mount: begin a new ownership generation before installing the tree,
        // then re-admit against the swapped-in variant so its images (which may
        // differ from the previous variant's) are registered and requested, and
        // release any that no longer appear.
        self.image_store.begin_reload_ownership();
        self.document = Some(parked.document);
        self.layout = parked.layout;
        self.spatial = parked.spatial;
        self.text_geometry_ready = true;
        self.widget_states = parked.widget_states;
        self.action_surface_inputs = parked.action_surface_inputs;
        self.action_surface_generation = self.action_surface_generation.wrapping_add(1);
        self.state.clear_image_keys();
        self.admit_document_images();
        self.image_store.finish_reload_ownership();
        self.image_request_sources
            .retain(|key, _| self.image_store.state(key).is_some());
        for warning in parked.warnings {
            if !self.load_warnings.contains(&warning) {
                self.load_warnings.push(warning);
            }
        }
        self.active_variant_page_id = Some(target.clone());
        self.active_page_key = target;
        self.focus.clear();
        self.focus.set_chain(focus_chain);
        self.gestures.reset();
        self.swap_state = SwapState::Idle;
        self.mutation_counter
            .set(self.mutation_counter.get().wrapping_add(1));
        // The parked geometry was built from the current viewport and staged
        // state, including defaults merged above. Do not make the next pump
        // redundantly rebuild the layout that was just committed.
        self.layout_mutation_seen = self.mutation_counter.get();
        Ok(())
    }

    pub(crate) fn complete_parked_after_ime(&mut self, request_id: u64) {
        let state = std::mem::take(&mut self.swap_state);
        match state {
            SwapState::AwaitingIme {
                request_id: current,
                parked,
            } if current == request_id => {
                // A resize that crossed out and back while the handshake was
                // pending re-parked the still-live variant. Committing it
                // would only churn observable state (focus cleared, caret
                // reset) for a document that is already mounted — drop the
                // park instead and just lift the freeze.
                if self.active_variant_page_id.as_deref() == Some(parked.target_page_id.as_str()) {
                    return;
                }
                if let Err(error) = self.commit_parked(*parked) {
                    self.push_layout_error(format!("variant swap commit failed: {error}"));
                    self.swap_state = SwapState::Idle;
                }
            }
            other => self.swap_state = other,
        }
    }
}

fn copy_live_seed_state(live: &StateGraph, staging: &StateGraph) {
    // The staging graph must evaluate every binding scope exactly like the
    // live graph, otherwise layout-binding materialization for the parked
    // variant resolves against nulls/defaults and diverges from the geometry
    // a live relayout would later compute. The responsive flag is copied
    // first: responsive `$storage` reads go through the storage cache, and
    // `replace_storage` only populates the staging cache when the flag is
    // already set.
    staging.set_responsive(live.is_responsive());
    staging.set_now_ms(live.now_ms());
    staging.replace_viewport(&live.viewport_snapshot());
    staging.replace_route(&live.route_snapshot());
    if live.is_responsive() {
        // Responsive `$storage` evaluation reads the storage cache, not the
        // signal map: hydrated values exist only in the cache, and the map can
        // retain entries a wipe already dropped from the cache. Copy the
        // cache's present entries verbatim; reseeding through the map would
        // resurrect cleared values or miss hydrated ones.
        if let serde_json::Value::Object(entries) = live.storage_cache.snapshot() {
            for (key, value) in entries {
                staging.storage_cache.set_local(&key, value);
            }
        }
    } else {
        staging.replace_storage(&live.storage_snapshot());
    }
    for (name, signal) in live.app.borrow().iter() {
        staging.app_set(name, signal.get().0);
    }
    for (name, signal) in live.vars.borrow().iter() {
        staging.vars_set(name, signal.get().0);
    }
    // Retained `$page`/`$self` values survive variant swaps; the loader's
    // `PreserveExisting` seeding only fills keys that are still missing, so
    // copying them here means a switch back re-materializes against the
    // user's mutated values rather than authored defaults.
    for (page_key, fields) in live.page.borrow().iter() {
        for (name, signal) in fields {
            staging.page_set(page_key, name, signal.get().0);
        }
    }
    for ((page_key, node_id), fields) in live.self_.borrow().iter() {
        for (name, signal) in fields {
            staging.self_set(page_key, node_id, name, signal.get().0);
        }
    }
}

fn merge_staged_defaults(staging: &StateGraph, live: &StateGraph) {
    for (name, signal) in staging.app.borrow().iter() {
        if live.app_get(name).is_none() {
            live.app_set(name, signal.get().0);
        }
    }
    for (name, signal) in staging.vars.borrow().iter() {
        if live.vars_get(name).is_none() {
            live.vars_set(name, signal.get().0);
        }
    }
    // First-visit `$page`/`$self` defaults seeded into the staging graph must
    // reach the live graph too, or post-commit evaluation of the committed
    // variant's page-scoped bindings would see missing keys where the parked
    // build saw the authored defaults.
    for (page_key, fields) in staging.page.borrow().iter() {
        for (name, signal) in fields {
            if live.page_get(page_key, name).is_none() {
                live.page_set(page_key, name, signal.get().0);
            }
        }
    }
    for ((page_key, node_id), fields) in staging.self_.borrow().iter() {
        for (name, signal) in fields {
            if live.self_get(page_key, node_id, name).is_none() {
                live.self_set(page_key, node_id, name, signal.get().0);
            }
        }
    }
}
