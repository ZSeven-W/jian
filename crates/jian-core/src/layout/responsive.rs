use super::{constraints, LayoutEngine};
use crate::document::NodeKey;
#[cfg(test)]
use crate::document::NodeTree;
use crate::error::{CoreError, CoreResult};
use slotmap::SecondaryMap;
use taffy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResolvedBox {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl LayoutEngine {
    /// Compute one root, resolve only that root's responsive subtree, and recompute.
    pub fn compute_responsive(&mut self, root: NodeId, viewport: (f32, f32)) -> CoreResult<()> {
        const COMPUTE_BOUND: usize = 3;
        let root_key = self
            .map
            .iter()
            .find_map(|(key, node_id)| (*node_id == root).then_some(key))
            .ok_or_else(|| CoreError::Layout("responsive root is not in the layout map".into()))?;

        self.restore_base_styles(root_key)?;
        self.overrides = SecondaryMap::new();
        self.compute_count = 0;
        self.bound_hit = false;
        self.compute(root, viewport)?;
        self.compute_count = 1;
        if self.reference.is_none() {
            return Ok(());
        }

        for _ in 0..(COMPUTE_BOUND - 1) {
            if !self.resolve_walk(root_key)? {
                return Ok(());
            }
            self.apply_overrides()?;
            self.compute(root, viewport)?;
            self.compute_count += 1;
        }
        if self.resolve_walk(root_key)? {
            self.bound_hit = true;
            #[cfg(debug_assertions)]
            eprintln!("constraint loop hit the {COMPUTE_BOUND}-compute bound; last compute stands");
        }
        Ok(())
    }

    fn restore_base_styles(&mut self, root: NodeKey) -> CoreResult<()> {
        for &key in &self.node_order {
            if self.root_owner.get(key) != Some(&root) {
                continue;
            }
            let style = self.base_styles[key].clone();
            self.tree
                .set_style(self.map[key], style)
                .map_err(|error| CoreError::Layout(error.to_string()))?;
        }
        Ok(())
    }

    fn resolve_walk(&mut self, root: NodeKey) -> CoreResult<bool> {
        let reference = self
            .reference
            .as_ref()
            .expect("responsive compute requires a reference table");
        let mut effective: SecondaryMap<NodeKey, (f32, f32)> = SecondaryMap::new();
        let mut changed = false;

        for &key in &self.node_order {
            if self.root_owner.get(key) != Some(&root) {
                continue;
            }
            let node_id = self.map[key];
            let layout = self
                .tree
                .layout(node_id)
                .map_err(|error| CoreError::Layout(error.to_string()))?;
            let mut resolved = ResolvedBox {
                x: layout.location.x,
                y: layout.location.y,
                width: layout.size.width,
                height: layout.size.height,
            };
            let Some(&parent_key) = self.parent.get(key) else {
                effective.insert(key, (resolved.width, resolved.height));
                continue;
            };
            let (parent_width, parent_height) =
                effective.get(parent_key).copied().unwrap_or_else(|| {
                    let parent_layout = self.tree.layout(self.map[parent_key]).unwrap();
                    (parent_layout.size.width, parent_layout.size.height)
                });
            let Some(node_ref) = reference.get(key) else {
                effective.insert(key, (resolved.width, resolved.height));
                continue;
            };
            let mut constrained = false;
            if let Some(axis) = node_ref.h {
                let clamp_hits = axis.max.is_some_and(|max| axis.size > max)
                    || axis.min.is_some_and(|min| axis.size < min);
                if (parent_width - axis.parent_size).abs() > f32::EPSILON || clamp_hits {
                    let (x, width) =
                        constraints::resolve_axis(node_ref.h_kind, &axis, parent_width);
                    resolved.x = x;
                    resolved.width = width;
                    constrained = true;
                }
            }
            if let Some(axis) = node_ref.v {
                let clamp_hits = axis.max.is_some_and(|max| axis.size > max)
                    || axis.min.is_some_and(|min| axis.size < min);
                if (parent_height - axis.parent_size).abs() > f32::EPSILON || clamp_hits {
                    let (y, height) = constraints::resolve_axis(
                        constraints::vertical_as_h(node_ref.v_kind),
                        &axis,
                        parent_height,
                    );
                    resolved.y = y;
                    resolved.height = height;
                    constrained = true;
                }
            }
            effective.insert(key, (resolved.width, resolved.height));
            if constrained && self.overrides.get(key) != Some(&resolved) {
                self.overrides.insert(key, resolved);
                changed = true;
            }
        }
        Ok(changed)
    }

    fn apply_overrides(&mut self) -> CoreResult<()> {
        for (key, resolved) in &self.overrides {
            let node_id = self.map[key];
            let mut style = self
                .tree
                .style(node_id)
                .map_err(|error| CoreError::Layout(error.to_string()))?
                .clone();
            style.position = Position::Absolute;
            style.inset.left = length(resolved.x);
            style.inset.top = length(resolved.y);
            style.size.width = length(resolved.width);
            style.size.height = length(resolved.height);
            self.tree
                .set_style(node_id, style)
                .map_err(|error| CoreError::Layout(error.to_string()))?;
        }
        Ok(())
    }

    pub fn last_compute_count(&self) -> usize {
        self.compute_count
    }

    pub fn last_bound_hit(&self) -> bool {
        self.bound_hit
    }

    pub fn override_root_for_viewport(
        &mut self,
        root: NodeKey,
        viewport: (f32, f32),
    ) -> CoreResult<()> {
        let node_id = self.map[root];
        let mut style = self
            .tree
            .style(node_id)
            .map_err(|error| CoreError::Layout(error.to_string()))?
            .clone();
        style.position = Position::Relative;
        style.inset = taffy::geometry::Rect {
            left: auto(),
            right: auto(),
            top: auto(),
            bottom: auto(),
        };
        style.size.width = length(viewport.0);
        style.size.height = length(viewport.1);
        style.min_size = Size::auto();
        style.max_size = Size::auto();
        self.tree
            .set_style(node_id, style.clone())
            .map_err(|error| CoreError::Layout(error.to_string()))?;
        self.base_styles.insert(root, style);
        self.origin_normalized.insert(root);
        Ok(())
    }

    pub fn is_origin_normalized(&self, root: NodeKey) -> bool {
        self.origin_normalized.contains(&root)
    }

    pub fn constraint_lints(&self) -> &[String] {
        &self.constraint_lints
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_ops_schema::node::PenNode;

    fn setup(json: &str, viewport: (f32, f32)) -> (NodeTree, LayoutEngine, NodeId) {
        let root: PenNode = serde_json::from_str(json).unwrap();
        let mut doc = NodeTree::new();
        let root_key = doc.insert_subtree(root, None);
        let mut engine = LayoutEngine::new();
        let root_id = engine.build_responsive(&doc, true).unwrap()[0];
        engine
            .override_root_for_viewport(root_key, viewport)
            .unwrap();
        (doc, engine, root_id)
    }

    #[test]
    fn compute_responsive_stretch_reflows_fill_child() {
        let (doc, mut engine, root) = setup(
            r#"{"type":"frame","id":"root","width":400,"height":300,"children":[
                {"type":"frame","id":"panel","x":20,"y":0,"width":360,"height":100,
                "constraints":{"h":"left_right","v":"top"},"layout":"vertical",
                "children":[{"type":"rectangle","id":"fill","width":"fill_container","height":40}]}]}"#,
            (600.0, 300.0),
        );
        engine.compute_responsive(root, (600.0, 300.0)).unwrap();
        assert_eq!(
            engine
                .node_rect(doc.get("panel").unwrap())
                .unwrap()
                .size
                .width,
            560.0
        );
        assert_eq!(
            engine
                .node_rect(doc.get("fill").unwrap())
                .unwrap()
                .size
                .width,
            560.0
        );
    }

    #[test]
    fn compute_responsive_injects_position_only_change() {
        let (doc, mut engine, root) = setup(
            r#"{"type":"frame","id":"root","width":400,"height":300,"children":[
                {"type":"rectangle","id":"btn","x":300,"y":10,"width":80,"height":40,
                "constraints":{"h":"right","v":"top"}}]}"#,
            (600.0, 300.0),
        );
        engine.compute_responsive(root, (600.0, 300.0)).unwrap();
        let rect = engine.node_rect(doc.get("btn").unwrap()).unwrap();
        assert_eq!((rect.origin.x, rect.size.width), (500.0, 80.0));
    }

    #[test]
    fn compute_responsive_reanchors_own_clamp_and_nested_chain_in_two_computes() {
        let (doc, mut engine, root) = setup(
            r#"{"type":"frame","id":"root","width":400,"height":300,"children":[
                {"type":"frame","id":"panel","x":20,"y":0,"width":360,"height":100,
                "constraints":{"h":"left_right","v":"top"},"children":[
                    {"type":"rectangle","id":"inner","x":340,"y":0,"width":20,"height":20,
                    "maxWidth":15,"constraints":{"h":"right","v":"top"}}]}]}"#,
            (600.0, 300.0),
        );
        engine.compute_responsive(root, (600.0, 300.0)).unwrap();
        let inner = engine.node_rect(doc.get("inner").unwrap()).unwrap();
        assert_eq!((inner.origin.x, inner.size.width), (565.0, 15.0));
        assert_eq!(engine.last_compute_count(), 2);
    }

    #[test]
    fn legacy_compute_leaves_constraints_inert() {
        let root: PenNode = serde_json::from_str(
            r#"{"type":"frame","id":"root","width":400,"height":300,"children":[
                {"type":"rectangle","id":"c","x":80,"y":0,"width":30,"height":10,
                "constraints":{"h":"right","v":"top"}}]}"#,
        )
        .unwrap();
        let mut doc = NodeTree::new();
        doc.insert_subtree(root, None);
        let mut engine = LayoutEngine::new();
        let root = engine.build(&doc).unwrap()[0];
        engine.compute(root, (600.0, 300.0)).unwrap();
        assert_eq!(
            engine.node_rect(doc.get("c").unwrap()).unwrap().origin.x,
            80.0
        );
    }

    #[test]
    fn responsive_compute_is_repeatable_without_rebuild() {
        let (doc, mut engine, root) = setup(
            r#"{"type":"frame","id":"root","width":400,"height":300,"children":[
                {"type":"rectangle","id":"btn","x":300,"y":10,"width":80,"height":40,
                "constraints":{"h":"right","v":"top"}}]}"#,
            (600.0, 300.0),
        );
        engine.compute_responsive(root, (600.0, 300.0)).unwrap();
        engine.compute_responsive(root, (600.0, 300.0)).unwrap();
        let rect = engine.node_rect(doc.get("btn").unwrap()).unwrap();
        assert_eq!((rect.origin.x, rect.size.width), (500.0, 80.0));
        assert_eq!(engine.last_compute_count(), 2);
    }

    #[test]
    fn current_root_compute_does_not_mutate_extra_root_styles() {
        let first: PenNode =
            serde_json::from_str(r#"{"type":"frame","id":"first","width":400,"height":300}"#)
                .unwrap();
        let second: PenNode = serde_json::from_str(
            r#"{"type":"frame","id":"second","width":100,"height":100,"children":[
                {"type":"rectangle","id":"child","x":80,"y":0,"width":30,"height":10,
                "maxWidth":20,"constraints":{"h":"right","v":"top"}}]}"#,
        )
        .unwrap();
        let mut doc = NodeTree::new();
        doc.insert_subtree(first, None);
        doc.insert_subtree(second, None);
        let mut engine = LayoutEngine::new();
        let roots = engine.build_responsive(&doc, true).unwrap();
        engine.compute_responsive(roots[0], (400.0, 300.0)).unwrap();

        let child = doc.get("child").unwrap();
        let style = engine.tree.style(engine.map[child]).unwrap();
        assert_eq!(style.size.width, length(30.0));

        engine.compute_responsive(roots[1], (100.0, 100.0)).unwrap();
        let rect = engine.node_rect(child).unwrap();
        assert_eq!((rect.origin.x, rect.size.width), (90.0, 20.0));

        let mut reverse = LayoutEngine::new();
        let reverse_roots = reverse.build_responsive(&doc, true).unwrap();
        reverse
            .compute_responsive(reverse_roots[1], (100.0, 100.0))
            .unwrap();
        let first_style = reverse.tree.style(reverse_roots[0]).unwrap();
        assert_eq!(first_style.size.width, length(400.0));
        reverse
            .compute_responsive(reverse_roots[0], (400.0, 300.0))
            .unwrap();
        assert_eq!(reverse.node_rect(child).unwrap().size.width, 20.0);
    }

    #[test]
    fn defensive_bound_reports_when_order_is_deliberately_corrupted() {
        let (_doc, mut engine, root) = setup(
            r#"{"type":"frame","id":"root","width":400,"height":300,"children":[
                {"type":"frame","id":"a","x":20,"y":0,"width":360,"height":100,
                "constraints":{"h":"left_right","v":"top"},"children":[
                    {"type":"frame","id":"b","x":20,"y":0,"width":320,"height":80,
                    "constraints":{"h":"left_right","v":"top"},"children":[
                        {"type":"rectangle","id":"c","x":300,"y":0,"width":20,"height":20,
                        "constraints":{"h":"right","v":"top"}}]}]}]}"#,
            (600.0, 300.0),
        );
        engine.node_order.reverse();
        engine.compute_responsive(root, (600.0, 300.0)).unwrap();
        assert_eq!(engine.last_compute_count(), 3);
        assert!(engine.last_bound_hit());
    }
}
