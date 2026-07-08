//! Explicit-marker promotion of legacy frames to widget nodes.
//!
//! Decision record 2026-06-13 #1: ONLY explicit markers are honoured —
//! `base.role` strings from the AI role vocabulary, or
//! `semantics.role == Input`. Names and looks are never guessed.
//!
//! Three callers share this single implementation:
//! 1. `compat::load_str_with(promote_legacy_widgets: true)` — in-memory,
//!    used by runtime hosts; the file is not rewritten.
//! 2. OpenPencil's "semantic upgrade" editor command — persists the
//!    rewrite and reports every change.
//! 3. The AI pipeline normalisation pass after `batch_design`.

use crate::document::PenDocument;
use crate::node::text::TextContent;
use crate::node::{
    CheckboxNode, FrameNode, NumberInputNode, NumberOrExpression, PenNode, ProgressNode,
    RadioGroupNode, SelectNode, SelectOption, SliderNode, SwitchNode, TextAreaNode, TextInputNode,
};
use crate::semantics::SemanticRole;
use crate::style::PenFill;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetKind {
    TextInput,
    TextArea,
    Select,
    Switch,
    Checkbox,
    Slider,
    RadioGroup,
    NumberInput,
    Progress,
}

impl WidgetKind {
    pub fn tag(self) -> &'static str {
        match self {
            WidgetKind::TextInput => "text_input",
            WidgetKind::TextArea => "text_area",
            WidgetKind::Select => "select",
            WidgetKind::Switch => "switch",
            WidgetKind::Checkbox => "checkbox",
            WidgetKind::Slider => "slider",
            WidgetKind::RadioGroup => "radio_group",
            WidgetKind::NumberInput => "number_input",
            WidgetKind::Progress => "progress",
        }
    }
}

/// Explicit `base.role` → widget-kind table. Single source of truth:
/// `role_to_kind` and `promotable_roles` both derive from it. Aliases
/// come from the AI role vocabulary (role-definitions.md) — extend
/// ONLY when that vocabulary grows, never with name heuristics.
const ROLE_TABLE: &[(&str, WidgetKind)] = &[
    ("input", WidgetKind::TextInput),
    ("form-input", WidgetKind::TextInput),
    ("textarea", WidgetKind::TextArea),
    ("text-area", WidgetKind::TextArea),
    ("select", WidgetKind::Select),
    ("dropdown", WidgetKind::Select),
    ("switch", WidgetKind::Switch),
    ("toggle", WidgetKind::Switch),
    ("checkbox", WidgetKind::Checkbox),
    ("slider", WidgetKind::Slider),
    ("radio-group", WidgetKind::RadioGroup),
    ("radio", WidgetKind::RadioGroup),
    ("number-input", WidgetKind::NumberInput),
    ("progress", WidgetKind::Progress),
    ("progress-bar", WidgetKind::Progress),
];

fn role_to_kind(role: &str) -> Option<WidgetKind> {
    ROLE_TABLE.iter().find(|(r, _)| *r == role).map(|(_, k)| *k)
}

/// Every `base.role` string the promote table honours — derived from
/// `ROLE_TABLE`, so downstream skill/prompt parity tests can never
/// drift from the actual lookup.
pub fn promotable_roles() -> impl Iterator<Item = &'static str> {
    ROLE_TABLE.iter().map(|(role, _)| *role)
}

/// Explicit-marker check: `base.role` first, then `semantics.role`.
pub fn widget_kind_for(node: &PenNode) -> Option<WidgetKind> {
    let PenNode::Frame(f) = node else { return None };
    if let Some(role) = f.base.role.as_deref() {
        if let Some(k) = role_to_kind(role) {
            return Some(k);
        }
    }
    if matches!(
        f.semantics.as_ref().and_then(|s| s.role),
        Some(SemanticRole::Input)
    ) {
        return Some(WidgetKind::TextInput);
    }
    None
}

#[derive(Debug, Clone)]
pub struct PromoteNote {
    pub node_id: String,
    pub from_role: String,
    pub to: &'static str,
}

/// Achromatic-ish mid-tone text is treated as a placeholder, anything
/// else as the value. Only ever evaluated INSIDE explicitly marked
/// frames; this is content classification, not widget detection.
fn is_muted_hex(hex: &str) -> bool {
    let h = hex.trim_start_matches('#');
    // Byte-slicing below assumes ASCII; a non-ASCII color string (e.g.
    // a `$variable` ref or garbage) must not panic on a char boundary.
    if h.len() < 6 || !h.is_ascii() {
        return false;
    }
    let (Ok(r), Ok(g), Ok(b)) = (
        u8::from_str_radix(&h[0..2], 16),
        u8::from_str_radix(&h[2..4], 16),
        u8::from_str_radix(&h[4..6], 16),
    ) else {
        return false;
    };
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    max - min <= 24 && (64..=200).contains(&max)
}

/// First `PenFill::Solid` colour string in a fill list, if any.
fn first_solid_hex(fill: &Option<Vec<PenFill>>) -> Option<String> {
    fill.as_ref()?.iter().find_map(|f| match f {
        PenFill::Solid(body) => Some(body.color.clone()),
        _ => None,
    })
}

/// Flatten a text node's content (styled segments are concatenated).
fn text_content_string(content: &TextContent) -> String {
    match content {
        TextContent::Plain(s) => s.clone(),
        TextContent::Styled(segs) => segs.iter().map(|s| s.text.as_str()).collect(),
    }
}

/// Split the frame's text children into `(placeholder, value)`: the
/// first muted-fill text becomes the placeholder, the first other text
/// becomes the value.
fn split_text(frame: &FrameNode) -> (Option<String>, Option<String>) {
    let mut placeholder = None;
    let mut value = None;
    for child in frame.children.iter().flatten() {
        if let PenNode::Text(t) = child {
            let content = text_content_string(&t.content);
            let muted = first_solid_hex(&t.fill)
                .map(|h| is_muted_hex(&h))
                .unwrap_or(false);
            if muted {
                if placeholder.is_none() {
                    placeholder = Some(content);
                }
            } else if value.is_none() {
                value = Some(content);
            }
        }
    }
    (placeholder, value)
}

/// All visible (non-muted) text-child contents, in authoring order —
/// radio-group promotion turns each into one option.
fn collect_text_labels(frame: &FrameNode) -> Vec<String> {
    let mut labels = Vec::new();
    for child in frame.children.iter().flatten() {
        if let PenNode::Text(t) = child {
            let muted = first_solid_hex(&t.fill)
                .map(|h| is_muted_hex(&h))
                .unwrap_or(false);
            if muted {
                continue;
            }
            let content = text_content_string(&t.content);
            if !content.trim().is_empty() {
                labels.push(content);
            }
        }
    }
    labels
}

/// Pull leading + trailing lucide glyph names from a frame's `icon_font`
/// children: the FIRST icon_font (by child order) is the leading icon, a
/// SECOND icon_font (if any) is the trailing icon (e.g. a password-reveal
/// eye). A bare frame yields `(None, None)`. Lets a `role=input` field
/// keep its icons after collapsing to a single `text_input` node.
fn split_icons(frame: &FrameNode) -> (Option<String>, Option<String>) {
    let mut names = frame.children.iter().flatten().filter_map(|c| match c {
        PenNode::IconFont(i) => Some(i.icon_font_name.clone()),
        _ => None,
    });
    let leading = names.next();
    let trailing = names.next();
    (leading, trailing)
}

/// Build the widget node replacing `frame`. Style (fill/stroke/effects/
/// corner_radius) and box sizing are carried over verbatim from the
/// frame's container; the first text child becomes `placeholder` (muted
/// fill) or `value` (anything else), and `icon_font` children become the
/// `text_input` leading/trailing icons. Remaining children are dropped —
/// widget nodes are leaves.
pub fn promote_frame(frame: &FrameNode, kind: WidgetKind) -> PenNode {
    let (placeholder, value) = split_text(frame);
    let (leading_icon, trailing_icon) = split_icons(frame);
    let c = &frame.container;
    let mut base = frame.base.clone();
    base.role = None; // the node type now carries the meaning

    match kind {
        WidgetKind::TextInput => PenNode::TextInput(TextInputNode {
            base,
            width: c.width.clone(),
            height: c.height.clone(),
            placeholder,
            value,
            leading_icon,
            trailing_icon,
            fill: c.fill.clone(),
            stroke: c.stroke.clone(),
            effects: c.effects.clone(),
            corner_radius: c.corner_radius.clone(),
            states: None,
            state: frame.state.clone(),
            bindings: frame.bindings.clone(),
            events: frame.events.clone(),
            lifecycle: frame.lifecycle.clone(),
            semantics: frame.semantics.clone(),
            gestures: frame.gestures.clone(),
            route: frame.route.clone(),
        }),
        WidgetKind::TextArea => PenNode::TextArea(TextAreaNode {
            base,
            width: c.width.clone(),
            height: c.height.clone(),
            placeholder,
            value,
            leading_icon,
            trailing_icon,
            max_visible_lines: None,
            fill: c.fill.clone(),
            stroke: c.stroke.clone(),
            effects: c.effects.clone(),
            corner_radius: c.corner_radius.clone(),
            states: None,
            state: frame.state.clone(),
            bindings: frame.bindings.clone(),
            events: frame.events.clone(),
            lifecycle: frame.lifecycle.clone(),
            semantics: frame.semantics.clone(),
            gestures: frame.gestures.clone(),
            route: frame.route.clone(),
        }),
        WidgetKind::Select => PenNode::Select(SelectNode {
            base,
            width: c.width.clone(),
            height: c.height.clone(),
            // Any visible text becomes the placeholder; legacy frames
            // carry no option list, so authors fill it post-migration.
            placeholder: placeholder.or(value),
            value: None,
            options: None,
            fill: c.fill.clone(),
            stroke: c.stroke.clone(),
            effects: c.effects.clone(),
            corner_radius: c.corner_radius.clone(),
            states: None,
            state: frame.state.clone(),
            bindings: frame.bindings.clone(),
            events: frame.events.clone(),
            lifecycle: frame.lifecycle.clone(),
            semantics: frame.semantics.clone(),
            gestures: frame.gestures.clone(),
            route: frame.route.clone(),
        }),
        WidgetKind::Switch => PenNode::Switch(SwitchNode {
            base,
            width: c.width.clone(),
            height: c.height.clone(),
            checked: None,
            fill: c.fill.clone(),
            stroke: c.stroke.clone(),
            effects: c.effects.clone(),
            corner_radius: c.corner_radius.clone(),
            states: None,
            state: frame.state.clone(),
            bindings: frame.bindings.clone(),
            events: frame.events.clone(),
            lifecycle: frame.lifecycle.clone(),
            semantics: frame.semantics.clone(),
            gestures: frame.gestures.clone(),
            route: frame.route.clone(),
        }),
        WidgetKind::Checkbox => PenNode::Checkbox(CheckboxNode {
            base,
            width: c.width.clone(),
            height: c.height.clone(),
            checked: None,
            // The visible (non-muted) text is the checkbox label.
            label: value.or(placeholder),
            fill: c.fill.clone(),
            stroke: c.stroke.clone(),
            effects: c.effects.clone(),
            corner_radius: c.corner_radius.clone(),
            states: None,
            state: frame.state.clone(),
            bindings: frame.bindings.clone(),
            events: frame.events.clone(),
            lifecycle: frame.lifecycle.clone(),
            semantics: frame.semantics.clone(),
            gestures: frame.gestures.clone(),
            route: frame.route.clone(),
        }),
        WidgetKind::Slider => PenNode::Slider(SliderNode {
            base,
            width: c.width.clone(),
            height: c.height.clone(),
            min: None,
            max: None,
            step: None,
            value: None,
            fill: c.fill.clone(),
            stroke: c.stroke.clone(),
            effects: c.effects.clone(),
            corner_radius: c.corner_radius.clone(),
            states: None,
            state: frame.state.clone(),
            bindings: frame.bindings.clone(),
            events: frame.events.clone(),
            lifecycle: frame.lifecycle.clone(),
            semantics: frame.semantics.clone(),
            gestures: frame.gestures.clone(),
            route: frame.route.clone(),
        }),
        WidgetKind::RadioGroup => PenNode::RadioGroup(RadioGroupNode {
            base,
            width: c.width.clone(),
            height: c.height.clone(),
            value: None,
            options: {
                let labels = collect_text_labels(frame);
                (!labels.is_empty()).then(|| {
                    labels
                        .into_iter()
                        .map(|l| SelectOption {
                            value: l.clone(),
                            label: l,
                        })
                        .collect()
                })
            },
            fill: c.fill.clone(),
            stroke: c.stroke.clone(),
            effects: c.effects.clone(),
            corner_radius: c.corner_radius.clone(),
            states: None,
            state: frame.state.clone(),
            bindings: frame.bindings.clone(),
            events: frame.events.clone(),
            lifecycle: frame.lifecycle.clone(),
            semantics: frame.semantics.clone(),
            gestures: frame.gestures.clone(),
            route: frame.route.clone(),
        }),
        WidgetKind::NumberInput => PenNode::NumberInput(NumberInputNode {
            base,
            width: c.width.clone(),
            height: c.height.clone(),
            placeholder,
            value: value
                .as_deref()
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map(NumberOrExpression::Number),
            leading_icon,
            trailing_icon,
            min: None,
            max: None,
            step: None,
            fill: c.fill.clone(),
            stroke: c.stroke.clone(),
            effects: c.effects.clone(),
            corner_radius: c.corner_radius.clone(),
            states: None,
            state: frame.state.clone(),
            bindings: frame.bindings.clone(),
            events: frame.events.clone(),
            lifecycle: frame.lifecycle.clone(),
            semantics: frame.semantics.clone(),
            gestures: frame.gestures.clone(),
            route: frame.route.clone(),
        }),
        WidgetKind::Progress => PenNode::Progress(ProgressNode {
            base,
            width: c.width.clone(),
            height: c.height.clone(),
            value: None,
            max: None,
            indeterminate: None,
            fill: c.fill.clone(),
            stroke: c.stroke.clone(),
            effects: c.effects.clone(),
            corner_radius: c.corner_radius.clone(),
            states: None,
            state: frame.state.clone(),
            bindings: frame.bindings.clone(),
            events: frame.events.clone(),
            lifecycle: frame.lifecycle.clone(),
            semantics: frame.semantics.clone(),
            gestures: frame.gestures.clone(),
            route: frame.route.clone(),
        }),
    }
}

/// Mutable children of a container-style node (those that can hold a
/// marked frame). Widget nodes are leaves and never recursed into.
fn children_mut(node: &mut PenNode) -> Option<&mut Vec<PenNode>> {
    match node {
        PenNode::Frame(f) => f.children.as_mut(),
        PenNode::Group(g) => g.children.as_mut(),
        PenNode::Rectangle(r) => r.children.as_mut(),
        PenNode::Tabs(t) => t.children.as_mut(),
        PenNode::Ref(r) => r.children.as_mut(),
        _ => None,
    }
}

fn promote_in_slice(nodes: &mut [PenNode], notes: &mut Vec<PromoteNote>) {
    for node in nodes.iter_mut() {
        if let Some(kind) = widget_kind_for(node) {
            let PenNode::Frame(frame) = node.clone() else {
                unreachable!("widget_kind_for only returns Some for frames")
            };
            let from_role = frame
                .base
                .role
                .clone()
                .unwrap_or_else(|| "semantics.role=input".into());
            let id = frame.base.id.clone();
            *node = promote_frame(&frame, kind);
            notes.push(PromoteNote {
                node_id: id,
                from_role,
                to: kind.tag(),
            });
        } else if let Some(children) = children_mut(node) {
            promote_in_slice(children, notes);
        }
    }
}

/// Walk every page/children tree, replacing explicitly marked frames.
/// Returns one note per promotion (for warnings / migration reports).
pub fn promote_document(doc: &mut PenDocument) -> Vec<PromoteNote> {
    let mut notes = Vec::new();
    promote_in_slice(&mut doc.children, &mut notes);
    if let Some(pages) = doc.pages.as_mut() {
        for page in pages.iter_mut() {
            promote_in_slice(&mut page.children, &mut notes);
        }
    }
    notes
}

/// Promote explicitly marked frames in a flat forest (slice of top-level nodes).
///
/// Thin public wrapper over `promote_in_slice` for callers that already hold
/// a `Vec<PenNode>` forest without a full [`PenDocument`] context — e.g. the
/// AI orchestrator's generated-subtree pipeline. Recurses into child containers
/// exactly as [`promote_document`] does.
pub fn promote_forest(nodes: &mut [PenNode]) -> Vec<PromoteNote> {
    let mut notes = Vec::new();
    promote_in_slice(nodes, &mut notes);
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json: &str) -> PenDocument {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn explicit_role_input_promotes_with_placeholder_split() {
        let mut d = doc(r##"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"f1","role":"input",
           "fill":[{"type":"solid","color":"#1E1E1E"}],
           "children":[{"type":"text","id":"t1","content":"Enter email",
                        "fill":[{"type":"solid","color":"#9A9A9A"}]}]}]}"##);
        let notes = promote_document(&mut d);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].to, "text_input");
        let PenNode::TextInput(ti) = &d.children[0] else {
            panic!()
        };
        assert_eq!(ti.placeholder.as_deref(), Some("Enter email")); // muted grey
        assert!(ti.value.is_none());
        assert!(ti.base.role.is_none());
        // Style carried over from the frame container.
        assert!(ti.fill.is_some());
    }

    #[test]
    fn promote_input_frame_carries_icons_and_placeholder() {
        // role=input field: leading mail icon + muted placeholder + trailing eye.
        let mut d = doc(r##"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"emailField","role":"input","width":300,"height":48,
           "cornerRadius":12,"fill":[{"type":"solid","color":"#f3f4f6"}],"children":[
             {"type":"icon_font","id":"i1","iconFontName":"mail","width":20,"height":20},
             {"type":"text","id":"t1","content":"you@example.com",
              "fill":[{"type":"solid","color":"#9ca3af"}]},
             {"type":"icon_font","id":"i2","iconFontName":"eye","width":20,"height":20}
           ]}]}"##);
        let notes = promote_document(&mut d);
        assert_eq!(notes[0].to, "text_input");
        let PenNode::TextInput(t) = &d.children[0] else {
            panic!("did not become text_input")
        };
        assert_eq!(t.leading_icon.as_deref(), Some("mail"));
        assert_eq!(t.trailing_icon.as_deref(), Some("eye"));
        assert_eq!(t.placeholder.as_deref(), Some("you@example.com"));
        assert_eq!(t.base.id, "emailField"); // id preserved
    }

    #[test]
    fn text_input_round_trips_leading_and_trailing_icon() {
        let node: PenNode = serde_json::from_str(
            r#"{"type":"text_input","id":"f","leadingIcon":"mail","trailingIcon":"eye","placeholder":"you@example.com"}"#,
        )
        .expect("parse");
        let PenNode::TextInput(t) = &node else {
            panic!("not a text_input")
        };
        assert_eq!(t.leading_icon.as_deref(), Some("mail"));
        assert_eq!(t.trailing_icon.as_deref(), Some("eye"));
        // Absent icon fields must NOT serialize (skip_serializing_if).
        let bare: PenNode = serde_json::from_str(r#"{"type":"text_input","id":"g"}"#).unwrap();
        let out = serde_json::to_string(&bare).unwrap();
        assert!(
            !out.contains("leadingIcon"),
            "absent icon must be omitted: {out}"
        );
    }

    #[test]
    fn unmarked_frame_is_untouched() {
        let mut d = doc(r#"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"f1","name":"Input looking thing","children":[]}]}"#);
        assert!(promote_document(&mut d).is_empty());
        assert!(matches!(d.children[0], PenNode::Frame(_)));
    }

    #[test]
    fn semantics_role_input_promotes() {
        let mut d = doc(r#"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"f1","semantics":{"role":"input"},"children":[]}]}"#);
        let notes = promote_document(&mut d);
        assert_eq!(notes[0].to, "text_input");
    }

    #[test]
    fn nested_marked_frame_promotes() {
        let mut d = doc(r#"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"outer","children":[
            {"type":"frame","id":"inner","role":"switch","children":[]}]}]}"#);
        let notes = promote_document(&mut d);
        assert_eq!(notes[0].node_id, "inner");
        assert_eq!(notes[0].to, "switch");
        let PenNode::Frame(outer) = &d.children[0] else {
            panic!()
        };
        assert!(matches!(
            outer.children.as_ref().unwrap()[0],
            PenNode::Switch(_)
        ));
    }

    #[test]
    fn checkbox_label_from_visible_text() {
        let mut d = doc(r##"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"c1","role":"checkbox",
           "children":[{"type":"text","id":"t","content":"Accept terms",
                        "fill":[{"type":"solid","color":"#111111"}]}]}]}"##);
        let notes = promote_document(&mut d);
        assert_eq!(notes[0].to, "checkbox");
        let PenNode::Checkbox(cb) = &d.children[0] else {
            panic!()
        };
        assert_eq!(cb.label.as_deref(), Some("Accept terms"));
    }

    #[test]
    fn marked_frame_inside_tabs_panel_promotes() {
        // Tabs is a container; promotion must recurse into its panels.
        let mut d = doc(r#"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"tabs","id":"tb","value":"a",
           "tabs":[{"value":"a","label":"A"}],
           "children":[
             {"type":"frame","id":"panel","children":[
               {"type":"frame","id":"in","role":"input","children":[]}]}]}]}"#);
        let notes = promote_document(&mut d);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].node_id, "in");
        assert_eq!(notes[0].to, "text_input");
    }

    #[test]
    fn promote_frame_preserves_events_bindings_state_semantics() {
        // A role-tagged input frame carrying interactivity must keep its
        // events/bindings/state/semantics after promotion to a text_input widget.
        let mut d = doc(r##"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"f1","role":"input",
           "state":{"q":{"type":"string","default":""}},
           "bindings":{"value":"$app.q"},
           "events":{"onChange":[{"set":{"$app.q":"$event.value"}}]},
           "semantics":{"label":"Search field"},
           "children":[{"type":"text","id":"t1","content":"Search",
                        "fill":[{"type":"solid","color":"#9A9A9A"}]}]}]}"##);
        let notes = promote_document(&mut d);
        assert_eq!(notes[0].to, "text_input");
        let PenNode::TextInput(ti) = &d.children[0] else {
            panic!("did not become text_input")
        };
        assert!(ti.state.is_some(), "state must survive promotion");
        assert!(ti.bindings.is_some(), "bindings must survive promotion");
        assert!(ti.events.is_some(), "events must survive promotion");
        assert!(ti.semantics.is_some(), "semantics must survive promotion");
    }

    #[test]
    fn muted_hex_rejects_non_ascii_without_panicking() {
        // A non-ASCII / non-hex fill string must not panic on a byte-
        // boundary slice; it simply isn't treated as a muted placeholder.
        assert!(!is_muted_hex("$变量"));
        assert!(!is_muted_hex("#abc"));
        assert!(is_muted_hex("#9A9A9A"));
    }

    #[test]
    fn promote_forest_input_frame_becomes_text_input_with_placeholder_and_icons() {
        // promote_forest operates on a bare Vec<PenNode> forest (no PenDocument
        // wrapper) — the AI pipeline passes generated subtrees this way.
        // role="input" + leading mail icon + muted placeholder + trailing eye
        // should promote to a TextInput node carrying all three attributes.
        let json = r##"[
          {"type":"frame","id":"f1","role":"input","width":320,"height":48,
           "cornerRadius":8,"fill":[{"type":"solid","color":"#f9fafb"}],"children":[
             {"type":"icon_font","id":"ico1","iconFontName":"mail","width":20,"height":20},
             {"type":"text","id":"t1","content":"Enter your email",
              "fill":[{"type":"solid","color":"#9ca3af"}]},
             {"type":"icon_font","id":"ico2","iconFontName":"eye","width":20,"height":20}
           ]}
        ]"##;
        let mut nodes: Vec<PenNode> = serde_json::from_str(json).unwrap();
        let notes = promote_forest(&mut nodes);

        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].to, "text_input");
        assert_eq!(notes[0].node_id, "f1");

        let PenNode::TextInput(ti) = &nodes[0] else {
            panic!("expected TextInput after promote_forest");
        };
        // Muted-fill text → placeholder.
        assert_eq!(ti.placeholder.as_deref(), Some("Enter your email"));
        assert!(ti.value.is_none(), "no non-muted text → value must be None");
        // Icons carried through from the frame's icon_font children.
        assert_eq!(ti.leading_icon.as_deref(), Some("mail"));
        assert_eq!(ti.trailing_icon.as_deref(), Some("eye"));
        // Base role cleared — the node type now carries the semantic.
        assert!(ti.base.role.is_none());
        // Original id preserved.
        assert_eq!(ti.base.id, "f1");
        // Container style forwarded.
        assert!(
            ti.fill.is_some(),
            "fill must be carried over from the frame"
        );
    }

    #[test]
    fn promotes_radio_group_role_with_text_options() {
        let mut d = doc(r##"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"rg","role":"radio-group","width":200,"height":80,"children":[
            {"type":"text","id":"o1","content":"Monthly"},
            {"type":"text","id":"o2","content":"Yearly"}]}]}"##);
        let notes = promote_document(&mut d);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].to, "radio_group");
        let PenNode::RadioGroup(rg) = &d.children[0] else {
            panic!("expected RadioGroup, got {:?}", d.children[0]);
        };
        assert!(rg.base.role.is_none(), "role marker must be cleared");
        let opts = rg.options.as_ref().expect("options from text children");
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].label, "Monthly");
        assert_eq!(opts[1].value, "Yearly");
    }

    #[test]
    fn promotes_number_input_role_with_muted_placeholder() {
        let mut d = doc(r##"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"amt","role":"number-input","width":160,"height":40,"children":[
            {"type":"text","id":"ph","content":"0.00",
             "fill":[{"type":"solid","color":"#9A9A9A"}]}]}]}"##);
        let notes = promote_document(&mut d);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].to, "number_input");
        let PenNode::NumberInput(n) = &d.children[0] else {
            panic!("expected NumberInput, got {:?}", d.children[0]);
        };
        assert_eq!(n.placeholder.as_deref(), Some("0.00"));
        assert!(n.value.is_none(), "muted text is placeholder, not value");
    }

    #[test]
    fn promotes_progress_role() {
        let mut d = doc(r##"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"bar","role":"progress","width":300,"height":8}]}"##);
        let notes = promote_document(&mut d);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].to, "progress");
        assert!(
            matches!(&d.children[0], PenNode::Progress(_)),
            "got {:?}",
            d.children[0]
        );
    }

    #[test]
    fn tabs_role_is_not_promoted() {
        // Deliberate exclusion: legacy tabs frames stay frames.
        let mut d = doc(r##"{"version":"1.1","formatVersion":"1.1","children":[
          {"type":"frame","id":"t","role":"tabs","width":300,"height":200}]}"##);
        let notes = promote_document(&mut d);
        assert!(notes.is_empty());
        assert!(matches!(&d.children[0], PenNode::Frame(_)));
    }

    #[test]
    fn role_table_has_no_duplicate_keys() {
        // `role_to_kind` is first-match over ROLE_TABLE — a duplicate
        // key would silently shadow a mapping.
        let mut seen = std::collections::BTreeSet::new();
        for role in promotable_roles() {
            assert!(seen.insert(role), "duplicate role key {role}");
        }
    }
}
