//! Screen transitions: how a navigation animates the screen it leaves and
//! the screen it enters.
//!
//! A navigate / back action may carry a [`ScreenTransitionSpec`]. Resolving
//! it against the router verb gives a [`ResolvedScreenTransition`] whose
//! [`layers_at`](ResolvedScreenTransition::layers_at) places both screens for
//! a host to composite. Without a spec the result is OpenPencil Preview's
//! original behaviour, which the recorded transition vectors pin: push and
//! pop slide for 240 ms on `1 − (1 − t)³`, replace cross-fades for 160 ms
//! linearly.
//!
//! A spec names the forward motion. Push and replace play it as written;
//! pop plays it in reverse, so a screen pushed with `slideUp` leaves
//! downward when the user goes back.

use super::curve::{ease, ease_out_cubic};
use crate::action::services::Easing;
use jian_ops_schema::motion::Easing as SchemaEasing;
use serde::{Deserialize, Serialize};

/// How long a slide runs when the spec does not say.
pub const DEFAULT_SLIDE_MS: u64 = 240;
/// How long a fade runs when the spec does not say.
pub const DEFAULT_FADE_MS: u64 = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScreenTransitionKind {
    /// Slide for push / pop, fade for replace.
    #[default]
    Default,
    /// The entering screen arrives from the right, moving left.
    SlideLeft,
    /// The entering screen arrives from the left, moving right.
    SlideRight,
    /// The entering screen arrives from below, moving up.
    SlideUp,
    /// The entering screen arrives from above, moving down.
    SlideDown,
    Fade,
    /// Switch at once.
    None,
}

/// The `transition` a navigate / back action carries. Unset fields take
/// the kind's defaults.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenTransitionSpec {
    #[serde(default)]
    pub kind: ScreenTransitionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub easing: Option<SchemaEasing>,
}

/// The router verb a transition animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationKind {
    Push,
    Pop,
    Replace,
}

/// The way content travels when a slide plays forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlideDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionEffect {
    Slide(SlideDirection),
    Fade,
    Cut,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TransitionCurve {
    /// `1 − (1 − t)³`, the original slide curve.
    EaseOutCubic,
    Linear,
    Authored(Easing),
}

/// Which screen a layer shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionScreen {
    Outgoing,
    Entering,
}

/// One screen's placement at a moment of the transition: an offset in
/// viewport pixels and an alpha for the whole screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransitionLayer {
    pub screen: TransitionScreen,
    pub dx: f32,
    pub dy: f32,
    pub alpha: f32,
}

/// A spec resolved against the router verb: what plays and for how long.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedScreenTransition {
    pub navigation: NavigationKind,
    pub effect: TransitionEffect,
    pub duration_ms: u64,
    curve: TransitionCurve,
}

impl ResolvedScreenTransition {
    pub fn resolve(spec: Option<&ScreenTransitionSpec>, navigation: NavigationKind) -> Self {
        let spec = spec.cloned().unwrap_or_default();
        let slide = |direction| (TransitionEffect::Slide(direction), DEFAULT_SLIDE_MS);
        let (effect, default_ms) = match spec.kind {
            ScreenTransitionKind::Default => match navigation {
                NavigationKind::Push | NavigationKind::Pop => slide(SlideDirection::Left),
                NavigationKind::Replace => (TransitionEffect::Fade, DEFAULT_FADE_MS),
            },
            ScreenTransitionKind::SlideLeft => slide(SlideDirection::Left),
            ScreenTransitionKind::SlideRight => slide(SlideDirection::Right),
            ScreenTransitionKind::SlideUp => slide(SlideDirection::Up),
            ScreenTransitionKind::SlideDown => slide(SlideDirection::Down),
            ScreenTransitionKind::Fade => (TransitionEffect::Fade, DEFAULT_FADE_MS),
            ScreenTransitionKind::None => (TransitionEffect::Cut, 0),
        };
        let curve = match (spec.easing.as_ref(), effect) {
            (Some(easing), _) => TransitionCurve::Authored(Easing::from(easing)),
            (None, TransitionEffect::Slide(_)) => TransitionCurve::EaseOutCubic,
            (None, _) => TransitionCurve::Linear,
        };
        let duration_ms = match effect {
            TransitionEffect::Cut => 0,
            _ => spec.duration_ms.unwrap_or(default_ms),
        };
        Self {
            navigation,
            effect,
            duration_ms,
            curve,
        }
    }

    /// Eased progress for a linear progress in [0, 1]. A zero-length
    /// transition is already complete.
    pub fn progress(&self, linear_t: f32) -> f32 {
        if self.duration_ms == 0 {
            return 1.0;
        }
        let t = linear_t.clamp(0.0, 1.0);
        match self.curve {
            TransitionCurve::EaseOutCubic => ease_out_cubic(t),
            TransitionCurve::Linear => t,
            TransitionCurve::Authored(easing) => ease(easing, t),
        }
    }

    /// Both screens, bottom layer first, at `linear_t` for a viewport of
    /// `(width, height)`.
    pub fn layers_at(&self, linear_t: f32, viewport: (f32, f32)) -> [TransitionLayer; 2] {
        let p = self.progress(linear_t);
        let still = |screen| TransitionLayer {
            screen,
            dx: 0.0,
            dy: 0.0,
            alpha: 1.0,
        };
        let moved = |screen, direction, share: f32| {
            let (dx, dy) = slide_offset(direction, share, viewport);
            TransitionLayer {
                screen,
                dx,
                dy,
                alpha: 1.0,
            }
        };
        match self.effect {
            TransitionEffect::Slide(direction) => match self.navigation {
                NavigationKind::Push | NavigationKind::Replace => [
                    still(TransitionScreen::Outgoing),
                    moved(TransitionScreen::Entering, direction, 1.0 - p),
                ],
                NavigationKind::Pop => [
                    still(TransitionScreen::Entering),
                    moved(TransitionScreen::Outgoing, direction, p),
                ],
            },
            TransitionEffect::Fade => [
                TransitionLayer {
                    alpha: 1.0 - p,
                    ..still(TransitionScreen::Outgoing)
                },
                TransitionLayer {
                    alpha: p,
                    ..still(TransitionScreen::Entering)
                },
            ],
            TransitionEffect::Cut => [
                TransitionLayer {
                    alpha: 0.0,
                    ..still(TransitionScreen::Outgoing)
                },
                still(TransitionScreen::Entering),
            ],
        }
    }
}

/// Offset of a screen that is `share` of the way off the viewport on the
/// side a forward slide in `direction` enters from.
fn slide_offset(direction: SlideDirection, share: f32, (width, height): (f32, f32)) -> (f32, f32) {
    match direction {
        SlideDirection::Left => (width * share, 0.0),
        SlideDirection::Right => (-width * share, 0.0),
        SlideDirection::Up => (0.0, height * share),
        SlideDirection::Down => (0.0, -height * share),
    }
}

#[cfg(test)]
#[path = "screen_transition_tests.rs"]
mod tests;
