//! Hand-written `Deserialize` for [`PenNode`].
//!
//! The derived impl of an internally tagged enum is generic over the
//! deserializer, and stable rustc does not share generic instances across
//! crates in optimized builds: every crate that decodes a `PenNode` (or any
//! type containing one) through a concrete deserializer monomorphized its own
//! copy of all 21 variant visitors. The web bundle carried eight such copies.
//!
//! Here the generic surface is only "read the tag and buffer the fields" (the
//! derive buffers each node too, into serde's private `Content`). The fields
//! land in the concrete [`Buf`], and every variant visitor runs in the
//! non-generic `decode_tree`, compiled once in this crate.
//!
//! Subtrees are decoded with an explicit stack: the `children` array is lifted
//! off each container before its struct is decoded, so nested nodes are never
//! re-buffered and decoding does not recurse per level.
//!
//! Error behaviour follows the derive: a missing tag is ``missing field
//! `type` ``, an unknown one lists the snake_case variants, a non-string tag
//! is not a variant identifier, non-map input is rejected as an "internally
//! tagged enum PenNode", and repeated keys and error positions behave as
//! before (`de_tests.rs` checks all of this against a reference derive).

use super::buf::Buf;
use super::{
    CheckboxNode, EllipseNode, FrameNode, GroupNode, IconFontNode, ImageNode, LineNode,
    NumberInputNode, PathNode, PenNode, PolygonNode, ProgressNode, RadioGroupNode, RectangleNode,
    RefNode, SelectNode, SliderNode, SwitchNode, TabsNode, TextAreaNode, TextInputNode, TextNode,
};
use serde::de::{Deserialize, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::Error;
use std::fmt;

/// JSON tags, in `PenNode` declaration order.
const VARIANTS: &[&str] = &[
    "frame",
    "group",
    "rectangle",
    "ellipse",
    "line",
    "polygon",
    "path",
    "text",
    "text_input",
    "image",
    "icon_font",
    "text_area",
    "select",
    "switch",
    "checkbox",
    "slider",
    "radio_group",
    "number_input",
    "progress",
    "tabs",
    "ref",
];

const EXPECTING: &str = "internally tagged enum PenNode";

impl<'de> Deserialize<'de> for PenNode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Same split as the derive: the tag is read and checked inside the
        // visitor (a positional format reports a bad tag at the tag), the
        // fields are decoded after the node has been buffered.
        let (variant, body) = deserializer.deserialize_any(TagVisitor)?;
        decode_tree(variant, body).map_err(D::Error::custom)
    }
}

/// Buffers one node: its variant index plus the remaining fields (a map, or
/// the values of the derive's `[tag, ...]` array form).
struct TagVisitor;

impl<'de> Visitor<'de> for TagVisitor {
    type Value = (usize, Buf);

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(EXPECTING)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut variant = None;
        let mut fields = Vec::with_capacity(access.size_hint().unwrap_or(0).min(4096));
        while let Some(key) = access.next_key::<String>()? {
            if key == "type" {
                if variant.is_some() {
                    return Err(A::Error::duplicate_field("type"));
                }
                let tag: Buf = access.next_value()?;
                variant = Some(variant_index(&tag).map_err(A::Error::custom)?);
            } else {
                fields.push((key, access.next_value::<Buf>()?));
            }
        }
        let variant = variant.ok_or_else(|| A::Error::missing_field("type"))?;
        Ok((variant, Buf::Map(fields)))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let tag: Buf = access
            .next_element()?
            .ok_or_else(|| A::Error::missing_field("type"))?;
        let variant = variant_index(&tag).map_err(A::Error::custom)?;
        let mut rest = Vec::new();
        while let Some(item) = access.next_element::<Buf>()? {
            rest.push(item);
        }
        Ok((variant, Buf::Seq(rest)))
    }
}

impl PenNode {
    fn children_slot(&mut self) -> Option<&mut Option<Vec<PenNode>>> {
        match self {
            PenNode::Frame(n) => Some(&mut n.children),
            PenNode::Group(n) => Some(&mut n.children),
            PenNode::Rectangle(n) => Some(&mut n.children),
            PenNode::Tabs(n) => Some(&mut n.children),
            PenNode::Ref(n) => Some(&mut n.children),
            _ => None,
        }
    }
}

/// A node decoded without its `children`, plus that undecoded array.
type Shallow = (PenNode, Option<Vec<Buf>>);

/// Decode a buffered node and its whole subtree, with an explicit stack
/// instead of recursion.
#[inline(never)]
fn decode_tree(variant: usize, body: Buf) -> Result<PenNode, Error> {
    let (node, children) = decode_parts(variant, body)?;
    let mut stack = vec![Pending::new(node, children)];
    loop {
        let top = stack.last_mut().expect("decode stack is never empty here");
        if let Some(child) = top.children.as_mut().and_then(Iterator::next) {
            let (node, children) = decode_child(child)?;
            stack.push(Pending::new(node, children));
            continue;
        }
        let node = stack.pop().expect("top frame exists").finish();
        match stack.last_mut() {
            Some(parent) => parent.built.push(node),
            None => return Ok(node),
        }
    }
}

/// A decoded container whose lifted `children` array is still being decoded.
struct Pending {
    node: PenNode,
    children: Option<std::vec::IntoIter<Buf>>,
    built: Vec<PenNode>,
}

impl Pending {
    fn new(node: PenNode, children: Option<Vec<Buf>>) -> Self {
        let built = Vec::with_capacity(children.as_ref().map_or(0, Vec::len));
        Pending {
            node,
            children: children.map(Vec::into_iter),
            built,
        }
    }

    fn finish(self) -> PenNode {
        let Pending {
            mut node,
            children,
            built,
        } = self;
        if children.is_some() {
            if let Some(slot) = node.children_slot() {
                *slot = Some(built);
            }
        }
        node
    }
}

/// Decode one lifted child: find its tag the way [`TagVisitor`] does, then
/// its fields (leaving its own children undecoded).
fn decode_child(child: Buf) -> Result<Shallow, Error> {
    match child {
        Buf::Map(entries) => {
            let mut variant = None;
            let mut fields = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                if key == "type" {
                    if variant.is_some() {
                        return Err(Error::duplicate_field("type"));
                    }
                    variant = Some(variant_index(&value)?);
                } else {
                    fields.push((key, value));
                }
            }
            let variant = variant.ok_or_else(|| Error::missing_field("type"))?;
            decode_parts(variant, Buf::Map(fields))
        }
        Buf::Seq(mut items) => {
            if items.is_empty() {
                return Err(Error::missing_field("type"));
            }
            let variant = variant_index(&items.remove(0))?;
            decode_parts(variant, Buf::Seq(items))
        }
        other => Err(Error::invalid_type(other.unexpected(), &EXPECTING)),
    }
}

/// Decode a node's fields. A container's `children` array is lifted off and
/// returned undecoded; any other `children` shape (or a repeated key) stays
/// in place so the struct reports it exactly as the derive did.
fn decode_parts(variant: usize, body: Buf) -> Result<Shallow, Error> {
    let (body, children) = match body {
        Buf::Map(mut fields) if matches!(variant, 0 | 1 | 2 | 19 | 20) => {
            let children = take_children(&mut fields);
            (Buf::Map(fields), children)
        }
        other => (other, None),
    };
    Ok((decode_variant(variant, body)?, children))
}

fn take_children(fields: &mut Vec<(String, Buf)>) -> Option<Vec<Buf>> {
    let mut found = None;
    for (index, (key, _)) in fields.iter().enumerate() {
        if key == "children" {
            if found.is_some() {
                return None;
            }
            found = Some(index);
        }
    }
    let index = found?;
    if !matches!(fields[index].1, Buf::Seq(_)) {
        return None;
    }
    match fields.remove(index).1 {
        Buf::Seq(items) => Some(items),
        _ => None,
    }
}

/// The derive reads an internally tagged enum's tag as a variant *name* only
/// (an integer is not a variant identifier here), so this does too.
fn variant_index(tag: &Buf) -> Result<usize, Error> {
    match tag {
        Buf::Str(name) => VARIANTS
            .iter()
            .position(|v| v == name)
            .ok_or_else(|| Error::unknown_variant(name, VARIANTS)),
        other => Err(Error::invalid_type(
            other.unexpected(),
            &"variant identifier",
        )),
    }
}

fn decode_variant(variant: usize, body: Buf) -> Result<PenNode, Error> {
    Ok(match variant {
        0 => PenNode::Frame(FrameNode::deserialize(body)?),
        1 => PenNode::Group(GroupNode::deserialize(body)?),
        2 => PenNode::Rectangle(RectangleNode::deserialize(body)?),
        3 => PenNode::Ellipse(EllipseNode::deserialize(body)?),
        4 => PenNode::Line(LineNode::deserialize(body)?),
        5 => PenNode::Polygon(PolygonNode::deserialize(body)?),
        6 => PenNode::Path(PathNode::deserialize(body)?),
        7 => PenNode::Text(TextNode::deserialize(body)?),
        8 => PenNode::TextInput(TextInputNode::deserialize(body)?),
        9 => PenNode::Image(ImageNode::deserialize(body)?),
        10 => PenNode::IconFont(IconFontNode::deserialize(body)?),
        11 => PenNode::TextArea(TextAreaNode::deserialize(body)?),
        12 => PenNode::Select(SelectNode::deserialize(body)?),
        13 => PenNode::Switch(SwitchNode::deserialize(body)?),
        14 => PenNode::Checkbox(CheckboxNode::deserialize(body)?),
        15 => PenNode::Slider(SliderNode::deserialize(body)?),
        16 => PenNode::RadioGroup(RadioGroupNode::deserialize(body)?),
        17 => PenNode::NumberInput(NumberInputNode::deserialize(body)?),
        18 => PenNode::Progress(ProgressNode::deserialize(body)?),
        19 => PenNode::Tabs(TabsNode::deserialize(body)?),
        20 => PenNode::Ref(RefNode::deserialize(body)?),
        _ => unreachable!("variant_index bounds the index"),
    })
}

#[cfg(test)]
#[path = "de_tests.rs"]
mod tests;
