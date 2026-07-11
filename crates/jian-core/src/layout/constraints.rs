use crate::document::{NodeKey, NodeTree};
use jian_ops_schema::constraints::{HConstraint, VConstraint};
use jian_ops_schema::node::{PenNode, PenNodeBase};
use jian_ops_schema::sizing::{SizeLimits, SizingBehavior};
use slotmap::SecondaryMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisRef {
    pub pos: f32,
    pub size: f32,
    pub parent_size: f32,
    pub min: Option<f32>,
    pub max: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeRef {
    pub h: Option<AxisRef>,
    pub v: Option<AxisRef>,
    pub h_kind: HConstraint,
    pub v_kind: VConstraint,
}

pub struct ReferenceTable {
    map: SecondaryMap<NodeKey, NodeRef>,
}

impl ReferenceTable {
    pub fn build(tree: &NodeTree) -> (Self, Vec<String>) {
        let mut map = SecondaryMap::new();
        let mut lints = Vec::new();

        for (key, data) in &tree.nodes {
            let Some(base) = node_base(&data.schema) else {
                continue;
            };
            let Some(constraints) = base.constraints else {
                continue;
            };
            let absolute = base.x.is_some() || base.y.is_some();
            let (width, height) = numeric_size(&data.schema);
            let (parent_width, parent_height) = data
                .parent
                .map(|parent| numeric_size(&tree.nodes[parent].schema))
                .unwrap_or((None, None));
            let mut ignored_limit_warnings = Vec::new();
            let limits = node_limits(&data.schema)
                .copied()
                .unwrap_or_default()
                .sanitized(&base.id, &mut ignored_limit_warnings);

            let mut axis = |name: &str,
                            pos: f32,
                            size: Option<f32>,
                            parent_size: Option<f32>,
                            min: Option<f64>,
                            max: Option<f64>| {
                if !absolute {
                    lints.push(format!(
                        "node `{}`: {name} constraint ignored; node is not absolutely positioned",
                        base.id
                    ));
                    return None;
                }
                let (Some(size), Some(parent_size)) = (size, parent_size) else {
                    lints.push(format!(
                        "node `{}`: {name} constraint ignored; node or parent size is not a plain non-negative number",
                        base.id
                    ));
                    return None;
                };
                Some(AxisRef {
                    pos,
                    size,
                    parent_size,
                    min: min.map(|value| value as f32),
                    max: max.map(|value| value as f32),
                })
            };

            let h = axis(
                "horizontal",
                base.x.unwrap_or(0.0) as f32,
                width,
                parent_width,
                limits.min_width,
                limits.max_width,
            );
            let v = axis(
                "vertical",
                base.y.unwrap_or(0.0) as f32,
                height,
                parent_height,
                limits.min_height,
                limits.max_height,
            );
            if h.as_ref()
                .is_some_and(|axis| constraints.h == HConstraint::Scale && axis.parent_size == 0.0)
            {
                lints.push(format!(
                    "node `{}`: horizontal scale constraint degrades to left because the authored parent size is zero",
                    base.id
                ));
            }
            if v.as_ref()
                .is_some_and(|axis| constraints.v == VConstraint::Scale && axis.parent_size == 0.0)
            {
                lints.push(format!(
                    "node `{}`: vertical scale constraint degrades to top because the authored parent size is zero",
                    base.id
                ));
            }
            map.insert(
                key,
                NodeRef {
                    h,
                    v,
                    h_kind: constraints.h,
                    v_kind: constraints.v,
                },
            );
        }
        (Self { map }, lints)
    }

    pub fn get(&self, key: NodeKey) -> Option<&NodeRef> {
        self.map.get(key)
    }
}

fn number(value: Option<&SizingBehavior>) -> Option<f32> {
    match value {
        Some(SizingBehavior::Number(value)) if *value >= 0.0 && value.is_finite() => {
            Some(*value as f32)
        }
        _ => None,
    }
}

fn numeric_size(node: &PenNode) -> (Option<f32>, Option<f32>) {
    match node {
        PenNode::Frame(node) => (
            number(node.container.width.as_ref()),
            number(node.container.height.as_ref()),
        ),
        PenNode::Group(node) => (
            number(node.container.width.as_ref()),
            number(node.container.height.as_ref()),
        ),
        PenNode::Rectangle(node) => (
            number(node.container.width.as_ref()),
            number(node.container.height.as_ref()),
        ),
        PenNode::Text(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::TextInput(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::IconFont(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Image(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Ellipse(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Path(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Polygon(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::TextArea(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Select(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Switch(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Checkbox(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Slider(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::RadioGroup(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::NumberInput(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Progress(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Tabs(node) => (number(node.width.as_ref()), number(node.height.as_ref())),
        PenNode::Line(_) | PenNode::Ref(_) => (None, None),
    }
}

fn node_base(node: &PenNode) -> Option<&PenNodeBase> {
    Some(match node {
        PenNode::Frame(node) => &node.base,
        PenNode::Group(node) => &node.base,
        PenNode::Rectangle(node) => &node.base,
        PenNode::Text(node) => &node.base,
        PenNode::TextInput(node) => &node.base,
        PenNode::IconFont(node) => &node.base,
        PenNode::Image(node) => &node.base,
        PenNode::Ellipse(node) => &node.base,
        PenNode::Line(node) => &node.base,
        PenNode::Path(node) => &node.base,
        PenNode::Polygon(node) => &node.base,
        PenNode::TextArea(node) => &node.base,
        PenNode::Select(node) => &node.base,
        PenNode::Switch(node) => &node.base,
        PenNode::Checkbox(node) => &node.base,
        PenNode::Slider(node) => &node.base,
        PenNode::RadioGroup(node) => &node.base,
        PenNode::NumberInput(node) => &node.base,
        PenNode::Progress(node) => &node.base,
        PenNode::Tabs(node) => &node.base,
        PenNode::Ref(node) => &node.base,
    })
}

fn node_limits(node: &PenNode) -> Option<&SizeLimits> {
    match node {
        PenNode::Frame(node) => Some(&node.container.limits),
        PenNode::Group(node) => Some(&node.container.limits),
        PenNode::Rectangle(node) => Some(&node.container.limits),
        PenNode::Text(node) => Some(&node.limits),
        PenNode::TextInput(node) => Some(&node.limits),
        PenNode::IconFont(node) => Some(&node.limits),
        PenNode::Image(node) => Some(&node.limits),
        PenNode::Ellipse(node) => Some(&node.limits),
        PenNode::Path(node) => Some(&node.limits),
        PenNode::Polygon(node) => Some(&node.limits),
        PenNode::TextArea(node) => Some(&node.limits),
        PenNode::Select(node) => Some(&node.limits),
        PenNode::Switch(node) => Some(&node.limits),
        PenNode::Checkbox(node) => Some(&node.limits),
        PenNode::Slider(node) => Some(&node.limits),
        PenNode::RadioGroup(node) => Some(&node.limits),
        PenNode::NumberInput(node) => Some(&node.limits),
        PenNode::Progress(node) => Some(&node.limits),
        PenNode::Tabs(node) => Some(&node.limits),
        PenNode::Line(_) | PenNode::Ref(_) => None,
    }
}

/// Resolve one constrained axis from immutable authored geometry.
pub fn resolve_axis(kind: HConstraint, reference: &AxisRef, parent_actual: f32) -> (f32, f32) {
    let kind = if kind == HConstraint::Scale && reference.parent_size == 0.0 {
        HConstraint::Left
    } else {
        kind
    };
    let left = reference.pos;
    let right_margin = reference.parent_size - reference.pos - reference.size;
    let center_offset = reference.pos + reference.size / 2.0 - reference.parent_size / 2.0;

    let (raw_pos, raw_size) = match kind {
        HConstraint::Left => (left, reference.size),
        HConstraint::Right => (
            parent_actual - right_margin - reference.size,
            reference.size,
        ),
        HConstraint::Center => (
            parent_actual / 2.0 + center_offset - reference.size / 2.0,
            reference.size,
        ),
        HConstraint::LeftRight => (left, parent_actual - left - right_margin),
        HConstraint::Scale => {
            let factor = parent_actual / reference.parent_size;
            (reference.pos * factor, reference.size * factor)
        }
    };

    let mut size = raw_size;
    if let Some(max) = reference.max {
        size = size.min(max);
    }
    if let Some(min) = reference.min {
        size = size.max(min);
    }
    size = size.max(0.0);

    let pos = match kind {
        HConstraint::Left => raw_pos,
        HConstraint::Right => parent_actual - right_margin - size,
        HConstraint::Center => parent_actual / 2.0 + center_offset - size / 2.0,
        HConstraint::Scale => {
            let factor = parent_actual / reference.parent_size;
            let scaled_center = (reference.pos + reference.size / 2.0) * factor;
            scaled_center - size / 2.0
        }
        HConstraint::LeftRight => {
            let slot_end = parent_actual - right_margin;
            left + ((slot_end - left) - size) / 2.0
        }
    };
    (pos, size)
}

pub fn vertical_as_h(kind: VConstraint) -> HConstraint {
    match kind {
        VConstraint::Top => HConstraint::Left,
        VConstraint::Bottom => HConstraint::Right,
        VConstraint::Center => HConstraint::Center,
        VConstraint::TopBottom => HConstraint::LeftRight,
        VConstraint::Scale => HConstraint::Scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_ops_schema::node::PenNode;

    fn tree_from_json(json: &str) -> NodeTree {
        let root: PenNode = serde_json::from_str(json).unwrap();
        let mut tree = NodeTree::new();
        tree.insert_subtree(root, None);
        tree
    }

    #[test]
    fn eligibility_requires_absolute_and_numeric_nonnegative_sizes() {
        let tree = tree_from_json(
            r#"{"type":"frame","id":"p","width":400,"height":300,
                "children":[{"type":"rectangle","id":"c","x":10,"y":10,
                "width":100,"height":50,"constraints":{"h":"right","v":"bottom"}}]}"#,
        );
        let (table, lints) = ReferenceTable::build(&tree);
        let child = table.get(tree.get("c").unwrap()).unwrap();
        assert!(child.h.is_some() && child.v.is_some());
        assert!(lints.is_empty());

        let tree = tree_from_json(
            r#"{"type":"frame","id":"p","width":"fit_content","height":300,
                "children":[{"type":"rectangle","id":"c","x":10,"y":10,
                "width":100,"height":50,"constraints":{"h":"right","v":"bottom"}}]}"#,
        );
        let (table, lints) = ReferenceTable::build(&tree);
        let child = table.get(tree.get("c").unwrap()).unwrap();
        assert!(child.h.is_none());
        assert!(child.v.is_some());
        assert_eq!(lints.len(), 1);
    }

    #[test]
    fn negative_node_or_parent_size_is_ineligible_per_axis() {
        let tree = tree_from_json(
            r#"{"type":"frame","id":"p","width":400,"height":-1,
                "children":[{"type":"rectangle","id":"c","x":10,"y":10,
                "width":-5,"height":50,"constraints":{"h":"right","v":"bottom"}}]}"#,
        );
        let (table, lints) = ReferenceTable::build(&tree);
        let child = table.get(tree.get("c").unwrap()).unwrap();
        assert!(child.h.is_none());
        assert!(child.v.is_none());
        assert_eq!(lints.len(), 2);
    }

    #[test]
    fn missing_coordinate_is_authored_zero_and_non_absolute_is_ineligible() {
        let tree = tree_from_json(
            r#"{"type":"frame","id":"p","width":400,"height":300,
                "children":[{"type":"rectangle","id":"c","y":10,
                "width":100,"height":50,"constraints":{"h":"right","v":"top"}}]}"#,
        );
        let (table, _) = ReferenceTable::build(&tree);
        assert_eq!(
            table.get(tree.get("c").unwrap()).unwrap().h.unwrap().pos,
            0.0
        );

        let tree = tree_from_json(
            r#"{"type":"frame","id":"p","width":400,"height":300,
                "children":[{"type":"rectangle","id":"c","width":100,"height":50,
                "constraints":{"h":"right","v":"top"}}]}"#,
        );
        let (table, lints) = ReferenceTable::build(&tree);
        let child = table.get(tree.get("c").unwrap()).unwrap();
        assert!(child.h.is_none() && child.v.is_none());
        assert_eq!(lints.len(), 2);
    }

    #[test]
    fn resolve_axis_matrix() {
        let reference = AxisRef {
            pos: 80.0,
            size: 30.0,
            parent_size: 100.0,
            min: None,
            max: None,
        };
        let cases = [
            (HConstraint::Left, (80.0, 30.0)),
            (HConstraint::Right, (180.0, 30.0)),
            (HConstraint::Center, (130.0, 30.0)),
            (HConstraint::LeftRight, (80.0, 130.0)),
            (HConstraint::Scale, (160.0, 60.0)),
        ];
        for (kind, expected) in cases {
            assert_eq!(resolve_axis(kind, &reference, 200.0), expected, "{kind:?}");
        }
    }

    #[test]
    fn clamp_is_max_first_min_last_then_anchor_rederived() {
        let reference = AxisRef {
            pos: 80.0,
            size: 30.0,
            parent_size: 100.0,
            min: None,
            max: Some(20.0),
        };
        assert_eq!(
            resolve_axis(HConstraint::Right, &reference, 100.0),
            (90.0, 20.0)
        );
        let contradictory = AxisRef {
            min: Some(50.0),
            ..reference
        };
        assert_eq!(
            resolve_axis(HConstraint::Right, &contradictory, 100.0),
            (60.0, 50.0)
        );
    }

    #[test]
    fn negative_slack_floors_at_zero_and_left_right_centers_clamped_box() {
        let reference = AxisRef {
            pos: 10.0,
            size: 80.0,
            parent_size: 100.0,
            min: None,
            max: None,
        };
        let (pos, size) = resolve_axis(HConstraint::LeftRight, &reference, 15.0);
        assert_eq!(size, 0.0);
        assert!((pos - 7.5).abs() < 1e-5);
    }

    #[test]
    fn scale_with_zero_parent_degrades_to_left() {
        let reference = AxisRef {
            pos: 10.0,
            size: 20.0,
            parent_size: 0.0,
            min: None,
            max: None,
        };
        assert_eq!(
            resolve_axis(HConstraint::Scale, &reference, 300.0),
            (10.0, 20.0)
        );
    }
}
