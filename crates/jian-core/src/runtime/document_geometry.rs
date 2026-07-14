use super::layout_runtime::{root_has_limits, select_viewport_root};
use super::Runtime;
use crate::document::RuntimeDocument;
use crate::error::CoreResult;
use crate::layout::StagedLayout;
use crate::spatial::{NodeBBox, SpatialIndex};
use crate::state::StateGraph;

impl Runtime {
    pub(super) fn stage_document_geometry(
        &self,
        live_doc: &RuntimeDocument,
        state: &StateGraph,
        page_key: &str,
        available: (f32, f32),
    ) -> CoreResult<(StagedLayout, SpatialIndex, Vec<String>)> {
        let responsive = live_doc.schema.is_responsive();
        let mut materialized;
        let doc = if responsive {
            materialized = live_doc.clone();
            for (_, node) in materialized.tree.nodes.iter_mut() {
                crate::binding::materialize_layout_bindings(
                    &mut node.schema,
                    state,
                    Some(page_key),
                );
            }
            &materialized
        } else {
            live_doc
        };
        let mut staged = self.layout.build_staged(doc)?;
        let mut warnings = Vec::new();
        if responsive {
            warnings.extend(staged.engine.constraint_lints().iter().cloned());
            if let Some(root) = select_viewport_root(&doc.tree, &mut warnings) {
                if root_has_limits(&doc.tree.nodes[root].schema) {
                    warnings.push("responsive viewport root min/max bounds are ignored".to_owned());
                }
                staged.engine.override_root_for_viewport(root, available)?;
            }
            for root in staged.roots.iter().copied() {
                staged.engine.compute_responsive(root, available)?;
            }
        } else {
            for root in staged.roots.iter().copied() {
                staged.engine.compute(root, available)?;
            }
        }
        let items: Vec<NodeBBox> = doc
            .tree
            .nodes
            .iter()
            .filter(|(_, node)| {
                serde_json::to_value(&node.schema)
                    .ok()
                    .and_then(|json| json.get("visible").and_then(serde_json::Value::as_bool))
                    .unwrap_or(true)
            })
            .filter_map(|(key, _)| {
                staged
                    .engine
                    .node_scene_rect(doc, key)
                    .map(|rect| NodeBBox { key, rect })
            })
            .collect();
        let mut spatial = SpatialIndex::new();
        spatial.rebuild(items);
        Ok((staged, spatial, warnings))
    }
}
