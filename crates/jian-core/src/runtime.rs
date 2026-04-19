//! Runtime — the composition root.
//!
//! Typical startup:
//! ```ignore
//! let mut rt = Runtime::new();
//! rt.load_str(&src)?;
//! rt.build_layout((800.0, 600.0))?;
//! rt.rebuild_spatial();
//! ```
//!
//! Render is driven by the host, which calls `rt.render(&mut backend, &mut surface)`.

use crate::document::{loader, RuntimeDocument};
use crate::effect::EffectRegistry;
use crate::error::CoreResult;
use crate::geometry::size;
use crate::layout::LayoutEngine;
use crate::scene::SceneGraph;
use crate::signal::scheduler::Scheduler;
use crate::spatial::{NodeBBox, SpatialIndex};
use crate::state::StateGraph;
use crate::viewport::Viewport;
use jian_ops_schema::load_str;
use std::rc::Rc;

pub struct Runtime {
    pub scheduler: Rc<Scheduler>,
    pub effects: Rc<EffectRegistry>,
    pub state: StateGraph,
    pub document: Option<RuntimeDocument>,
    pub layout: LayoutEngine,
    pub spatial: SpatialIndex,
    pub viewport: Viewport,
    pub scene: SceneGraph,
}

impl Runtime {
    pub fn new() -> Self {
        let scheduler = Rc::new(Scheduler::new());
        let effects = EffectRegistry::new();
        effects.install_on(&scheduler);
        Self {
            state: StateGraph::new(scheduler.clone()),
            scheduler,
            effects,
            document: None,
            layout: LayoutEngine::new(),
            spatial: SpatialIndex::new(),
            viewport: Viewport::new(size(800.0, 600.0)),
            scene: SceneGraph::new(),
        }
    }

    pub fn load_str(&mut self, src: &str) -> CoreResult<()> {
        let schema = load_str(src)?.value;
        let doc = loader::build(schema, &self.state)?;
        self.document = Some(doc);
        Ok(())
    }

    pub fn build_layout(&mut self, available: (f32, f32)) -> CoreResult<()> {
        let doc = self.document.as_ref().expect("no document loaded");
        let roots = self.layout.build(&doc.tree)?;
        for root in roots {
            self.layout.compute(root, available)?;
        }
        Ok(())
    }

    pub fn rebuild_spatial(&mut self) {
        let doc = self.document.as_ref().expect("no document loaded");
        let items: Vec<NodeBBox> = doc
            .tree
            .nodes
            .iter()
            .filter_map(|(key, _)| {
                self.layout
                    .node_rect(key)
                    .map(|rect| NodeBBox { key, rect })
            })
            .collect();
        self.spatial.rebuild(items);
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_pipeline_smoke() {
        let mut rt = Runtime::new();
        rt.load_str(
            r#"{
          "version":"0.8.0",
          "children":[{"type":"rectangle","id":"r","width":200,"height":100}]
        }"#,
        )
        .unwrap();
        rt.build_layout((800.0, 600.0)).unwrap();
        rt.rebuild_spatial();
        assert_eq!(rt.spatial.len(), 1);
    }
}
