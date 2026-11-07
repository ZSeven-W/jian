//! Parent-aware fill-text sizing without changing legacy start-row overflow.

use super::resolve;
use jian_ops_schema::node::container::{JustifyContent, LayoutMode};
use jian_ops_schema::node::{PenNode, TextGrowth};
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};
use taffy::prelude::*;

pub(super) fn legacy_style(node: &PenNode, parent: Option<&PenNode>) -> Style {
    let mut style = resolve::node_to_style(node);
    let PenNode::Text(text) = node else {
        return style;
    };
    // Authored minimums, positioning and constraints retain legacy measurement.
    if text.limits.min_width.is_some()
        || text.base.x.is_some()
        || text.base.y.is_some()
        || text.base.constraints.is_some()
    {
        return style;
    }
    if parent.is_some_and(supports_legacy_fill_shrink) {
        apply_min_width(&mut style, node);
    }
    style
}

fn supports_legacy_fill_shrink(parent: &PenNode) -> bool {
    let container = match parent {
        PenNode::Frame(node) => &node.container,
        PenNode::Group(node) => &node.container,
        PenNode::Rectangle(node) => &node.container,
        _ => return false,
    };
    let allocates_width = match container.width.as_ref() {
        Some(SizingBehavior::Number(width)) => width.is_finite() && *width > 0.0,
        Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)) => true,
        _ => false,
    };
    allocates_width
        && matches!(container.layout, None | Some(LayoutMode::Horizontal))
        && matches!(
            container.justify_content,
            Some(
                JustifyContent::Center
                    | JustifyContent::End
                    | JustifyContent::SpaceBetween
                    | JustifyContent::SpaceAround
            )
        )
}

/// Wrapping text may yield to fixed siblings. Apply before responsive bounds
/// so an explicit minimum still wins; no-wrap text retains visible overflow.
pub(super) fn apply_min_width(style: &mut Style, node: &PenNode) {
    let PenNode::Text(text) = node else {
        return;
    };
    if text.text_growth == Some(TextGrowth::FixedWidth)
        && matches!(
            text.width,
            Some(SizingBehavior::Keyword(SizingKeyword::FillContainer))
        )
    {
        style.min_size.width = length(0.0_f32);
    }
}
