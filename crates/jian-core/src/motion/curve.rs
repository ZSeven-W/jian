//! Easing curves for document motion and UI transitions.
//!
//! `ease` is the runtime curve every motion consumer samples. It is the
//! preview implementation moved from OpenPencil unchanged, quirks included:
//! the named curves are polynomials rather than the CSS keyword curves, and a
//! cubic-bezier solves x with 16 bisection steps and clamps y to [0, 1].
//! Exporters that need CSS-compatible curves compile these shapes instead of
//! substituting CSS keywords (see `motion::compile`).

use crate::action::services::Easing;
use jian_ops_schema::motion::Easing as SchemaEasing;

/// The single conversion from a document easing to the runtime easing.
impl From<&SchemaEasing> for Easing {
    fn from(easing: &SchemaEasing) -> Self {
        match easing {
            SchemaEasing::Linear => Easing::Linear,
            SchemaEasing::Ease => Easing::Ease,
            SchemaEasing::EaseIn => Easing::EaseIn,
            SchemaEasing::EaseOut => Easing::EaseOut,
            SchemaEasing::EaseInOut => Easing::EaseInOut,
            SchemaEasing::Standard => Easing::Standard,
            SchemaEasing::Emphasized => Easing::Emphasized,
            SchemaEasing::EmphasizedDecelerate => Easing::EmphasizedDecelerate,
            SchemaEasing::EmphasizedAccelerate => Easing::EmphasizedAccelerate,
            SchemaEasing::CubicBezier(a, b, c, d) => Easing::CubicBezier(*a, *b, *c, *d),
        }
    }
}

/// Parse an authored easing name with the same names and aliases a document
/// accepts.
pub fn parse_easing_name(authored: &str) -> Option<Easing> {
    serde_json::from_value::<SchemaEasing>(serde_json::Value::String(authored.to_owned()))
        .ok()
        .map(|easing| Easing::from(&easing))
}

/// Eased progress for `progress` in [0, 1].
pub fn ease(easing: Easing, progress: f32) -> f32 {
    match easing {
        Easing::Linear => progress,
        Easing::Ease => progress * progress * (3.0 - 2.0 * progress),
        Easing::EaseIn => progress * progress,
        Easing::EaseOut => 1.0 - (1.0 - progress) * (1.0 - progress),
        Easing::EaseInOut if progress < 0.5 => 2.0 * progress * progress,
        Easing::EaseInOut => 1.0 - (-2.0 * progress + 2.0).powi(2) / 2.0,
        Easing::Standard => cubic_bezier(0.2, 0.0, 0.0, 1.0, progress),
        Easing::Emphasized => cubic_bezier(0.2, 0.0, 0.0, 1.0, progress),
        Easing::EmphasizedDecelerate => cubic_bezier(0.05, 0.7, 0.1, 1.0, progress),
        Easing::EmphasizedAccelerate => cubic_bezier(0.3, 0.0, 0.8, 0.15, progress),
        Easing::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, progress),
    }
}

/// Ease-out cubic `1 − (1 − t)³` for UI transitions (panels, drawers,
/// screen push / pop). Callers clamp `t` when it can leave [0, 1].
pub fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Ease-in-out cubic, clamped to [0, 1]: slow start, fast middle, soft end.
pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let u = -2.0 * t + 2.0;
        1.0 - u * u * u / 2.0
    }
}

pub(crate) fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..16 {
        let t = (low + high) / 2.0;
        let curve_x = bezier(t, x1, x2);
        if curve_x < x {
            low = t;
        } else {
            high = t;
        }
    }
    bezier((low + high) / 2.0, y1, y2).clamp(0.0, 1.0)
}

pub(crate) fn bezier(t: f32, first: f32, second: f32) -> f32 {
    let inverse = 1.0 - t;
    3.0 * inverse * inverse * t * first + 3.0 * inverse * t * t * second + t * t * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_document_easing_name_parses_like_the_schema() {
        for (name, expected) in [
            ("linear", Easing::Linear),
            ("ease_in", Easing::EaseIn),
            ("ease-in", Easing::EaseIn),
            ("easeIn", Easing::EaseIn),
            ("emphasizedDecelerate", Easing::EmphasizedDecelerate),
            ("emphasized_decelerate", Easing::EmphasizedDecelerate),
        ] {
            assert_eq!(parse_easing_name(name), Some(expected), "{name}");
        }
        assert_eq!(parse_easing_name("bouncy"), None);
    }

    #[test]
    fn ui_cubic_curves_hit_their_endpoints() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_in_out_cubic(0.0), 0.0);
        assert_eq!(ease_in_out_cubic(1.0), 1.0);
        assert_eq!(ease_in_out_cubic(2.0), 1.0);
    }
}
