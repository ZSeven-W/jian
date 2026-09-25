//! Parity between the hand-written `PenNode` decoder and the serde derive it
//! replaces: the reference enum below is exactly the old derived shape.

use super::super::*;
use reference::PenNode as Reference;
use serde::Deserialize;

mod reference {
    use super::super::super::*;
    use serde::Deserialize;

    /// Named `PenNode` so derived error messages match word for word.
    #[derive(Deserialize)]
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
        TextInput(TextInputNode),
        Image(ImageNode),
        IconFont(IconFontNode),
        TextArea(TextAreaNode),
        Select(SelectNode),
        Switch(SwitchNode),
        Checkbox(CheckboxNode),
        Slider(SliderNode),
        RadioGroup(RadioGroupNode),
        NumberInput(NumberInputNode),
        Progress(ProgressNode),
        Tabs(TabsNode),
        Ref(RefNode),
    }
}

impl From<Reference> for PenNode {
    fn from(r: Reference) -> Self {
        match r {
            Reference::Frame(n) => PenNode::Frame(n),
            Reference::Group(n) => PenNode::Group(n),
            Reference::Rectangle(n) => PenNode::Rectangle(n),
            Reference::Ellipse(n) => PenNode::Ellipse(n),
            Reference::Line(n) => PenNode::Line(n),
            Reference::Polygon(n) => PenNode::Polygon(n),
            Reference::Path(n) => PenNode::Path(n),
            Reference::Text(n) => PenNode::Text(n),
            Reference::TextInput(n) => PenNode::TextInput(n),
            Reference::Image(n) => PenNode::Image(n),
            Reference::IconFont(n) => PenNode::IconFont(n),
            Reference::TextArea(n) => PenNode::TextArea(n),
            Reference::Select(n) => PenNode::Select(n),
            Reference::Switch(n) => PenNode::Switch(n),
            Reference::Checkbox(n) => PenNode::Checkbox(n),
            Reference::Slider(n) => PenNode::Slider(n),
            Reference::RadioGroup(n) => PenNode::RadioGroup(n),
            Reference::NumberInput(n) => PenNode::NumberInput(n),
            Reference::Progress(n) => PenNode::Progress(n),
            Reference::Tabs(n) => PenNode::Tabs(n),
            Reference::Ref(n) => PenNode::Ref(n),
        }
    }
}

const CASES: &[&str] = &[
    // Every variant, including container children of mixed kinds.
    r#"{"type":"frame","id":"f","width":200,"height":"fill_container","layout":"vertical","gap":4,
        "children":[{"type":"text","id":"t","content":"hi","fontWeight":600},
                    {"id":"g","type":"group","children":[{"type":"ellipse","id":"e","startAngle":0,"sweepAngle":90}]},
                    {"type":"rectangle","id":"r","children":[],"cornerRadius":[1,2,3,4]}]}"#,
    r#"{"type":"line","id":"l","x":1,"y":2}"#,
    r#"{"type":"polygon","id":"p","polygonCount":5}"#,
    r#"{"type":"path","id":"p","d":"M0 0L1 1","anchors":[{"x":0.0,"y":0.0,"handleIn":null,"handleOut":null}]}"#,
    r#"{"type":"text_input","id":"i","placeholder":"x","value":""}"#,
    r#"{"type":"image","id":"i","src":"https://example.com/x.png","exposure":10.0}"#,
    r#"{"type":"icon_font","id":"i","iconFontName":"heart"}"#,
    r#"{"type":"text_area","id":"a","maxVisibleLines":4}"#,
    r#"{"type":"select","id":"s","options":[{"value":"a","label":"A"}]}"#,
    r#"{"type":"switch","id":"w","checked":true}"#,
    r#"{"type":"checkbox","id":"c"}"#,
    r#"{"type":"slider","id":"l","min":0,"max":100,"step":5,"value":40}"#,
    r#"{"type":"radio_group","id":"rg","value":"a","options":[{"value":"a","label":"A"}]}"#,
    r#"{"type":"number_input","id":"ni","min":0,"max":10,"step":1,"value":3}"#,
    r#"{"type":"progress","id":"pg","value":40,"max":100}"#,
    r#"{"type":"tabs","id":"tb","value":"one","tabs":[{"value":"one","label":"One"}],"children":[{"type":"frame","id":"p1"}]}"#,
    r#"{"type":"ref","id":"r","ref":"btn","descendants":{"label":{"content":"OK"}},"children":[{"type":"text","id":"t","content":"x"}]}"#,
    // `children` on a leaf is an ignored unknown key; null children is None.
    r#"{"type":"text","id":"t","content":"x","children":[{"type":"nope"}]}"#,
    r#"{"type":"frame","id":"f","children":null}"#,
    // Repeated keys: unknown ones are ignored, known ones are errors.
    r#"{"type":"text","id":"t","zz":1,"zz":2,"content":"x"}"#,
    r#"{"type":"text","id":"a","id":"b","content":"x"}"#,
    r#"{"type":"frame","id":"f","children":[],"children":[]}"#,
    r#"{"type":"frame","id":"f","type":"frame"}"#,
    // Errors.
    r#"{"id":"f"}"#,
    r#"{"type":"blob","id":"f"}"#,
    r#"{"type":7,"id":"t","content":"x"}"#,
    r#"{"type":-1,"id":"f"}"#,
    r#"{"type":true,"id":"f"}"#,
    r#"{"type":null,"id":"f"}"#,
    r#"{"type":"text","id":"t"}"#,
    r#"{"type":"frame","id":"f","children":{"a":1}}"#,
    r#"{"type":"frame","id":"f","children":"nope"}"#,
    r#"{"type":"frame","id":"f","children":[{"type":"text","id":"t"}]}"#,
    r#"{"type":"frame","id":"f","children":[{"type":"frame","id":"g","children":[{"id":"x"}]}]}"#,
    r#"{"type":"frame","id":"f","children":[null]}"#,
    r#"{"type":"frame","id":"f","width":"huge"}"#,
    r#"null"#,
    r#"7"#,
    r#""frame""#,
    r#"[]"#,
    r#"["frame"]"#,
    r#"["frame",{"id":"x"}]"#,
];

fn via_reference_str(src: &str) -> Result<PenNode, String> {
    serde_json::from_str::<Reference>(src)
        .map(PenNode::from)
        .map_err(|e| e.to_string())
}

fn via_reference_value(src: &str) -> Result<PenNode, String> {
    let value: serde_json::Value = serde_json::from_str(src).unwrap();
    serde_json::from_value::<Reference>(value)
        .map(PenNode::from)
        .map_err(|e| e.to_string())
}

#[test]
fn from_str_matches_the_derive() {
    for src in CASES {
        let ours = serde_json::from_str::<PenNode>(src).map_err(|e| e.to_string());
        assert_eq!(ours, via_reference_str(src), "input: {src}");
    }
}

#[test]
fn from_value_matches_the_derive() {
    for src in CASES {
        let value: serde_json::Value = serde_json::from_str(src).unwrap();
        let ours = serde_json::from_value::<PenNode>(value).map_err(|e| e.to_string());
        assert_eq!(ours, via_reference_value(src), "input: {src}");
    }
}

#[test]
fn nested_vec_in_a_foreign_struct_matches_the_derive() {
    #[derive(Deserialize)]
    struct Holder {
        nodes: Vec<PenNode>,
    }
    #[derive(Deserialize)]
    struct RefHolder {
        nodes: Vec<Reference>,
    }
    let decode = |src: &str| {
        let ours = serde_json::from_str::<Holder>(src)
            .map(|h| h.nodes)
            .map_err(|e| e.to_string());
        let reference = serde_json::from_str::<RefHolder>(src)
            .map(|h| h.nodes.into_iter().map(PenNode::from).collect::<Vec<_>>())
            .map_err(|e| e.to_string());
        assert_eq!(ours, reference, "input: {src}");
    };
    decode(&format!("{{\"nodes\":[{}]}}", CASES[..3].join(",")));
    // Error positions inside a surrounding document match too.
    for bad in CASES
        .iter()
        .filter(|c| serde_json::from_str::<PenNode>(c).is_err())
    {
        decode(&format!("{{\"nodes\":[{},\n  {bad}]}}", CASES[1]));
    }
}

#[test]
fn deep_trees_decode_without_native_recursion() {
    use super::super::buf::Buf;
    let depth = 20_000;
    let field = |k: &str, v: Buf| (k.to_owned(), v);
    let mut node = Buf::Map(vec![
        field("type", Buf::Str("text".into())),
        field("id", Buf::Str("leaf".into())),
        field("content", Buf::Str("x".into())),
    ]);
    for i in 0..depth {
        node = Buf::Map(vec![
            field("type", Buf::Str("frame".into())),
            field("id", Buf::Str(format!("f{i}"))),
            field("children", Buf::Seq(vec![node])),
        ]);
    }
    let Buf::Map(mut fields) = node else {
        unreachable!()
    };
    fields.remove(0);
    // Run on a small stack: decoding must not recurse per level. The result
    // is leaked below because dropping a `PenNode` tree does recurse.
    let node = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || super::decode_tree(0, Buf::Map(fields)).expect("deep tree decodes"))
        .unwrap()
        .join()
        .unwrap();
    let mut levels = 0;
    let mut cur = &node;
    while let Some(children) = cur.children_ref() {
        levels += 1;
        cur = &children[0];
    }
    assert_eq!(levels, depth);
    std::mem::forget(node);
}
