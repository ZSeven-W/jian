//! Image-source prepass in the historical Value visitor's insertion order.
//!
//! That visitor used a LIFO node stack, so sibling and page sources entered the
//! preserve-order image map in reverse tree order. This bounded typed walk
//! preserves the same table order without rebuilding a JSON tree.

use super::SaveCollector;
use crate::node::PenNode;
use crate::state_override::{StyleOverride, WidgetStates};
use crate::style::{PenFill, PenStroke};
use crate::PenDocument;

/// Supplies image sources in the insertion order used by the legacy
/// `Value`-based externalizer. Serializable document views (for example a
/// structurally shared save snapshot) implement this without materializing an
/// owned [`PenDocument`].
pub trait SaveImageOrder {
    fn visit_save_image_sources(&self, visit: &mut dyn FnMut(&crate::node::ImageSrc));
}

impl SaveImageOrder for PenDocument {
    fn visit_save_image_sources(&self, visit: &mut dyn FnMut(&crate::node::ImageSrc)) {
        // PenDocument serializes pages before top-level children. The old
        // visitor pushed both arrays in map order, then consumed the stack
        // LIFO: roots first, followed by pages in reverse order.
        visit_legacy_node_roots(self.children.iter(), visit);
        if let Some(pages) = self.pages.as_deref() {
            for page in pages.iter().rev() {
                visit_legacy_node_roots(page.children.iter(), visit);
            }
        }
    }
}

pub(super) fn prepare<D: SaveImageOrder + ?Sized>(document: &D, collector: &mut SaveCollector) {
    document.visit_save_image_sources(&mut |source| {
        collector.record_source(source);
    });
}

/// Walk one root-node group in the legacy externalizer's LIFO order.
///
/// This is public so allocation-bounded document views can implement
/// [`SaveImageOrder`] while sharing the schema's authoritative typed-source
/// traversal.
#[doc(hidden)]
pub fn visit_legacy_node_roots<'a>(
    nodes: impl IntoIterator<Item = &'a PenNode>,
    visit: &mut dyn FnMut(&crate::node::ImageSrc),
) {
    let mut stack: Vec<&PenNode> = nodes.into_iter().collect();
    while let Some(node) = stack.pop() {
        let children = visit_own_sources(node, visit);
        if let Some(children) = children {
            stack.extend(children.iter());
        }
    }
}

fn visit_own_sources<'a>(
    node: &'a PenNode,
    visit: &mut dyn FnMut(&crate::node::ImageSrc),
) -> Option<&'a [PenNode]> {
    macro_rules! style {
        ($node:expr) => {{
            visit_fills($node.fill.as_deref(), visit);
            visit_stroke($node.stroke.as_ref(), visit);
        }};
    }
    macro_rules! widget_style {
        ($node:expr) => {{
            style!($node);
            visit_states($node.states.as_ref(), visit);
        }};
    }

    match node {
        PenNode::Frame(node) => {
            visit_fills(node.container.fill.as_deref(), visit);
            visit_stroke(node.container.stroke.as_ref(), visit);
            node.children.as_deref()
        }
        PenNode::Group(node) => {
            visit_fills(node.container.fill.as_deref(), visit);
            visit_stroke(node.container.stroke.as_ref(), visit);
            node.children.as_deref()
        }
        PenNode::Rectangle(node) => {
            visit_fills(node.container.fill.as_deref(), visit);
            visit_stroke(node.container.stroke.as_ref(), visit);
            node.children.as_deref()
        }
        PenNode::Ellipse(node) => {
            style!(node);
            None
        }
        PenNode::Line(node) => {
            visit_stroke(node.stroke.as_ref(), visit);
            None
        }
        PenNode::Polygon(node) => {
            style!(node);
            None
        }
        PenNode::Path(node) => {
            style!(node);
            None
        }
        PenNode::Text(node) => {
            visit_fills(node.fill.as_deref(), visit);
            None
        }
        PenNode::TextInput(node) => {
            widget_style!(node);
            None
        }
        PenNode::Image(node) => {
            visit(&node.src);
            None
        }
        PenNode::IconFont(node) => {
            style!(node);
            None
        }
        PenNode::TextArea(node) => {
            widget_style!(node);
            None
        }
        PenNode::Select(node) => {
            widget_style!(node);
            None
        }
        PenNode::Switch(node) => {
            widget_style!(node);
            None
        }
        PenNode::Checkbox(node) => {
            widget_style!(node);
            None
        }
        PenNode::Slider(node) => {
            widget_style!(node);
            None
        }
        PenNode::RadioGroup(node) => {
            widget_style!(node);
            None
        }
        PenNode::NumberInput(node) => {
            widget_style!(node);
            None
        }
        PenNode::Progress(node) => {
            widget_style!(node);
            None
        }
        PenNode::Tabs(node) => {
            widget_style!(node);
            node.children.as_deref()
        }
        PenNode::Ref(node) => node.children.as_deref(),
    }
}

fn visit_fills(fills: Option<&[PenFill]>, visit: &mut dyn FnMut(&crate::node::ImageSrc)) {
    for fill in fills.into_iter().flatten() {
        if let PenFill::Image(image) = fill {
            visit(&image.url);
        }
    }
}

fn visit_stroke(stroke: Option<&PenStroke>, visit: &mut dyn FnMut(&crate::node::ImageSrc)) {
    if let Some(stroke) = stroke {
        visit_fills(stroke.fill.as_deref(), visit);
    }
}

fn visit_states(states: Option<&WidgetStates>, visit: &mut dyn FnMut(&crate::node::ImageSrc)) {
    let Some(states) = states else {
        return;
    };
    for state in [
        states.hover.as_ref(),
        states.pressed.as_ref(),
        states.focused.as_ref(),
        states.disabled.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        visit_override(state, visit);
    }
}

fn visit_override(state: &StyleOverride, visit: &mut dyn FnMut(&crate::node::ImageSrc)) {
    visit_fills(state.fill.as_deref(), visit);
    visit_stroke(state.stroke.as_ref(), visit);
}
