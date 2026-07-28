//! Convert `jian-ops-schema` layout hints to `taffy` equivalents.

use jian_ops_schema::node::container::{
    AlignItems as OpsAlign, ContainerProps, JustifyContent as OpsJustify, LayoutMode,
    Padding as OpsPadding,
};
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};
use taffy::prelude::*;

pub fn resolve_sizing(s: Option<&SizingBehavior>) -> Dimension {
    match s {
        Some(SizingBehavior::Number(v)) => length(*v as f32),
        Some(SizingBehavior::Keyword(SizingKeyword::FitContent)) => auto(),
        Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)) => percent(1.0_f32),
        // Expression-sized nodes get auto; the runtime re-resolves once the
        // expression is evaluated and calls LayoutEngine::mark_dirty.
        Some(SizingBehavior::Expression(_)) => auto(),
        None => auto(),
    }
}

pub fn resolve_padding(p: Option<&OpsPadding>) -> Rect<LengthPercentage> {
    // `.op` padding follows CSS shorthand order despite the variant
    // names — that's what real designs author against. Two-value form
    // is `[vertical, horizontal]`; four-value is `[top, right, bottom,
    // left]` (TRBL).
    match p {
        Some(OpsPadding::Uniform(v)) => {
            let lp: LengthPercentage = length(*v as f32);
            Rect {
                left: lp,
                right: lp,
                top: lp,
                bottom: lp,
            }
        }
        Some(OpsPadding::XY([v, h])) => {
            let lv: LengthPercentage = length(*v as f32);
            let lh: LengthPercentage = length(*h as f32);
            Rect {
                left: lh,
                right: lh,
                top: lv,
                bottom: lv,
            }
        }
        Some(OpsPadding::LtrB([t, r, b, l])) => Rect {
            top: length(*t as f32),
            right: length(*r as f32),
            bottom: length(*b as f32),
            left: length(*l as f32),
        },
        _ => zero(),
    }
}

pub fn resolve_flex_direction(layout: Option<&LayoutMode>) -> FlexDirection {
    match layout {
        Some(LayoutMode::Vertical) => FlexDirection::Column,
        Some(LayoutMode::Horizontal) => FlexDirection::Row,
        _ => FlexDirection::Row, // default
    }
}

pub fn resolve_justify(j: Option<&OpsJustify>) -> JustifyContent {
    match j {
        Some(OpsJustify::Start) => JustifyContent::FlexStart,
        Some(OpsJustify::Center) => JustifyContent::Center,
        Some(OpsJustify::End) => JustifyContent::FlexEnd,
        Some(OpsJustify::SpaceBetween) => JustifyContent::SpaceBetween,
        Some(OpsJustify::SpaceAround) => JustifyContent::SpaceAround,
        _ => JustifyContent::FlexStart,
    }
}

pub fn resolve_align(a: Option<&OpsAlign>) -> AlignItems {
    match a {
        Some(OpsAlign::Start) => AlignItems::FlexStart,
        Some(OpsAlign::Center) => AlignItems::Center,
        Some(OpsAlign::End) => AlignItems::FlexEnd,
        // `stretch` renders as start: the position step has no font-metrics
        // stretch pass, matching TS `normalizeAlignItems('stretch') -> 'start'`.
        Some(OpsAlign::Stretch) => AlignItems::FlexStart,
        None => AlignItems::FlexStart,
    }
}

/// True when this container stacks its children at a common origin rather
/// than flowing them: `layout: "none"` — the `.op` spelling of a Figma
/// "absolute position" frame / CSS `position: relative` stacking context.
pub fn is_stack_container(layout: Option<&LayoutMode>) -> bool {
    matches!(layout, Some(LayoutMode::None))
}

/// Same predicate, keyed off a whole node. Leaves and non-container
/// variants are never stacks.
pub fn node_is_stack_container(n: &jian_ops_schema::node::PenNode) -> bool {
    use jian_ops_schema::node::PenNode;
    let layout = match n {
        PenNode::Frame(f) => f.container.layout.as_ref(),
        PenNode::Group(g) => g.container.layout.as_ref(),
        PenNode::Rectangle(r) => r.container.layout.as_ref(),
        _ => None,
    };
    is_stack_container(layout)
}

/// Style for a `layout: "none"` container: a single-cell CSS grid.
///
/// Flex has no way to overlap siblings, so `layout: "none"` used to fall
/// through `resolve_flex_direction`'s `_ => Row` default and lay its
/// children out side by side — a hero frame holding [content, scrim,
/// photo] rendered as two half-width columns instead of three stacked
/// layers. A 1×1 grid is the standard CSS stacking primitive and keeps
/// the two properties flex would lose:
///
/// - **`fill_container` resolves against the parent.** `justify_content` /
///   `align_content` are deliberately left unset so taffy's grid default
///   (`Stretch`) grows the single auto track to the container's content
///   box; a `percent(1.0)` child then measures the full frame instead of
///   an equal flex share.
/// - **`fit_content` parents still hug.** An auto track sizes to its
///   largest item, so a `fit_content` badge wrapping one absolute label
///   keeps its content size (pure `position: absolute` children would
///   contribute nothing and collapse it to 0×0).
///
/// Authored `justifyContent` / `alignItems` move from the container to the
/// ITEMS (`justify_items` / `align_items`), which keeps the meaning they had
/// under the old flex-row reading — horizontal from `justifyContent`,
/// vertical from `alignItems` — for the very common
/// `layout:"none" + justifyContent:center + alignItems:center` icon badge
/// holding one glyph. Leaving them on the container would instead position
/// the TRACK and defeat the stretch that `fill_container` depends on.
/// Children are placed into the shared cell by [`apply_stack_child`].
fn stack_container_to_style(c: &ContainerProps, mut style: Style) -> Style {
    style.display = Display::Grid;
    style.grid_template_rows = vec![auto()];
    style.grid_template_columns = vec![auto()];
    style.justify_content = None;
    style.align_content = None;
    style.justify_items = Some(resolve_justify_items(c.justify_content.as_ref()));
    style.align_items = Some(resolve_align_items(c.align_items.as_ref()));
    // Gap is meaningless between overlapping layers, and a non-zero value
    // would offset the implicit second track if a child ever escapes the
    // shared cell.
    style.gap = Size {
        width: zero(),
        height: zero(),
    };
    style
}

/// `justifyContent` read as a per-item inline-axis alignment. The
/// distribution values have no single-item meaning and fall back to `Start`,
/// which is where flex put a lone child anyway.
fn resolve_justify_items(j: Option<&OpsJustify>) -> AlignItems {
    match j {
        Some(OpsJustify::Center) => AlignItems::Center,
        Some(OpsJustify::End) => AlignItems::End,
        _ => AlignItems::Start,
    }
}

/// `alignItems` read as a per-item block-axis alignment. `Start` rather than
/// grid's `Stretch` default, which would inflate every `fit_content` child to
/// the full cell.
fn resolve_align_items(a: Option<&OpsAlign>) -> AlignItems {
    match a {
        Some(OpsAlign::Center) => AlignItems::Center,
        Some(OpsAlign::End) => AlignItems::End,
        _ => AlignItems::Start,
    }
}

/// Place a child into a stack parent's shared cell and neutralise the flex
/// hints that no longer apply. Called by `LayoutEngine::build` once the
/// parent is known; children keep their own `Position::Absolute` inset when
/// they authored an explicit `x` / `y` (grid positions those against the
/// container's padding box, which is the same origin as the cell).
pub fn apply_stack_child(style: &mut Style) {
    style.grid_row = line(1);
    style.grid_column = line(1);
    // A stacked layer is never flexed, and `min_size: 0` (set by
    // `container_to_style` so a fill child can yield space to a fixed
    // sibling in a row) would let a grid item shrink below its content.
    style.flex_shrink = 0.0;
    style.flex_grow = 0.0;
    style.align_self = None;
}

pub fn container_to_style(c: &ContainerProps) -> Style {
    let gap_val = match c.gap.as_ref() {
        Some(jian_ops_schema::node::base::NumberOrExpression::Number(n)) => *n as f32,
        _ => 0.0,
    };
    let gap_lp: LengthPercentage = length(gap_val);
    // A fixed (Number) container must not be flex-shrunk below its size —
    // same rule as leaf nodes. TS sums fixed sizes and shares only the
    // remainder among fill_container, so a 44x44 icon-button background or
    // a round pin background keeps its square/circle instead of taffy's
    // default flex_shrink:1 squeezing it into a thin oval/rectangle.
    let fixed = matches!(c.width.as_ref(), Some(SizingBehavior::Number(_)))
        && matches!(c.height.as_ref(), Some(SizingBehavior::Number(_)));
    // A fill_container axis must be allowed to shrink below its content's
    // min-size, so a fixed sibling (flex_shrink=0) gets its space. TS gives
    // fill the *remaining* space (text wraps), not its longest-line
    // min-content; taffy's default `min_size: auto` would clamp a fill text
    // column at min-content and push the fixed image out of the card.
    let fill_w = matches!(
        c.width.as_ref(),
        Some(SizingBehavior::Keyword(SizingKeyword::FillContainer))
    );
    let fill_h = matches!(
        c.height.as_ref(),
        Some(SizingBehavior::Keyword(SizingKeyword::FillContainer))
    );
    let style = Style {
        display: Display::Flex,
        size: Size {
            width: resolve_sizing(c.width.as_ref()),
            height: resolve_sizing(c.height.as_ref()),
        },
        min_size: Size {
            width: if fill_w { length(0.0_f32) } else { auto() },
            height: if fill_h { length(0.0_f32) } else { auto() },
        },
        flex_shrink: if fixed { 0.0 } else { 1.0 },
        padding: resolve_padding(c.padding.as_ref()),
        flex_direction: resolve_flex_direction(c.layout.as_ref()),
        justify_content: Some(resolve_justify(c.justify_content.as_ref())),
        align_items: Some(resolve_align(c.align_items.as_ref())),
        gap: Size {
            width: gap_lp,
            height: gap_lp,
        },
        ..Default::default()
    };
    if is_stack_container(c.layout.as_ref()) {
        return stack_container_to_style(c, style);
    }
    style
}

/// Map optional authored bounds onto taffy's min/max dimensions.
pub(crate) fn apply_limits(style: &mut Style, limits: &jian_ops_schema::sizing::SizeLimits) {
    if let Some(value) = limits.min_width {
        style.min_size.width = length(value as f32);
    }
    if let Some(value) = limits.max_width {
        style.max_size.width = length(value as f32);
    }
    if let Some(value) = limits.min_height {
        style.min_size.height = length(value as f32);
    }
    if let Some(value) = limits.max_height {
        style.max_size.height = length(value as f32);
    }
}

/// Build a Style for any node type. Containers (Frame/Group/Rectangle)
/// delegate to `container_to_style`; leaf nodes (Text / IconFont /
/// Image / Line / …) pull their own `width` / `height` so flex parents
/// measure them correctly. If the node declares an explicit `x` or
/// `y`, it's treated as absolutely positioned within its parent
/// (mirrors Figma / design-tool semantics where an overlay icon sits
/// at a fixed offset inside a layer).
pub fn node_to_style(n: &jian_ops_schema::node::PenNode) -> Style {
    use jian_ops_schema::node::PenNode;
    let mut style = match n {
        PenNode::Frame(f) => container_to_style(&f.container),
        PenNode::Group(g) => container_to_style(&g.container),
        PenNode::Rectangle(r) => container_to_style(&r.container),
        PenNode::Line(l) => Style {
            size: Size {
                width: length(l.x2.unwrap_or(0.0).abs() as f32),
                height: length(l.y2.unwrap_or(0.0).abs() as f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        },
        _ => {
            let (w, h, _) = leaf_size_and_limits(n);
            // A fixed (Number) leaf must not be flex-shrunk below its size:
            // TS sums fixed sizes and shares only the remainder among
            // fill_container, whereas taffy's default `flex_shrink = 1`
            // compresses a fixed 132x132 image / 44x44 icon in a tight row
            // (square → rectangle). Pin shrink off only when BOTH axes are
            // fixed Number — a width=fill,height=Number tile must still
            // flex-share its row, so squares keep their size (matches TS).
            let fixed = matches!(w, Some(SizingBehavior::Number(_)))
                && matches!(h, Some(SizingBehavior::Number(_)));
            Style {
                size: Size {
                    width: resolve_sizing(w),
                    height: resolve_sizing(h),
                },
                flex_shrink: if fixed { 0.0 } else { 1.0 },
                ..Default::default()
            }
        }
    };
    if let Some((x, y)) = explicit_position(n) {
        style.position = Position::Absolute;
        style.inset = Rect {
            left: length(x),
            top: length(y),
            right: LengthPercentageAuto::Auto,
            bottom: LengthPercentageAuto::Auto,
        };
    }
    style
}

/// Responsive style conversion: legacy conversion plus sanitized min/max bounds.
pub(crate) fn node_to_style_responsive(
    node: &jian_ops_schema::node::PenNode,
    warnings: &mut Vec<String>,
) -> Style {
    let mut style = node_to_style(node);
    if let (Some(base), Some(limits)) = (node_base(node), node_limits(node)) {
        let limits = limits.sanitized(&base.id, warnings);
        apply_limits(&mut style, &limits);
    }
    style
}

/// Extract `x`, `y` from a `PenNodeBase` — `(x, y)` pair if either is
/// present, `None` if neither.
///
/// `pub(crate)` (not just a `node_to_style` implementation detail):
/// `LayoutEngine` records a document root's authored origin while building.
/// Taffy has no containing block for a root node, so the `Position::Absolute`
/// inset this function drives is honoured for children but silently dropped
/// for roots; `node_rect` restores the recorded offset after compute.
pub(crate) fn explicit_position(n: &jian_ops_schema::node::PenNode) -> Option<(f32, f32)> {
    let base = node_base(n)?;
    match (base.x, base.y) {
        (None, None) => None,
        (x, y) => Some((x.unwrap_or(0.0) as f32, y.unwrap_or(0.0) as f32)),
    }
}

fn node_base(
    n: &jian_ops_schema::node::PenNode,
) -> Option<&jian_ops_schema::node::base::PenNodeBase> {
    use jian_ops_schema::node::PenNode;
    Some(match n {
        PenNode::Frame(f) => &f.base,
        PenNode::Group(g) => &g.base,
        PenNode::Rectangle(r) => &r.base,
        PenNode::Text(t) => &t.base,
        PenNode::TextInput(t) => &t.base,
        PenNode::IconFont(i) => &i.base,
        PenNode::Image(i) => &i.base,
        PenNode::Ellipse(e) => &e.base,
        PenNode::Line(l) => &l.base,
        PenNode::Path(p) => &p.base,
        PenNode::Polygon(p) => &p.base,
        PenNode::TextArea(t) => &t.base,
        PenNode::Select(s) => &s.base,
        PenNode::Switch(s) => &s.base,
        PenNode::Checkbox(c) => &c.base,
        PenNode::Slider(s) => &s.base,
        PenNode::RadioGroup(r) => &r.base,
        PenNode::NumberInput(n) => &n.base,
        PenNode::Progress(p) => &p.base,
        PenNode::Tabs(t) => &t.base,
        _ => return None,
    })
}

fn node_limits(
    node: &jian_ops_schema::node::PenNode,
) -> Option<&jian_ops_schema::sizing::SizeLimits> {
    use jian_ops_schema::node::PenNode;
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

fn leaf_size_and_limits(
    n: &jian_ops_schema::node::PenNode,
) -> (
    Option<&SizingBehavior>,
    Option<&SizingBehavior>,
    Option<&jian_ops_schema::sizing::SizeLimits>,
) {
    use jian_ops_schema::node::PenNode;
    match n {
        PenNode::Text(t) => (t.width.as_ref(), t.height.as_ref(), Some(&t.limits)),
        PenNode::TextInput(t) => (t.width.as_ref(), t.height.as_ref(), Some(&t.limits)),
        PenNode::IconFont(i) => (i.width.as_ref(), i.height.as_ref(), Some(&i.limits)),
        PenNode::Image(i) => (i.width.as_ref(), i.height.as_ref(), Some(&i.limits)),
        // Ellipse was missing — its width/height never reached the flex
        // solver, so a 36×36 avatar ring measured 0×0 and `justifyContent:
        // center` parked its ORIGIN at the parent's center (+half, +half);
        // paint then drew the declared 36×36 shifted by its own radius
        // (measured: a 270° arc ring rendered half-off its avatar).
        PenNode::Ellipse(e) => (e.width.as_ref(), e.height.as_ref(), Some(&e.limits)),
        PenNode::Polygon(p) => (p.width.as_ref(), p.height.as_ref(), Some(&p.limits)),
        // Path has the same flex-solver requirement as Ellipse:
        // chart paths authored as fill_container inside a layout:none card
        // resolved to w=0, so the painter's path-fit fallback translated
        // without scaling and the chart line never stretched with its card.
        PenNode::Path(p) => (p.width.as_ref(), p.height.as_ref(), Some(&p.limits)),
        PenNode::TextArea(t) => (t.width.as_ref(), t.height.as_ref(), Some(&t.limits)),
        PenNode::Select(s) => (s.width.as_ref(), s.height.as_ref(), Some(&s.limits)),
        PenNode::Switch(s) => (s.width.as_ref(), s.height.as_ref(), Some(&s.limits)),
        PenNode::Checkbox(c) => (c.width.as_ref(), c.height.as_ref(), Some(&c.limits)),
        PenNode::Slider(s) => (s.width.as_ref(), s.height.as_ref(), Some(&s.limits)),
        PenNode::RadioGroup(r) => (r.width.as_ref(), r.height.as_ref(), Some(&r.limits)),
        PenNode::NumberInput(n) => (n.width.as_ref(), n.height.as_ref(), Some(&n.limits)),
        PenNode::Progress(p) => (p.width.as_ref(), p.height.as_ref(), Some(&p.limits)),
        PenNode::Tabs(t) => (t.width.as_ref(), t.height.as_ref(), Some(&t.limits)),
        _ => (None, None, None),
    }
}

/// `(width, height)` sizing for ANY node — containers pull from their
/// `ContainerProps`, every other variant from its own leaf width/height.
fn node_size_refs(
    n: &jian_ops_schema::node::PenNode,
) -> (Option<&SizingBehavior>, Option<&SizingBehavior>) {
    use jian_ops_schema::node::PenNode;
    match n {
        PenNode::Frame(f) => (f.container.width.as_ref(), f.container.height.as_ref()),
        PenNode::Group(g) => (g.container.width.as_ref(), g.container.height.as_ref()),
        PenNode::Rectangle(r) => (r.container.width.as_ref(), r.container.height.as_ref()),
        _ => {
            let (width, height, _) = leaf_size_and_limits(n);
            (width, height)
        }
    }
}

/// True when this node lays its children out in a Row (horizontal main axis).
/// A `null`/absent layout defaults to Row — matching `resolve_flex_direction`,
/// whose `None => Row` default would otherwise disagree with a `null`-layout
/// container being classed as vertical (the OycZJ sidebar+content root).
pub fn node_is_horizontal(n: &jian_ops_schema::node::PenNode) -> bool {
    use jian_ops_schema::node::PenNode;
    let layout = match n {
        PenNode::Frame(f) => f.container.layout.as_ref(),
        PenNode::Group(g) => g.container.layout.as_ref(),
        PenNode::Rectangle(r) => r.container.layout.as_ref(),
        _ => None,
    };
    !matches!(layout, Some(LayoutMode::Vertical))
}

/// True when the child's MAIN-AXIS size — width when the parent is a Row,
/// height when the parent is a Column — is a fixed `Number`. Such a child must
/// not be flex-shrunk below that size: a `width:200, height:fit_content` dish
/// card in a horizontal scroll row keeps its 200px instead of being squeezed
/// (which would wrap its title and leave sibling cards at unequal heights). The
/// cross axis is irrelevant, so this generalizes the both-axes-fixed rule that
/// `node_to_style` applies to fully-fixed squares.
pub fn main_axis_is_fixed_number(
    child: &jian_ops_schema::node::PenNode,
    parent_horizontal: bool,
) -> bool {
    let (w, h) = node_size_refs(child);
    let axis = if parent_horizontal { w } else { h };
    matches!(axis, Some(SizingBehavior::Number(_)))
}

/// Fix `fill_container` height by the parent's axis. taffy resolves
/// `fill_container` to `percent(1.0)`, which fails inside a parent whose own
/// height isn't definite (the OycZJ sidebar footer never sank to the bottom).
///
/// - **Horizontal parent** (height is the CROSS axis): map to
///   `align_self: Stretch` + `height: auto` so the child stretches across the
///   row's resolved height. Width (the main axis) is left untouched on purpose —
///   stretching it changes text wrapping and reintroduced a +350px height
///   regression in the corpus gate.
/// - **Vertical parent** (height is the MAIN axis): map to `flex_grow: 1` +
///   `height: auto` so an empty `fill_container` spacer GROWS into the column's
///   remaining space and pushes a sidebar footer to the bottom (without it,
///   `percent(1.0)` + `flex_shrink` collapses the spacer to zero). Height-only:
///   no text-wrap / width impact, so it is safe from the main-axis regression
///   above (which was width-driven).
///
/// Both remedies are flex-flow mechanisms, so `LayoutEngine::build` does not
/// call this for a child of a `layout: "none"` stack — those resolve their
/// `percent(1.0)` against the grid track directly. Applying it there rewrote a
/// full-bleed gradient scrim's height to `auto` with no flow line to stretch
/// against, and it measured 0.
pub fn apply_fill_container_axes(
    style: &mut Style,
    child: &jian_ops_schema::node::PenNode,
    parent_horizontal: bool,
) {
    let (_, h) = node_size_refs(child);
    if !matches!(
        h,
        Some(SizingBehavior::Keyword(SizingKeyword::FillContainer))
    ) {
        return;
    }
    if parent_horizontal {
        style.align_self = Some(AlignItems::Stretch);
        style.size.height = auto();
    } else {
        style.flex_grow = 1.0;
        style.size.height = auto();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dim_len(v: f32) -> Dimension {
        length(v)
    }
    fn lp_len(v: f32) -> LengthPercentage {
        length(v)
    }
    fn dim_auto() -> Dimension {
        auto()
    }
    fn dim_percent(v: f32) -> Dimension {
        percent(v)
    }

    #[test]
    fn sizing_number_to_length() {
        let s = SizingBehavior::Number(100.0);
        assert_eq!(resolve_sizing(Some(&s)), dim_len(100.0));
    }

    #[test]
    fn sizing_fit_content_to_auto() {
        let s = SizingBehavior::Keyword(SizingKeyword::FitContent);
        assert_eq!(resolve_sizing(Some(&s)), dim_auto());
    }

    #[test]
    fn sizing_fill_container_to_percent() {
        let s = SizingBehavior::Keyword(SizingKeyword::FillContainer);
        assert_eq!(resolve_sizing(Some(&s)), dim_percent(1.0));
    }

    #[test]
    fn padding_uniform() {
        let p = OpsPadding::Uniform(8.0);
        let r = resolve_padding(Some(&p));
        assert_eq!(r.left, lp_len(8.0));
        assert_eq!(r.right, lp_len(8.0));
    }

    #[test]
    fn padding_four_value_trbl() {
        // Four-value padding follows CSS TRBL shorthand
        // (top, right, bottom, left) regardless of the variant name.
        let p = OpsPadding::LtrB([1.0, 2.0, 3.0, 4.0]);
        let r = resolve_padding(Some(&p));
        assert_eq!(r.top, lp_len(1.0));
        assert_eq!(r.right, lp_len(2.0));
        assert_eq!(r.bottom, lp_len(3.0));
        assert_eq!(r.left, lp_len(4.0));
    }

    #[test]
    fn padding_two_value_is_vertical_horizontal() {
        let p = OpsPadding::XY([10.0, 20.0]);
        let r = resolve_padding(Some(&p));
        assert_eq!(r.top, lp_len(10.0));
        assert_eq!(r.bottom, lp_len(10.0));
        assert_eq!(r.left, lp_len(20.0));
        assert_eq!(r.right, lp_len(20.0));
    }

    #[test]
    fn layout_none_becomes_a_single_cell_grid() {
        let container: ContainerProps =
            serde_json::from_str(r#"{"layout":"none","width":375,"height":320}"#).unwrap();
        let style = container_to_style(&container);
        assert_eq!(style.display, Display::Grid);
        assert_eq!(style.grid_template_rows.len(), 1);
        assert_eq!(style.grid_template_columns.len(), 1);
        // Track alignment must stay at taffy's `Stretch` default so the auto
        // track absorbs the frame's definite size and a `fill_container`
        // child's `percent(1.0)` has something to resolve against.
        assert_eq!(style.justify_content, None);
        assert_eq!(style.align_content, None);
        // Items default to Start, not grid's Stretch, which would inflate
        // every `fit_content` layer to the whole cell.
        assert_eq!(style.align_items, Some(AlignItems::Start));
        assert_eq!(style.justify_items, Some(AlignItems::Start));
    }

    #[test]
    fn layout_none_moves_alignment_onto_the_items() {
        // Authored alignment keeps the axes it had under the old flex-row
        // reading: `justifyContent` horizontal, `alignItems` vertical.
        let container: ContainerProps = serde_json::from_str(
            r#"{"layout":"none","justifyContent":"center","alignItems":"end"}"#,
        )
        .unwrap();
        let style = container_to_style(&container);
        assert_eq!(style.justify_items, Some(AlignItems::Center));
        assert_eq!(style.align_items, Some(AlignItems::End));
        assert_eq!(style.justify_content, None);
        assert_eq!(style.align_content, None);
    }

    #[test]
    fn flow_layouts_stay_flex() {
        for layout in [r#""vertical""#, r#""horizontal""#] {
            let container: ContainerProps =
                serde_json::from_str(&format!(r#"{{"layout":{layout}}}"#)).unwrap();
            assert_eq!(container_to_style(&container).display, Display::Flex);
        }
        // An absent `layout` is a flex row, not a stack.
        let container: ContainerProps = serde_json::from_str(r#"{"width":10}"#).unwrap();
        assert_eq!(container_to_style(&container).display, Display::Flex);
    }

    #[test]
    fn stack_child_is_pinned_to_the_shared_cell() {
        let mut style = Style {
            flex_shrink: 1.0,
            flex_grow: 1.0,
            align_self: Some(AlignItems::Stretch),
            ..Default::default()
        };
        apply_stack_child(&mut style);
        assert_eq!(style.grid_row, line(1));
        assert_eq!(style.grid_column, line(1));
        assert_eq!(style.flex_shrink, 0.0);
        assert_eq!(style.flex_grow, 0.0);
        assert_eq!(style.align_self, None);
    }

    #[test]
    fn stack_child_keeps_its_fill_container_height() {
        // `apply_stack_child` is the whole per-child adjustment a stack gets:
        // the flow-line height remedy is skipped, so `percent(1.0)` survives
        // for the grid track to resolve. Rewriting it to `auto` left a
        // full-bleed gradient scrim with no height source and it measured 0.
        let node: jian_ops_schema::node::PenNode = serde_json::from_str(
            r#"{"type":"rectangle","id":"scrim","x":0,"y":0,
                "width":"fill_container","height":"fill_container"}"#,
        )
        .unwrap();
        let mut style = node_to_style(&node);
        assert_eq!(style.position, Position::Absolute);
        apply_stack_child(&mut style);
        assert_eq!(style.size.height, percent(1.0_f32));
        assert_eq!(style.align_self, None);
    }

    #[test]
    fn fill_height_remedy_still_applies_to_flow_children() {
        let node: jian_ops_schema::node::PenNode =
            serde_json::from_str(r#"{"type":"rectangle","id":"col","height":"fill_container"}"#)
                .unwrap();
        let mut style = node_to_style(&node);
        assert_eq!(style.position, Position::Relative);
        apply_fill_container_axes(&mut style, &node, true);
        assert_eq!(style.align_self, Some(AlignItems::Stretch));
        assert_eq!(style.size.height, Dimension::Auto);
    }

    #[test]
    fn legacy_style_ignores_limits() {
        let container: ContainerProps =
            serde_json::from_str(r#"{"width":100,"minWidth":50,"maxHeight":40}"#).unwrap();
        let style = container_to_style(&container);
        assert_eq!(style.min_size.width, Dimension::Auto);
        assert_eq!(style.max_size.height, Dimension::Auto);
        assert_eq!(style.max_size.width, Dimension::Auto);
    }

    #[test]
    fn responsive_style_maps_sanitized_limits() {
        let node: jian_ops_schema::node::PenNode = serde_json::from_str(
            r#"{"type":"rectangle","id":"n","width":100,"minWidth":50,"maxHeight":40}"#,
        )
        .unwrap();
        let mut warnings = Vec::new();
        let style = node_to_style_responsive(&node, &mut warnings);
        assert_eq!(style.min_size.width, length(50.0_f32));
        assert_eq!(style.max_size.height, length(40.0_f32));
        assert!(warnings.is_empty());
    }

    #[test]
    fn responsive_style_drops_negative_limits_with_warning() {
        let node: jian_ops_schema::node::PenNode =
            serde_json::from_str(r#"{"type":"rectangle","id":"n","width":100,"minWidth":-5}"#)
                .unwrap();
        let mut warnings = Vec::new();
        let style = node_to_style_responsive(&node, &mut warnings);
        assert_eq!(style.min_size.width, Dimension::Auto);
        assert_eq!(warnings.len(), 1);
    }
}
