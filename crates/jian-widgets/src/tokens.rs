//! Theme tokens for reusable components.

use crate::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    #[default]
    Desktop,
    Touch,
}

impl Density {
    pub fn row_height(self) -> f32 {
        match self {
            Density::Desktop => 30.0,
            Density::Touch => 44.0,
        }
    }

    pub fn min_hit_target(self) -> f32 {
        match self {
            Density::Desktop => 20.0,
            Density::Touch => 44.0,
        }
    }

    pub fn font_size(self) -> f32 {
        match self {
            Density::Desktop => 13.0,
            Density::Touch => 15.0,
        }
    }
}

/// Shared shadcn-style component tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub background: Color,
    pub foreground: Color,
    pub card: Color,
    pub card_foreground: Color,
    pub popover: Color,
    pub popover_foreground: Color,
    pub primary: Color,
    pub primary_foreground: Color,
    pub muted: Color,
    pub muted_foreground: Color,
    pub border: Color,
    pub accent: Color,
    pub accent_foreground: Color,
    pub destructive: Color,
    pub button_hover: Color,
    pub row_selected: Color,
    pub row_selected_primary: Color,
    pub density: Density,
}

impl Tokens {
    /// Dark palette copied from OP's shadcn theme.
    pub const fn dark() -> Self {
        Self {
            background: Color::rgb_u8(0x0a, 0x0a, 0x0a),
            foreground: Color::rgb_u8(0xfa, 0xfa, 0xfa),
            card: Color::rgb_u8(0x14, 0x14, 0x14),
            card_foreground: Color::rgb_u8(0xfa, 0xfa, 0xfa),
            popover: Color::rgb_u8(0x1a, 0x1a, 0x1a),
            popover_foreground: Color::rgb_u8(0xfa, 0xfa, 0xfa),
            primary: Color::rgb_u8(0x3b, 0x82, 0xf6),
            primary_foreground: Color::rgb_u8(0xff, 0xff, 0xff),
            muted: Color::rgb_u8(0x1f, 0x1f, 0x1f),
            muted_foreground: Color::rgb_u8(0x9a, 0x9a, 0x9a),
            border: Color::rgb_u8(0x26, 0x26, 0x26),
            accent: Color::rgb_u8(0x26, 0x26, 0x26),
            accent_foreground: Color::rgb_u8(0xfa, 0xfa, 0xfa),
            destructive: Color::rgb_u8(0xef, 0x44, 0x44),
            button_hover: Color::rgba_u8(0xff, 0xff, 0xff, 0.06),
            row_selected: Color::rgb_u8(0x26, 0x26, 0x26),
            row_selected_primary: Color::rgba_u8(0x3b, 0x82, 0xf6, 0.18),
            density: Density::Desktop,
        }
    }

    /// Light palette copied from OP's shadcn theme.
    pub const fn light() -> Self {
        Self {
            background: Color::rgb_u8(0xff, 0xff, 0xff),
            foreground: Color::rgb_u8(0x0a, 0x0a, 0x0a),
            card: Color::rgb_u8(0xff, 0xff, 0xff),
            card_foreground: Color::rgb_u8(0x0a, 0x0a, 0x0a),
            popover: Color::rgb_u8(0xff, 0xff, 0xff),
            popover_foreground: Color::rgb_u8(0x0a, 0x0a, 0x0a),
            primary: Color::rgb_u8(0x3b, 0x82, 0xf6),
            primary_foreground: Color::rgb_u8(0xff, 0xff, 0xff),
            muted: Color::rgb_u8(0xf5, 0xf5, 0xf5),
            muted_foreground: Color::rgb_u8(0x73, 0x73, 0x73),
            border: Color::rgb_u8(0xe5, 0xe5, 0xe5),
            accent: Color::rgb_u8(0xf5, 0xf5, 0xf5),
            accent_foreground: Color::rgb_u8(0x0a, 0x0a, 0x0a),
            destructive: Color::rgb_u8(0xef, 0x44, 0x44),
            button_hover: Color::rgba_u8(0x00, 0x00, 0x00, 0.06),
            row_selected: Color::rgb_u8(0xe5, 0xe5, 0xe5),
            row_selected_primary: Color::rgba_u8(0x3b, 0x82, 0xf6, 0.15),
            density: Density::Desktop,
        }
    }
}

impl Default for Tokens {
    fn default() -> Self {
        Self::dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_tokens_copy_op_shadcn_values_without_canvas_surface() {
        let t = Tokens::dark();

        assert_eq!(t.background, crate::Color::rgb_u8(0x0a, 0x0a, 0x0a));
        assert_eq!(t.foreground, crate::Color::rgb_u8(0xfa, 0xfa, 0xfa));
        assert_eq!(t.primary, crate::Color::rgb_u8(0x3b, 0x82, 0xf6));
        assert_eq!(
            t.button_hover,
            crate::Color::rgba_u8(0xff, 0xff, 0xff, 0.06)
        );
        assert_eq!(
            t.row_selected_primary,
            crate::Color::rgba_u8(0x3b, 0x82, 0xf6, 0.18)
        );
        assert_eq!(t.density, Density::Desktop);
    }

    #[test]
    fn light_tokens_copy_op_shadcn_values_without_canvas_surface() {
        let t = Tokens::light();

        assert_eq!(t.background, crate::Color::rgb_u8(0xff, 0xff, 0xff));
        assert_eq!(t.foreground, crate::Color::rgb_u8(0x0a, 0x0a, 0x0a));
        assert_eq!(t.border, crate::Color::rgb_u8(0xe5, 0xe5, 0xe5));
        assert_eq!(
            t.button_hover,
            crate::Color::rgba_u8(0x00, 0x00, 0x00, 0.06)
        );
        assert_eq!(
            t.row_selected_primary,
            crate::Color::rgba_u8(0x3b, 0x82, 0xf6, 0.15)
        );
        assert_eq!(t.density, Density::Desktop);
    }

    #[test]
    fn touch_density_uses_platform_minimum_hit_targets() {
        assert_eq!(Density::Desktop.row_height(), 30.0);
        assert_eq!(Density::Desktop.min_hit_target(), 20.0);
        assert_eq!(Density::Desktop.font_size(), 13.0);
        assert_eq!(Density::Touch.row_height(), 44.0);
        assert!(Density::Touch.min_hit_target() >= 44.0);
        assert_eq!(Density::Touch.font_size(), 15.0);
    }
}
