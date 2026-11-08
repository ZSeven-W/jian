//! Lifecycle, in-view, transition and switch triggers for document motion,
//! moved from OpenPencil's preview. They read the painted scene through
//! [`MotionSceneView`] and start tracks on a [`MotionTimeline`].

use super::declarations::node_declarations;
use super::in_view::InViewWindow;
use super::scene_view::MotionSceneView;
use super::timeline::MotionTimeline;
use crate::action::animation_registry::animatable_property_registry;
use crate::action::services::{
    AnimationDirection, AnimationFillMode, AnimationProperty, AnimationRequest, Easing,
};
use crate::Runtime;
use jian_ops_schema::motion::{
    MotionPreference, MotionTrigger, NodeAnimation, NodeAnimationFillMode, P1_MOTION_PROPERTIES,
};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// Animation tracks one screen may run at once.
pub const MOTION_TRACK_BUDGET: usize = 32;

/// Per-screen trigger bookkeeping, cleared by [`MotionTriggers::reset_screen`].
#[derive(Default)]
struct MotionInner {
    lifecycle_started: bool,
    observed: BTreeMap<(String, AnimationProperty), Value>,
    animation_tracks: BTreeSet<(String, AnimationProperty)>,
}

#[derive(Clone, Default)]
pub struct MotionTriggers {
    inner: Rc<RefCell<MotionInner>>,
    /// Host presentation state, so it survives `reset_screen`.
    in_view_window: Rc<RefCell<Option<InViewWindow>>>,
}

impl MotionTriggers {
    pub fn reset_screen(&self) {
        *self.inner.borrow_mut() = MotionInner::default();
    }

    /// Set the host's visible window; `None` uses the framed root. Returns
    /// whether the window changed, so the host can repaint.
    pub fn set_in_view_window(&self, window: Option<InViewWindow>) -> bool {
        let mut current = self.in_view_window.borrow_mut();
        if *current == window {
            return false;
        }
        *current = window;
        true
    }

    /// Mark the screen's lifecycle as started; false when it already was.
    pub fn begin_lifecycle(&self) -> bool {
        let mut inner = self.inner.borrow_mut();
        if inner.lifecycle_started {
            return false;
        }
        inner.lifecycle_started = true;
        true
    }

    pub fn lifecycle_started(&self) -> bool {
        self.inner.borrow().lifecycle_started
    }

    fn prune_tracks(&self, animation: &MotionTimeline) {
        self.inner
            .borrow_mut()
            .animation_tracks
            .retain(|(target, property)| animation.has_active(target, property));
    }

    fn track_budget_available(
        &self,
        animation: &MotionTimeline,
        target: &str,
        property: &AnimationProperty,
    ) -> bool {
        self.prune_tracks(animation);
        let inner = self.inner.borrow();
        inner
            .animation_tracks
            .contains(&(target.to_owned(), property.clone()))
            || inner.animation_tracks.len() < MOTION_TRACK_BUDGET
    }

    fn remember_track(&self, target: &str, property: &AnimationProperty) {
        self.inner
            .borrow_mut()
            .animation_tracks
            .insert((target.to_owned(), property.clone()));
    }

    /// Tween every `transition` property whose painted value changed since
    /// the last observation. The first observation only records values; a
    /// retarget starts from the value currently on screen.
    pub fn observe_transitions(
        &self,
        runtime: &Runtime,
        view: &dyn MotionSceneView,
        animation: &MotionTimeline,
        effective: MotionPreference,
        now_ms: u64,
    ) {
        let declarations = node_declarations(runtime, true);
        for (target, transition, _) in declarations {
            let Some(transition) = transition else {
                continue;
            };
            if view.node_rect(&target).is_none() {
                continue;
            }
            let properties: Vec<&str> = transition
                .properties
                .as_ref()
                .map(|properties| properties.iter().map(String::as_str).collect())
                .unwrap_or_else(|| P1_MOTION_PROPERTIES.to_vec());
            for name in properties {
                if !P1_MOTION_PROPERTIES.contains(&name) {
                    animation.warn(&format!("motion property {name} on node {target} rejected"));
                    continue;
                }
                let Some(descriptor) = animatable_property_registry().get(name) else {
                    continue;
                };
                let property = AnimationProperty::from_registered(name, descriptor.apply);
                let Some(desired) = view.property(&target, &property) else {
                    continue;
                };
                let key = (target.clone(), property.clone());
                let previous = self
                    .inner
                    .borrow_mut()
                    .observed
                    .insert(key, desired.clone());
                let Some(previous) = previous else {
                    continue;
                };
                if previous == desired {
                    continue;
                }
                let current = animation
                    .current_override(&target, &property)
                    .unwrap_or(previous);
                let request = AnimationRequest {
                    target: target.clone(),
                    property: property.clone(),
                    from: Some(current.clone()),
                    to: desired.clone(),
                    stops: None,
                    duration_ms: if effective == MotionPreference::Reduced {
                        0
                    } else {
                        transition.duration_ms
                    },
                    delay_ms: 0,
                    easing: Easing::from(&transition.easing),
                    iterations: 1,
                    direction: AnimationDirection::Normal,
                    fill_mode: AnimationFillMode::Forwards,
                    requested_at_ms: now_ms,
                };
                if effective == MotionPreference::Reduced {
                    animation.set_instant(
                        &target,
                        &property,
                        desired,
                        Some(&view.property(&target, &property).unwrap_or(Value::Null)),
                    );
                } else {
                    let _ = animation.start_immediate(request, current, Some(desired), now_ms);
                }
            }
        }
    }

    /// Start every `mount` animation in the document.
    pub fn start_mount_animations(
        &self,
        runtime: &Runtime,
        view: &dyn MotionSceneView,
        animation: &MotionTimeline,
        effective: MotionPreference,
        now_ms: u64,
    ) {
        for (target, _, animations) in node_declarations(runtime, false) {
            for declaration in animations {
                if declaration.trigger == MotionTrigger::Mount {
                    self.start_node_animation(
                        &target,
                        &declaration,
                        view,
                        animation,
                        effective,
                        now_ms,
                    );
                }
            }
        }
    }

    /// Paint mount and in-view animations at their first keyframe before the
    /// lifecycle starts. Manual animations keep the authored look until an
    /// interaction plays them.
    pub fn set_initial_lifecycle_values(
        &self,
        runtime: &Runtime,
        view: &dyn MotionSceneView,
        animation: &MotionTimeline,
    ) {
        for (target, _, animations) in node_declarations(runtime, false) {
            for declaration in animations {
                if !matches!(
                    declaration.trigger,
                    MotionTrigger::Mount | MotionTrigger::InView
                ) {
                    continue;
                }
                self.set_node_animation_initial_values(&target, &declaration, view, animation);
            }
        }
    }

    fn set_node_animation_initial_values(
        &self,
        target: &str,
        declaration: &NodeAnimation,
        view: &dyn MotionSceneView,
        animation: &MotionTimeline,
    ) {
        if view.node_rect(target).is_none() {
            return;
        }
        let mut by_property: BTreeMap<String, Vec<(f32, Value)>> = BTreeMap::new();
        for keyframe in &declaration.keyframes {
            for (property, value) in &keyframe.values {
                by_property
                    .entry(property.clone())
                    .or_default()
                    .push((keyframe.offset, value.clone()));
            }
        }
        for (name, stops) in by_property {
            if !P1_MOTION_PROPERTIES.contains(&name.as_str()) {
                continue;
            }
            let Some(descriptor) = animatable_property_registry().get(&name) else {
                continue;
            };
            let property = AnimationProperty::from_registered(&name, descriptor.apply);
            let Some(base) = view.property(target, &property) else {
                continue;
            };
            let Some((_, first)) = stops.first() else {
                continue;
            };
            animation.set_instant(target, &property, first.clone(), Some(&base));
        }
    }

    fn start_node_animation(
        &self,
        target: &str,
        declaration: &NodeAnimation,
        view: &dyn MotionSceneView,
        animation: &MotionTimeline,
        effective: MotionPreference,
        now_ms: u64,
    ) {
        if view.node_rect(target).is_none() {
            return;
        }
        let mut by_property: BTreeMap<String, Vec<(f32, Value)>> = BTreeMap::new();
        for keyframe in &declaration.keyframes {
            for (property, value) in &keyframe.values {
                by_property
                    .entry(property.clone())
                    .or_default()
                    .push((keyframe.offset, value.clone()));
            }
        }
        for (name, stops) in by_property {
            if !P1_MOTION_PROPERTIES.contains(&name.as_str()) {
                animation.warn(&format!("motion property {name} on node {target} rejected"));
                continue;
            }
            let Some(descriptor) = animatable_property_registry().get(&name) else {
                continue;
            };
            let property = AnimationProperty::from_registered(&name, descriptor.apply);
            let Some(base) = view.property(target, &property) else {
                continue;
            };
            let Some(first) = stops.first() else {
                continue;
            };
            let final_value = stops.last().expect("first stop exists").1.clone();
            if effective == MotionPreference::Reduced {
                if declaration.fill_mode == NodeAnimationFillMode::Forwards {
                    animation.set_instant(target, &property, final_value, Some(&base));
                }
                continue;
            }
            if !self.track_budget_available(animation, target, &property) {
                animation.warn(&format!(
                    "motion track budget exceeded for {name} on node {target}"
                ));
                continue;
            }
            let request = AnimationRequest {
                target: target.to_owned(),
                property: property.clone(),
                from: Some(first.1.clone()),
                to: final_value,
                stops: Some(stops),
                duration_ms: declaration.duration_ms,
                delay_ms: 0,
                easing: Easing::from(&declaration.easing),
                // An infinite animation runs u32::MAX passes: effectively forever.
                iterations: declaration.iterations.count_or(u32::MAX).max(1),
                direction: AnimationDirection::Normal,
                // Lifecycle keyframe 0 is the pre-reveal look. Schema P1 only
                // has forwards|none, so map those onto Both|Backwards to hold
                // the first keyframe for the whole delay instead of painting
                // the authored value until `delayMs` elapses.
                fill_mode: match declaration.fill_mode {
                    NodeAnimationFillMode::Forwards => AnimationFillMode::Both,
                    NodeAnimationFillMode::None => AnimationFillMode::Backwards,
                },
                requested_at_ms: now_ms.saturating_add(declaration.delay_ms),
            };
            if animation.start_immediate(request, base.clone(), Some(base), now_ms)
                && animation.has_active(target, &property)
            {
                self.remember_track(target, &property);
            }
        }
    }
}

#[cfg(test)]
#[path = "triggers_tests.rs"]
mod tests;
