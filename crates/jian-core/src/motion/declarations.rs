//! What a document declares about motion, read straight from the schema.

use crate::action::services::Easing;
use crate::document::tree::node_schema_id;
use crate::Runtime;
use jian_ops_schema::motion::{NodeAnimation, Transition, P1_MOTION_PROPERTIES};
use jian_ops_schema::node::PenNode;
use jian_ops_schema::PenDocument;

/// Material 3 `md.sys.motion.duration.short3` (150 ms) with the
/// `standard` curve (`md.sys.motion.easing.standard`, cubic-bezier(0.2, 0, 0, 1)).
/// Used as the intrinsic switch on/off tween when the node has no `transition`.
/// Sources: <https://m3.material.io/styles/motion/easing-and-duration/tokens-specs>
/// and <https://m3.material.io/components/switch/specs>
pub const SWITCH_TOGGLE_DURATION_MS: u64 = 150;

/// A node's motion declarations, keyed by its schema id.
pub type NodeDeclaration = (String, Option<Transition>, Vec<NodeAnimation>);

/// Motion declarations in top-down document order. `transitions_only`
/// keeps just the nodes that declare a `transition`; otherwise a node
/// qualifies with a transition or at least one animation.
pub fn node_declarations(runtime: &Runtime, transitions_only: bool) -> Vec<NodeDeclaration> {
    let mut result = Vec::new();
    let Some(document) = runtime.document.as_ref() else {
        return result;
    };
    for key in document.tree.keys_top_down() {
        let node = &document.tree.nodes[key].schema;
        let (transition, animations) = node.motion_declarations();
        let transition = transition.cloned();
        let animations = animations.cloned().unwrap_or_default();
        if (transitions_only && transition.is_some())
            || (!transitions_only && (transition.is_some() || !animations.is_empty()))
        {
            result.push((node_schema_id(node).to_owned(), transition, animations));
        }
    }
    result
}

/// The `transition` a node declares, by schema id.
pub fn document_transition(runtime: &Runtime, id: &str) -> Option<Transition> {
    let document = runtime.document.as_ref()?;
    let key = document.tree.by_id.get(id).copied()?;
    let node = document.tree.nodes.get(key)?;
    node.schema.motion_declarations().0.cloned()
}

/// Duration and curve of a switch's on/off tween: its own `transition`,
/// else the Material 3 default.
pub fn toggle_timing(runtime: &Runtime, target: &str) -> (u64, Easing) {
    match document_transition(runtime, target) {
        Some(transition) => (transition.duration_ms, Easing::from(&transition.easing)),
        None => (SWITCH_TOGGLE_DURATION_MS, Easing::Standard),
    }
}

/// Whether the document has a switch, whose tween must be observed.
pub fn runtime_has_switch_widgets(runtime: &Runtime) -> bool {
    runtime.document.as_ref().is_some_and(|document| {
        document
            .tree
            .nodes
            .values()
            .any(|node| matches!(node.schema, PenNode::Switch(_)))
    })
}

/// One message per declared property the runtime will ignore because it is
/// outside the P1 compositor whitelist (layout properties would relayout).
pub fn load_warnings(document: &PenDocument) -> Vec<String> {
    let mut warnings = Vec::new();
    collect_load_warnings(&document.children, &mut warnings);
    if let Some(pages) = &document.pages {
        for page in pages {
            collect_load_warnings(&page.children, &mut warnings);
        }
    }
    warnings
}

fn collect_load_warnings(nodes: &[PenNode], warnings: &mut Vec<String>) {
    for node in nodes {
        let (transition, animations) = node.motion_declarations();
        if let Some(transition) = transition {
            for property in transition.properties.as_deref().into_iter().flatten() {
                if !P1_MOTION_PROPERTIES.contains(&property.as_str()) {
                    warnings.push(format!(
                        "motion property {property} on node {} is ignored for transition",
                        node_schema_id(node)
                    ));
                }
            }
        }
        if let Some(animations) = animations {
            for animation in animations {
                for keyframe in &animation.keyframes {
                    for property in keyframe.values.keys() {
                        if !P1_MOTION_PROPERTIES.contains(&property.as_str()) {
                            warnings.push(format!(
                                "motion property {property} on node {} is ignored for animation",
                                node_schema_id(node)
                            ));
                        }
                    }
                }
            }
        }
        if let Some(children) = node.children_ref() {
            collect_load_warnings(children, warnings);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(source: &str) -> Runtime {
        let document = jian_ops_schema::load_str(source).expect("document").value;
        Runtime::new_from_document(document).expect("runtime")
    }

    #[test]
    fn declarations_come_top_down_and_respect_the_transition_filter() {
        let runtime = runtime(
            r#"{"version":"1.1","children":[{"type":"frame","id":"root","width":10,"height":10,
                "transition":{"durationMs":120,"easing":"linear"},
                "children":[
                  {"type":"rectangle","id":"plain","width":1,"height":1},
                  {"type":"rectangle","id":"hero","width":1,"height":1,
                   "animations":[{"trigger":"mount","durationMs":100,
                     "keyframes":{"from":{"opacity":0},"to":{"opacity":1}}}]}]}]}"#,
        );
        let all: Vec<_> = node_declarations(&runtime, false)
            .into_iter()
            .map(|(id, transition, animations)| (id, transition.is_some(), animations.len()))
            .collect();
        assert_eq!(
            all,
            [("root".to_owned(), true, 0), ("hero".to_owned(), false, 1)]
        );
        let transitions: Vec<_> = node_declarations(&runtime, true)
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(transitions, ["root"]);
        assert_eq!(toggle_timing(&runtime, "root"), (120, Easing::Linear));
        assert_eq!(
            toggle_timing(&runtime, "plain"),
            (SWITCH_TOGGLE_DURATION_MS, Easing::Standard)
        );
        assert!(!runtime_has_switch_widgets(&runtime));
    }

    #[test]
    fn a_switch_is_detected() {
        let runtime = runtime(
            r#"{"version":"1.1","children":[{"type":"switch","id":"toggle","width":60,"height":32}]}"#,
        );
        assert!(runtime_has_switch_widgets(&runtime));
    }

    #[test]
    fn layout_properties_are_reported_for_transitions_and_animations() {
        let document = jian_ops_schema::load_str(
            r#"{"version":"1.1","children":[{"type":"frame","id":"root","width":10,"height":10,
                "transition":{"durationMs":100,"properties":["opacity","width"]},
                "children":[{"type":"rectangle","id":"hero","width":10,"height":10,
                  "animations":[{"trigger":"mount","durationMs":100,"keyframes":[
                    {"offset":0,"values":{"x":0,"opacity":0}},
                    {"offset":1,"values":{"x":10,"opacity":1}}]}]}]}]}"#,
        )
        .expect("motion document")
        .value;
        assert_eq!(
            load_warnings(&document),
            [
                "motion property width on node root is ignored for transition",
                "motion property x on node hero is ignored for animation",
                "motion property x on node hero is ignored for animation",
            ]
        );
    }
}
