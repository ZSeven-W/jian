//! Out-of-line `Clone` and `PartialEq` for [`PenNode`].
//!
//! The derived impls are `#[inline]`, so every crate that clones or compares a
//! node codegens its own copy of the whole 21-variant body (the web bundle
//! carried nine `PenNode::clone`s). These impls are the derive's exact
//! semantics, kept `#[inline(never)]` so the one copy lives in this crate.

use super::PenNode;

macro_rules! pen_node_glue {
    ($($variant:ident),+ $(,)?) => {
        impl Clone for PenNode {
            #[inline(never)]
            fn clone(&self) -> Self {
                match self {
                    $(PenNode::$variant(node) => PenNode::$variant(node.clone()),)+
                }
            }
        }

        impl PartialEq for PenNode {
            #[inline(never)]
            fn eq(&self, other: &Self) -> bool {
                match (self, other) {
                    $((PenNode::$variant(a), PenNode::$variant(b)) => a == b,)+
                    _ => false,
                }
            }
        }
    };
}

pen_node_glue!(
    Frame,
    Group,
    Rectangle,
    Ellipse,
    Line,
    Polygon,
    Path,
    Text,
    TextInput,
    Image,
    IconFont,
    TextArea,
    Select,
    Switch,
    Checkbox,
    Slider,
    RadioGroup,
    NumberInput,
    Progress,
    Tabs,
    Ref,
);
