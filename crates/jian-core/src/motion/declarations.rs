//! What a document declares about motion, read straight from the schema.

use crate::document::tree::node_schema_id;
use jian_ops_schema::motion::P1_MOTION_PROPERTIES;
use jian_ops_schema::node::PenNode;
use jian_ops_schema::PenDocument;

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
