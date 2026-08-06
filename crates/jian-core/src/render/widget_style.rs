//! Interaction-state style composition for widget nodes.
//!
//! Order (spec §6): authored node style → auto-derived state visuals
//! → authored `states` override block, field by field.

use crate::scene::Color;
use jian_ops_schema::state_override::{StyleOverride, WidgetStates};

/// Legacy defaults for first-class widgets that predate authored visual
/// styles. Keeping the fallbacks here makes the resolver additive for old
/// documents while letting every renderer share the same semantics.
const LEGACY_ACTIVE: Color = Color::rgb(0x3b, 0x82, 0xf6);
const LEGACY_INACTIVE: Color = Color::rgb(0xd1, 0xd5, 0xdb);
const LEGACY_FOREGROUND: Color = Color::rgb(0x9c, 0xa3, 0xaf);
const DERIVED_INACTIVE_ALPHA: u8 = 0x59;

/// Resolved visual roles for a first-class widget.
///
/// The canonical widget schema intentionally exposes one authored `fill` and
/// one authored `stroke` rather than per-part paint fields. Their meanings are:
///
/// - `fill` is the active/accent paint;
/// - `stroke` is the inactive track / border paint;
/// - thumb, check-mark and text colours are contrast-derived.
///
/// `surface` and `border` preserve whether the author actually supplied those
/// paints. Select-like controls must use those optional fields so an unstyled
/// select remains transparent and borderless; `active` / `inactive` carry
/// backwards-compatible defaults for controls that always need a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthoredWidgetVisual {
    pub active: Color,
    pub inactive: Color,
    pub active_foreground: Color,
    pub inactive_foreground: Color,
    pub surface: Option<Color>,
    pub border: Option<Color>,
    /// Foreground painted on the authored surface (select/input value).
    pub foreground: Color,
    /// Muted foreground painted on the authored surface (placeholder/icon).
    pub muted_foreground: Color,
    /// Foreground adjacent to, rather than on top of, the authored surface.
    /// Checkbox/radio labels must not contrast against the accent swatch.
    pub label_foreground: Color,
    pub muted_label_foreground: Color,
}

/// Resolve authored widget paints into semantic visual roles.
///
/// This function accepts already-resolved colours so callers operating on a
/// JSON document, an OpenPencil scene, or another adapter can share the exact
/// same colour policy without reconstructing schema JSON.
pub fn resolve_authored_widget_visual(
    fill: Option<Color>,
    stroke: Option<Color>,
) -> AuthoredWidgetVisual {
    let active = fill.unwrap_or(LEGACY_ACTIVE);
    let inactive = stroke.unwrap_or_else(|| {
        fill.map(derive_inactive_from_active)
            .unwrap_or(LEGACY_INACTIVE)
    });
    let active_foreground = contrast_foreground(active);
    let inactive_foreground = contrast_foreground(inactive);
    let foreground = fill
        .filter(|surface| surface.a() >= 0x80)
        .map(contrast_foreground)
        .unwrap_or(LEGACY_FOREGROUND);
    let muted_foreground = multiply_alpha(foreground, 0xa6);
    let label_foreground = stroke.map(contrast_foreground).unwrap_or(LEGACY_FOREGROUND);
    let muted_label_foreground = multiply_alpha(label_foreground, 0xa6);

    AuthoredWidgetVisual {
        active,
        inactive,
        active_foreground,
        inactive_foreground,
        surface: fill,
        border: stroke,
        foreground,
        muted_foreground,
        label_foreground,
        muted_label_foreground,
    }
}

/// Keep an authored accent's hue for an inactive track while reducing its
/// visual weight. Multiplying rather than replacing alpha preserves intent
/// when the author already supplied a translucent fill.
fn derive_inactive_from_active(active: Color) -> Color {
    multiply_alpha(active, DERIVED_INACTIVE_ALPHA)
}

fn multiply_alpha(color: Color, alpha: u8) -> Color {
    let combined = ((u16::from(color.a()) * u16::from(alpha)) / 0xff) as u8;
    Color::rgba(color.r(), color.g(), color.b(), combined)
}

/// Multiply a derived visual colour by node opacity exactly once.
///
/// Paint-backed primitives should keep node opacity in `Paint::opacity` so
/// backends can compose it with the authored colour alpha. Text and other
/// primitives without a paint-level opacity field use this helper directly.
pub fn with_visual_opacity(color: Color, opacity: f32) -> Color {
    let opacity = opacity.clamp(0.0, 1.0);
    let alpha = (f32::from(color.a()) * opacity).round() as u8;
    Color::rgba(color.r(), color.g(), color.b(), alpha)
}

/// Pick black or white by WCAG relative contrast against `background`.
/// Alpha is deliberately ignored: the widget schema does not carry the
/// eventual parent backdrop at this layer, while RGB still gives callers a
/// stable and deterministic authored-colour policy.
pub fn contrast_foreground(background: Color) -> Color {
    let luminance = relative_luminance(background);
    let white_contrast = 1.05 / (luminance + 0.05);
    let black_contrast = (luminance + 0.05) / 0.05;
    if white_contrast >= black_contrast {
        Color::rgb(0xff, 0xff, 0xff)
    } else {
        Color::rgb(0x00, 0x00, 0x00)
    }
}

fn relative_luminance(color: Color) -> f64 {
    fn linear(byte: u8) -> f64 {
        let channel = f64::from(byte) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

/// Host-injectable knobs for the auto-derived visuals. Defaults are
/// dark-UI friendly; jian-widgets Tokens can map onto this later.
#[derive(Debug, Clone, Copy)]
pub struct WidgetTheme {
    pub hover_overlay: Color,
    pub pressed_overlay: Color,
    pub focus_ring: Color,
    pub selection: Color,
    pub disabled_alpha: f32,
}

impl Default for WidgetTheme {
    fn default() -> Self {
        Self {
            hover_overlay: Color::rgba(255, 255, 255, 15),
            pressed_overlay: Color::rgba(255, 255, 255, 31),
            focus_ring: Color::rgba(59, 130, 246, 255),
            selection: Color::rgba(59, 130, 246, 89),
            disabled_alpha: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InteractionState {
    pub hovered: bool,
    pub pressed: bool,
    pub focused: bool,
    pub disabled: bool,
}

/// Pick the authored override for the highest-priority active state:
/// disabled > pressed > hover > focused.
pub fn active_override(
    states: Option<&WidgetStates>,
    s: InteractionState,
) -> Option<&StyleOverride> {
    let st = states?;
    if s.disabled {
        if let Some(o) = st.disabled.as_ref() {
            return Some(o);
        }
    }
    if s.pressed {
        if let Some(o) = st.pressed.as_ref() {
            return Some(o);
        }
    }
    if s.hovered {
        if let Some(o) = st.hover.as_ref() {
            return Some(o);
        }
    }
    if s.focused {
        if let Some(o) = st.focused.as_ref() {
            return Some(o);
        }
    }
    None
}

/// Derived overlay to composite over the authored fill (None = no
/// overlay). Authored overrides suppress the derived overlay for the
/// same state.
pub fn derived_overlay(
    theme: &WidgetTheme,
    s: InteractionState,
    overridden: bool,
) -> Option<Color> {
    if overridden || s.disabled {
        return None;
    }
    if s.pressed {
        Some(theme.pressed_overlay)
    } else if s.hovered {
        Some(theme.hover_overlay)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_fill_and_stroke_map_to_visual_roles_exactly() {
        let active = Color::rgb(0x12, 0x08, 0x26);
        let inactive = Color::rgb(0xf4, 0xf4, 0xf5);
        let visual = resolve_authored_widget_visual(Some(active), Some(inactive));

        assert_eq!(visual.active, active);
        assert_eq!(visual.inactive, inactive);
        assert_eq!(visual.surface, Some(active));
        assert_eq!(visual.border, Some(inactive));
        assert_eq!(visual.active_foreground, Color::rgb(0xff, 0xff, 0xff));
        assert_eq!(visual.inactive_foreground, Color::rgb(0x00, 0x00, 0x00));
        assert_eq!(visual.foreground, Color::rgb(0xff, 0xff, 0xff));
        assert_eq!(visual.muted_foreground, Color::rgba(0xff, 0xff, 0xff, 0xa6));
        assert_eq!(visual.label_foreground, Color::rgb(0x00, 0x00, 0x00));
        assert_eq!(
            visual.muted_label_foreground,
            Color::rgba(0x00, 0x00, 0x00, 0xa6)
        );
    }

    #[test]
    fn absent_authored_paints_keep_track_defaults_but_no_surface_or_border() {
        let visual = resolve_authored_widget_visual(None, None);

        assert_eq!(visual.active, Color::rgb(0x3b, 0x82, 0xf6));
        assert_eq!(visual.inactive, Color::rgb(0xd1, 0xd5, 0xdb));
        assert_eq!(visual.surface, None);
        assert_eq!(visual.border, None);
        assert_eq!(visual.foreground, Color::rgb(0x9c, 0xa3, 0xaf));
        assert_eq!(visual.muted_foreground, Color::rgba(0x9c, 0xa3, 0xaf, 0xa6));
        assert_eq!(visual.label_foreground, Color::rgb(0x9c, 0xa3, 0xaf));
    }

    #[test]
    fn fill_without_stroke_derives_a_tinted_inactive_track() {
        let accent = Color::rgb(0x7c, 0x3a, 0xed);
        let visual = resolve_authored_widget_visual(Some(accent), None);

        assert_eq!(visual.active, accent);
        assert_eq!(visual.inactive, Color::rgba(0x7c, 0x3a, 0xed, 0x59));
        assert_eq!(visual.inactive_foreground, Color::rgb(0xff, 0xff, 0xff));
        assert_eq!(visual.surface, Some(accent));
        assert_eq!(visual.border, None);
    }

    #[test]
    fn contrast_foreground_chooses_exact_black_or_white() {
        assert_eq!(
            contrast_foreground(Color::rgb(0x05, 0x05, 0x08)),
            Color::rgb(0xff, 0xff, 0xff)
        );
        assert_eq!(
            contrast_foreground(Color::rgb(0xf8, 0xfa, 0xfc)),
            Color::rgb(0x00, 0x00, 0x00)
        );
    }

    #[test]
    fn transparent_surface_or_stroke_only_never_invents_surface_contrast() {
        let transparent =
            resolve_authored_widget_visual(Some(Color::rgba(0x12, 0x08, 0x26, 0x00)), None);
        let stroke_only = resolve_authored_widget_visual(None, Some(Color::rgb(0x12, 0x08, 0x26)));

        assert_eq!(transparent.foreground, Color::rgb(0x9c, 0xa3, 0xaf));
        assert_eq!(stroke_only.foreground, Color::rgb(0x9c, 0xa3, 0xaf));
    }

    #[test]
    fn visual_opacity_multiplies_existing_alpha_once() {
        assert_eq!(
            with_visual_opacity(Color::rgba(0x7c, 0x3a, 0xed, 0x80), 0.5),
            Color::rgba(0x7c, 0x3a, 0xed, 0x40)
        );
    }

    #[test]
    fn priority_disabled_beats_pressed() {
        let st: WidgetStates =
            serde_json::from_str(r#"{"pressed":{"opacity":0.8},"disabled":{"opacity":0.4}}"#)
                .unwrap();
        let s = InteractionState {
            pressed: true,
            disabled: true,
            ..Default::default()
        };
        assert_eq!(active_override(Some(&st), s).unwrap().opacity, Some(0.4));
    }

    #[test]
    fn derived_overlay_suppressed_by_override() {
        let t = WidgetTheme::default();
        let s = InteractionState {
            hovered: true,
            ..Default::default()
        };
        assert!(derived_overlay(&t, s, false).is_some());
        assert!(derived_overlay(&t, s, true).is_none());
    }
}
