pub mod base;
pub mod container;

pub mod ellipse;
pub mod frame;
pub mod icon_font;
pub mod image;
pub mod line;
pub mod path;
pub mod polygon;
pub mod ref_node;
pub mod text;

pub use base::{BoolOrExpression, NumberOrExpression, PenNodeBase};
pub use container::{
    AlignItems, ContainerProps, CornerRadius, JustifyContent, LayoutMode, Padding,
};

pub use ellipse::EllipseNode;
pub use frame::{FrameNode, GroupNode, RectangleNode};
pub use icon_font::IconFontNode;
pub use image::{ImageFitMode, ImageNode};
pub use line::LineNode;
pub use path::{PathNode, PenPathAnchor, PenPathHandle, PenPathPointType};
pub use polygon::PolygonNode;
pub use ref_node::{DescendantOverrides, RefNode};
pub use text::{
    FontStyleKind as TextFontStyle, FontWeight, TextAlign, TextAlignVertical, TextContent,
    TextGrowth, TextNode,
};

use serde::{Deserialize, Serialize};

/// Union of all concrete node types.
/// Tag is the JSON `"type"` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PenNode {
    Frame(FrameNode),
    Group(GroupNode),
    Rectangle(RectangleNode),
    Ellipse(EllipseNode),
    Line(LineNode),
    Polygon(PolygonNode),
    Path(PathNode),
    Text(TextNode),
    Image(ImageNode),
    IconFont(IconFontNode),
    Ref(RefNode),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let json = r#"{"type":"frame","id":"f1","width":200,"height":100}"#;
        let n: PenNode = serde_json::from_str(json).unwrap();
        assert!(matches!(n, PenNode::Frame(_)));
    }

    #[test]
    fn nested_frame_with_children() {
        let json = r#"{"type":"frame","id":"root","children":[{"type":"text","id":"t","content":"hi"}]}"#;
        let n: PenNode = serde_json::from_str(json).unwrap();
        if let PenNode::Frame(f) = n {
            let children = f.children.unwrap();
            assert_eq!(children.len(), 1);
            assert!(matches!(children[0], PenNode::Text(_)));
        } else {
            panic!();
        }
    }

    #[test]
    fn ellipse_roundtrip() {
        let json = r#"{"type":"ellipse","id":"e","startAngle":0.0,"sweepAngle":90.0}"#;
        let n: PenNode = serde_json::from_str(json).unwrap();
        assert!(matches!(n, PenNode::Ellipse(_)));
    }

    #[test]
    fn path_with_anchors() {
        let json = r#"{"type":"path","id":"p","d":"M 0 0 L 10 10","anchors":[{"x":0.0,"y":0.0,"handleIn":null,"handleOut":null}]}"#;
        let n: PenNode = serde_json::from_str(json).unwrap();
        if let PenNode::Path(p) = n {
            assert_eq!(p.anchors.as_ref().unwrap().len(), 1);
        } else {
            panic!();
        }
    }

    #[test]
    fn image_with_filters() {
        let json = r#"{"type":"image","id":"i","src":"https://example.com/x.png","exposure":10.0}"#;
        let n: PenNode = serde_json::from_str(json).unwrap();
        assert!(matches!(n, PenNode::Image(_)));
    }

    #[test]
    fn ref_node_with_descendants() {
        let json = r#"{"type":"ref","id":"r","ref":"button-primary","descendants":{"label":{"content":"OK"}}}"#;
        let n: PenNode = serde_json::from_str(json).unwrap();
        if let PenNode::Ref(r) = n {
            assert_eq!(r.target, "button-primary");
            let d = r.descendants.as_ref().unwrap();
            assert!(d.contains_key("label"));
        } else {
            panic!();
        }
    }
}
