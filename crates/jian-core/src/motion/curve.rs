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

/// The first progress at which `ease` reaches `output`: 0 when the curve
/// starts at or above it, 1 when it never gets there. Timelines use it to
/// place a keyframe in time, because offsets live in eased progress.
pub fn inverse(easing: Easing, output: f32) -> f32 {
    const STEPS: u32 = 1024;
    if ease(easing, 0.0) >= output {
        return 0.0;
    }
    let mut previous = 0.0;
    for step in 1..=STEPS {
        let progress = step as f32 / STEPS as f32;
        if ease(easing, progress) >= output {
            let (mut low, mut high) = (previous, progress);
            for _ in 0..24 {
                let middle = (low + high) / 2.0;
                if ease(easing, middle) >= output {
                    high = middle;
                } else {
                    low = middle;
                }
            }
            return high;
        }
        previous = progress;
    }
    1.0
}

/// One cubic-bezier piece of a curve: on progress `x0..x1` the curve runs
/// from `y0` to `y1` along `cubic-bezier(control)` normalized to that box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicPiece {
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
    pub control: [f32; 4],
}

impl CubicPiece {
    /// The piece's value at `progress`, solving x precisely (no clamping).
    pub fn eval(&self, progress: f32) -> f32 {
        let span = (self.x1 - self.x0).max(f32::EPSILON);
        let local = ((progress - self.x0) / span).clamp(0.0, 1.0);
        let [x1, y1, x2, y2] = self.control;
        let (mut low, mut high) = (0.0_f32, 1.0_f32);
        for _ in 0..40 {
            let t = (low + high) / 2.0;
            if bezier(t, x1, x2) < local {
                low = t;
            } else {
                high = t;
            }
        }
        self.y0 + (self.y1 - self.y0) * bezier((low + high) / 2.0, y1, y2)
    }
}

/// The curve as exact cubic-bezier pieces. The polynomial curves are cubic
/// in disguise (x(t) = t with controls 1/3 and 2/3); `easeInOut` splits at
/// 0.5. Bezier curves come back unclamped: the runtime also clamps their
/// output to [0, 1], which the curve compiler handles separately.
pub fn as_cubic(easing: Easing) -> Vec<CubicPiece> {
    const THIRD: f32 = 1.0 / 3.0;
    const TWO_THIRDS: f32 = 2.0 / 3.0;
    let whole = |control: [f32; 4]| {
        vec![CubicPiece {
            x0: 0.0,
            x1: 1.0,
            y0: 0.0,
            y1: 1.0,
            control,
        }]
    };
    match easing {
        Easing::Linear => whole([THIRD, THIRD, TWO_THIRDS, TWO_THIRDS]),
        Easing::Ease => whole([THIRD, 0.0, TWO_THIRDS, 1.0]),
        Easing::EaseIn => whole([THIRD, 0.0, TWO_THIRDS, THIRD]),
        Easing::EaseOut => whole([THIRD, TWO_THIRDS, TWO_THIRDS, 1.0]),
        Easing::EaseInOut => vec![
            CubicPiece {
                x0: 0.0,
                x1: 0.5,
                y0: 0.0,
                y1: 0.5,
                control: [THIRD, 0.0, TWO_THIRDS, THIRD],
            },
            CubicPiece {
                x0: 0.5,
                x1: 1.0,
                y0: 0.5,
                y1: 1.0,
                control: [THIRD, TWO_THIRDS, TWO_THIRDS, 1.0],
            },
        ],
        Easing::Standard | Easing::Emphasized => whole([0.2, 0.0, 0.0, 1.0]),
        Easing::EmphasizedDecelerate => whole([0.05, 0.7, 0.1, 1.0]),
        Easing::EmphasizedAccelerate => whole([0.3, 0.0, 0.8, 0.15]),
        Easing::CubicBezier(x1, y1, x2, y2) => whole([x1, y1, x2, y2]),
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

    const ALL: [Easing; 12] = [
        Easing::Linear,
        Easing::Ease,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
        Easing::Standard,
        Easing::Emphasized,
        Easing::EmphasizedDecelerate,
        Easing::EmphasizedAccelerate,
        Easing::CubicBezier(0.25, 0.1, 0.25, 1.0),
        Easing::CubicBezier(0.34, 1.56, 0.64, 1.0),
        Easing::CubicBezier(0.6, -0.28, 0.735, 0.045),
    ];

    fn piece_value(pieces: &[CubicPiece], progress: f32) -> f32 {
        let piece = pieces
            .iter()
            .find(|piece| progress <= piece.x1)
            .unwrap_or_else(|| pieces.last().expect("pieces"));
        piece.eval(progress)
    }

    #[test]
    fn cubic_pieces_reproduce_the_runtime_curves() {
        for easing in ALL {
            let pieces = as_cubic(easing);
            let polynomial = matches!(
                easing,
                Easing::Linear
                    | Easing::Ease
                    | Easing::EaseIn
                    | Easing::EaseOut
                    | Easing::EaseInOut
            );
            // The runtime solves bezier x with only 16 bisection steps.
            let tolerance = if polynomial { 1e-5 } else { 5e-4 };
            for step in 0..=1000 {
                let progress = step as f32 / 1000.0;
                let mut expected = piece_value(&pieces, progress);
                if !polynomial {
                    expected = expected.clamp(0.0, 1.0);
                }
                let actual = ease(easing, progress);
                assert!(
                    (actual - expected).abs() <= tolerance,
                    "{easing:?} at {progress}: {actual} vs {expected}"
                );
            }
        }
    }

    #[test]
    fn inverse_finds_the_first_time_a_curve_reaches_a_value() {
        for easing in ALL {
            for step in 1..100 {
                let target = step as f32 / 100.0;
                let progress = inverse(easing, target);
                assert!(
                    ease(easing, progress) >= target - 1e-4,
                    "{easing:?} {target}"
                );
                if progress > 0.0 {
                    assert!(
                        ease(easing, progress - 2e-3) < target + 1e-4,
                        "{easing:?} {target}"
                    );
                }
            }
        }
        assert_eq!(inverse(Easing::Linear, 0.0), 0.0);
        assert_eq!(inverse(Easing::Linear, 2.0), 1.0);
        assert!((inverse(Easing::Linear, 0.25) - 0.25).abs() < 1e-5);
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
