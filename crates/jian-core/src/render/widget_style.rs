//! Interaction-state style composition for widget nodes.
//!
//! Order (spec §6): authored node style → auto-derived state visuals
//! → authored `states` override block, field by field.

use crate::scene::Color;
use jian_ops_schema::state_override::{StyleOverride, WidgetStates};

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
