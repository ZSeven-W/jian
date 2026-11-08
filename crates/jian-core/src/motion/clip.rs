//! Stateless sampling of a screen's motion at any time, for a canvas
//! timeline scrubber and offline export.
//!
//! A [`Clip`] collects the animations one trigger starts under a root node
//! and samples them with the runtime's own track sampler, so `sample(t)`
//! equals the timeline's overrides after the trigger fired at 0 and the clock
//! reached `t`. Clips skip the runtime's per-screen track budget: an editor
//! or exporter shows every declared animation.

use super::curve::inverse;
use super::scene_view::MotionSceneView;
use super::timeline::sample_request;
use crate::action::animation_registry::animatable_property_registry;
use crate::action::services::{
    AnimationDirection, AnimationFillMode, AnimationProperty, AnimationRequest, Easing,
};
use crate::document::tree::node_schema_id;
use jian_ops_schema::motion::{
    Iterations, MotionTrigger, NodeAnimation, NodeAnimationFillMode, P1_MOTION_PROPERTIES,
};
use jian_ops_schema::node::PenNode;
use serde_json::Value;
use std::collections::BTreeMap;

/// Which animations a clip collects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipTrigger {
    Mount,
    InView,
    /// Manual animations with this id; nodes sharing it form one sequence.
    Manual(String),
}

impl ClipTrigger {
    fn matches(&self, animation: &NodeAnimation) -> bool {
        match self {
            Self::Mount => animation.trigger == MotionTrigger::Mount,
            Self::InView => animation.trigger == MotionTrigger::InView,
            Self::Manual(id) => {
                animation.trigger == MotionTrigger::Manual
                    && animation.id.as_deref() == Some(id.as_str())
            }
        }
    }
}

/// Overrides keyed like the timeline's: `(node id, property) -> value`.
pub type MotionOverrides = BTreeMap<(String, AnimationProperty), Value>;

/// Where one keyframe sits on the clip's time axis.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipKeyframe {
    pub node: String,
    pub animation: usize,
    pub keyframe: usize,
    pub time_ms: f32,
}

#[derive(Debug, Clone)]
struct ClipTrack {
    request: AnimationRequest,
    iterations: Iterations,
}

#[derive(Debug, Clone)]
struct ClipAnimation {
    node: String,
    index: usize,
    declaration: NodeAnimation,
}

#[derive(Debug, Clone, Default)]
pub struct Clip {
    tracks: Vec<ClipTrack>,
    animations: Vec<ClipAnimation>,
}

impl Clip {
    /// Collect the animations `trigger` starts on `root` and its descendants.
    pub fn build(root: &PenNode, trigger: &ClipTrigger) -> Self {
        let mut clip = Self::default();
        clip.collect(root, trigger);
        clip
    }

    fn collect(&mut self, node: &PenNode, trigger: &ClipTrigger) {
        let id = node_schema_id(node).to_owned();
        for (index, declaration) in node
            .motion_declarations()
            .1
            .into_iter()
            .flatten()
            .enumerate()
        {
            if !trigger.matches(declaration) {
                continue;
            }
            self.animations.push(ClipAnimation {
                node: id.clone(),
                index,
                declaration: declaration.clone(),
            });
            self.tracks.extend(tracks_for(&id, declaration));
        }
        for child in node.children_ref().into_iter().flatten() {
            self.collect(child, trigger);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// The overrides at `t_ms`: every track's sample that differs from the
    /// node's painted base value in `view`.
    pub fn sample(&self, t_ms: u64, view: &dyn MotionSceneView) -> MotionOverrides {
        let mut overrides = MotionOverrides::new();
        for track in &self.tracks {
            let target = &track.request.target;
            let property = &track.request.property;
            let Some(base) = view.property(target, property) else {
                continue;
            };
            if let (Some(value), _) = sample_request(&track.request, t_ms) {
                if value != base {
                    overrides.insert((target.clone(), property.clone()), value);
                }
            }
        }
        overrides
    }

    /// When the last finite animation ends; `None` when every animation
    /// loops forever.
    pub fn duration_ms(&self) -> Option<u64> {
        let finite: Vec<u64> = self
            .tracks
            .iter()
            .filter(|track| !track.iterations.is_infinite())
            .map(|track| {
                track.request.requested_at_ms.saturating_add(
                    track
                        .request
                        .duration_ms
                        .saturating_mul(u64::from(track.request.iterations)),
                )
            })
            .collect();
        if finite.is_empty() && !self.tracks.is_empty() {
            return None;
        }
        Some(finite.into_iter().max().unwrap_or(0))
    }

    /// Every keyframe's time on the clip axis (first pass): offsets live in
    /// eased progress, so `delay + duration × inverse(offset)`.
    pub fn keyframe_times(&self) -> Vec<ClipKeyframe> {
        let mut times = Vec::new();
        for animation in &self.animations {
            let declaration = &animation.declaration;
            let easing = Easing::from(&declaration.easing);
            for (keyframe, frame) in declaration.keyframes.iter().enumerate() {
                times.push(ClipKeyframe {
                    node: animation.node.clone(),
                    animation: animation.index,
                    keyframe,
                    time_ms: declaration.delay_ms as f32
                        + declaration.duration_ms as f32 * inverse(easing, frame.offset),
                });
            }
        }
        times
    }
}

/// The per-property tracks one declaration starts, timed from the trigger,
/// built exactly as the triggers build them.
fn tracks_for(target: &str, declaration: &NodeAnimation) -> Vec<ClipTrack> {
    let mut by_property: BTreeMap<String, Vec<(f32, Value)>> = BTreeMap::new();
    for keyframe in &declaration.keyframes {
        for (property, value) in &keyframe.values {
            by_property
                .entry(property.clone())
                .or_default()
                .push((keyframe.offset, value.clone()));
        }
    }
    let mut tracks = Vec::new();
    for (name, stops) in by_property {
        if !P1_MOTION_PROPERTIES.contains(&name.as_str()) {
            continue;
        }
        let Some(descriptor) = animatable_property_registry().get(&name) else {
            continue;
        };
        let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
            continue;
        };
        let request = AnimationRequest {
            target: target.to_owned(),
            property: AnimationProperty::from_registered(&name, descriptor.apply),
            from: Some(first.1.clone()),
            to: last.1.clone(),
            stops: Some(stops.clone()),
            duration_ms: declaration.duration_ms,
            delay_ms: 0,
            easing: Easing::from(&declaration.easing),
            iterations: declaration.iterations.count_or(u32::MAX).max(1),
            direction: AnimationDirection::Normal,
            fill_mode: match declaration.fill_mode {
                NodeAnimationFillMode::Forwards => AnimationFillMode::Both,
                NodeAnimationFillMode::None => AnimationFillMode::Backwards,
            },
            requested_at_ms: declaration.delay_ms,
        };
        tracks.push(ClipTrack {
            request,
            iterations: declaration.iterations,
        });
    }
    tracks
}

#[cfg(test)]
#[path = "clip_tests.rs"]
mod tests;
