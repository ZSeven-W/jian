use super::Runtime;
use crate::error::CoreResult;
use crate::geometry::size;
use crate::spatial::{NodeBBox, SpatialIndex};
use std::collections::HashSet;
use std::rc::Rc;

impl Runtime {
    pub fn build_layout(&mut self, available: (f32, f32)) -> CoreResult<()> {
        // Captured BEFORE any measurement: a registration racing the build
        // window must leave `font_generation_seen` behind the global counter
        // so the next pump repairs the mixed-generation geometry (§6.5's
        // at-most-one-frame staleness guarantee).
        let font_generation_at_build = self.layout.measure.font_generation();
        let responsive = self
            .document
            .as_ref()
            .expect("no document loaded")
            .schema
            .is_responsive();
        if responsive {
            self.viewport.size = size(available.0, available.1);
            self.state.set_viewport_size(available.0, available.1);
        }
        let live_doc = self.document.as_ref().expect("no document loaded");
        let mut materialized;
        let document = if responsive {
            materialized = live_doc.clone();
            for (_, node) in materialized.tree.nodes.iter_mut() {
                crate::binding::materialize_layout_bindings(
                    &mut node.schema,
                    &self.state,
                    Some(&self.active_page_key),
                );
            }
            &materialized
        } else {
            live_doc
        };
        // Seed tabs before deriving the active runtime tree. This makes a
        // persisted `bind:value` authoritative on the first layout instead of
        // briefly indexing the authored panel until the first paint.
        for (_, node) in document.tree.nodes.iter() {
            if matches!(node.schema, jian_ops_schema::node::PenNode::Tabs(_)) {
                let _ = self.widget_states.get_or_init(&node.schema, &self.state);
            }
        }
        let mut staged = self.layout.build_staged(document)?;
        if !responsive {
            for root in staged.roots.iter().copied() {
                staged.engine.compute(root, available)?;
            }
        } else {
            for warning in staged.engine.constraint_lints().to_vec() {
                if !self.load_warnings.contains(&warning) {
                    self.load_warnings.push(warning);
                }
            }
            let viewport_root = select_viewport_root(&document.tree, &mut self.load_warnings);
            if let Some(root_key) = viewport_root {
                if root_has_limits(&document.tree.nodes[root_key].schema) {
                    let warning = "responsive viewport root min/max bounds are ignored".to_owned();
                    if !self.load_warnings.contains(&warning) {
                        self.load_warnings.push(warning);
                    }
                }
                staged
                    .engine
                    .override_root_for_viewport(root_key, available)?;
            }
            for root in staged.roots.iter().copied() {
                staged.engine.compute_responsive(root, available)?;
            }
        }

        let active_nodes: HashSet<_> =
            crate::gesture::focus::active_tree_nodes(document, Some(&self.widget_states))
                .into_iter()
                .collect();
        let items: Vec<NodeBBox> = document
            .tree
            .nodes
            .iter()
            .filter(|(key, _)| active_nodes.contains(key))
            .filter(|(_, node)| {
                serde_json::to_value(&node.schema)
                    .ok()
                    .and_then(|json| json.get("visible").and_then(|value| value.as_bool()))
                    .unwrap_or(true)
            })
            .filter_map(|(key, _)| {
                staged
                    .engine
                    .node_scene_rect(document, key)
                    .map(|rect| NodeBBox { key, rect })
            })
            .collect();
        let focused_became_hidden = self.focus.current().is_some_and(|focused| {
            !active_nodes.contains(&focused)
                || document.tree.nodes.get(focused).is_some_and(|node| {
                    serde_json::to_value(&node.schema)
                        .ok()
                        .and_then(|json| json.get("visible").and_then(|value| value.as_bool()))
                        == Some(false)
                })
        });
        let focus_chain =
            crate::gesture::focus::collect_focus_chain_with_states(document, &self.widget_states);
        let mut spatial = SpatialIndex::new();
        spatial.rebuild(items);
        self.layout.install(staged);
        self.spatial = spatial;
        self.focus.set_chain(focus_chain);
        self.text_geometry_ready = true;
        if focused_became_hidden {
            self.focus.clear();
        }
        self.layout_mutation_seen = self.mutation_counter.get();
        self.font_generation_seen = font_generation_at_build;
        self.mark_dirty();
        Ok(())
    }

    pub fn relayout(&mut self) -> CoreResult<()> {
        self.build_layout((self.viewport.size.width, self.viewport.size.height))
    }

    pub fn set_viewport_size(&mut self, viewport: (f32, f32)) {
        self.update_viewport_size(viewport, true);
    }

    pub fn set_viewport_size_without_relayout(&mut self, viewport: (f32, f32)) {
        self.update_viewport_size(viewport, false);
    }

    fn update_viewport_size(&mut self, viewport: (f32, f32), relayout: bool) {
        if (self.viewport.size.width, self.viewport.size.height) != viewport {
            self.viewport.size = size(viewport.0, viewport.1);
            self.state.set_viewport_size(viewport.0, viewport.1);
            self.scheduler.flush();
            self.mutation_counter
                .set(self.mutation_counter.get().wrapping_add(1));
            self.mark_dirty();
            let responsive = self
                .document
                .as_ref()
                .is_some_and(|document| document.schema.is_responsive());
            if relayout && responsive {
                if let Err(error) = self.relayout() {
                    self.push_layout_error(format!("viewport relayout failed: {error}"));
                }
            } else if !relayout {
                self.layout_mutation_seen = self.mutation_counter.get();
            }
        }
    }

    pub fn preload_initial_layout(
        &mut self,
        snapshot: &jian_ops_schema::pack::initial_layout::InitialLayoutSnapshot,
    ) -> usize {
        let Some(document) = self.document.as_ref() else {
            return 0;
        };
        self.layout.preload_initial(snapshot, &document.tree)
    }

    pub fn build_layout_with(
        &mut self,
        measure: Rc<dyn crate::layout::measure::MeasureBackend>,
        available: (f32, f32),
    ) -> CoreResult<()> {
        self.layout.set_backend(measure);
        self.build_layout(available)
    }

    pub fn rebuild_spatial(&mut self) {
        let document = self.document.as_ref().expect("no document loaded");
        let active_nodes =
            crate::gesture::focus::active_tree_nodes(document, Some(&self.widget_states));
        let items: Vec<NodeBBox> = active_nodes
            .iter()
            .filter_map(|&key| {
                self.layout
                    .node_scene_rect(document, key)
                    .map(|rect| NodeBBox { key, rect })
            })
            .collect();
        self.spatial.rebuild(items);
        let focus_chain =
            crate::gesture::focus::collect_focus_chain_with_states(document, &self.widget_states);
        self.focus.set_chain(focus_chain);
    }

    pub fn node_scene_rect(&self, key: crate::document::NodeKey) -> Option<crate::geometry::Rect> {
        let document = self.document.as_ref()?;
        self.layout.node_scene_rect(document, key)
    }

    pub fn focused_node_rect(&self) -> Option<crate::geometry::Rect> {
        self.focus
            .current()
            .and_then(|key| self.node_scene_rect(key))
    }

    pub fn rebuild_spatial_for_first_frame(
        &mut self,
        viewport: crate::geometry::Rect,
    ) -> Vec<NodeBBox> {
        let document = self.document.as_ref().expect("no document loaded");
        let mut visible = Vec::new();
        let mut hidden = Vec::new();
        for key in crate::gesture::focus::active_tree_nodes(document, Some(&self.widget_states)) {
            let Some(rect) = self.layout.node_scene_rect(document, key) else {
                continue;
            };
            let bbox = NodeBBox { key, rect };
            if rects_intersect(rect, viewport) {
                visible.push(bbox);
            } else {
                hidden.push(bbox);
            }
        }
        self.spatial.rebuild(visible);
        hidden
    }
}

pub(super) fn select_viewport_root(
    tree: &crate::document::NodeTree,
    warnings: &mut Vec<String>,
) -> Option<crate::document::NodeKey> {
    let &first = tree.roots.first()?;
    if tree.roots.len() > 1 {
        let warning =
            "responsive document has extra top-level roots; only the first root is viewport-sized"
                .to_owned();
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    }
    if !matches!(
        tree.nodes[first].schema,
        jian_ops_schema::node::PenNode::Frame(_)
    ) {
        let warning =
            "responsive document's first top-level node is not a frame; viewport sizing skipped"
                .to_owned();
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
        return None;
    }
    Some(first)
}

pub(super) fn root_has_limits(node: &jian_ops_schema::node::PenNode) -> bool {
    let jian_ops_schema::node::PenNode::Frame(frame) = node else {
        return false;
    };
    let limits = frame.container.limits;
    limits.min_width.is_some()
        || limits.max_width.is_some()
        || limits.min_height.is_some()
        || limits.max_height.is_some()
}

fn rects_intersect(a: crate::geometry::Rect, b: crate::geometry::Rect) -> bool {
    a.min_x() <= b.max_x()
        && a.max_x() >= b.min_x()
        && a.min_y() <= b.max_y()
        && a.max_y() >= b.min_y()
}
