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
        Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)) => percent(1.0),
        // Expression-sized nodes get auto; the runtime re-resolves once the
        // expression is evaluated and calls LayoutEngine::mark_dirty.
        Some(SizingBehavior::Expression(_)) => auto(),
        None => auto(),
    }
}

pub fn resolve_padding(p: Option<&OpsPadding>) -> Rect<LengthPercentage> {
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
        Some(OpsPadding::XY([x, y])) => {
            let lx: LengthPercentage = length(*x as f32);
            let ly: LengthPercentage = length(*y as f32);
            Rect {
                left: lx,
                right: lx,
                top: ly,
                bottom: ly,
            }
        }
        Some(OpsPadding::LtrB([l, t, r, b])) => Rect {
            left: length(*l as f32),
            top: length(*t as f32),
            right: length(*r as f32),
            bottom: length(*b as f32),
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
        _ => AlignItems::FlexStart,
    }
}

pub fn container_to_style(c: &ContainerProps) -> Style {
    Style {
        display: Display::Flex,
        size: Size {
            width: resolve_sizing(c.width.as_ref()),
            height: resolve_sizing(c.height.as_ref()),
        },
        padding: resolve_padding(c.padding.as_ref()),
        flex_direction: resolve_flex_direction(c.layout.as_ref()),
        justify_content: Some(resolve_justify(c.justify_content.as_ref())),
        align_items: Some(resolve_align(c.align_items.as_ref())),
        gap: zero(),
        ..Default::default()
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
    fn padding_ltrb() {
        let p = OpsPadding::LtrB([1.0, 2.0, 3.0, 4.0]);
        let r = resolve_padding(Some(&p));
        assert_eq!(r.left, lp_len(1.0));
        assert_eq!(r.top, lp_len(2.0));
        assert_eq!(r.right, lp_len(3.0));
        assert_eq!(r.bottom, lp_len(4.0));
    }
}
