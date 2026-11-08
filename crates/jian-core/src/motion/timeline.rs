//! The bounded animation timeline every motion consumer drives, moved
//! unchanged from OpenPencil's preview runtime.
//!
//! Requests arrive through [`AnimationSink`] (the `animate` action) or are
//! started directly by the motion triggers. Each `(target, property)` pair
//! owns at most one track; `tick` samples every track at the host clock and
//! keeps the current value as an override until the track finishes. A host
//! applies the overrides to whatever scene it paints.

use super::curve::ease;
use super::interpolate::{interpolate_kind, interpolate_pair, interpolate_stops};
use crate::action::animation_registry::AnimationInterpolate;
use crate::action::services::{
    AnimationDirection, AnimationFillMode, AnimationOutcome, AnimationProperty, AnimationRequest,
    AnimationSink,
};
use crate::binding::{BindingTarget, InvalidationKind};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

/// Notified when a request reaches the timeline; hosts use it for traces.
pub trait MotionObserver {
    fn animation_requested(&self, target: &str, property: &str, at_ms: u64);

    /// A declaration was skipped (unsupported property, track budget).
    /// jian-core carries no logging dependency, so hosts route these to
    /// their own logs.
    fn motion_warning(&self, _message: &str) {}
}

/// The invalidation an animated property causes. Properties outside the
/// registry repaint.
pub fn invalidation_for(property: &AnimationProperty) -> InvalidationKind {
    property
        .invalidation()
        .unwrap_or(InvalidationKind::PaintOnly)
}

/// Sample one request at `now_ms` the way [`MotionTimeline::tick`] samples
/// an admitted track: `(value, finished)`, where `None` means no override.
pub fn sample_request(
    request: &AnimationRequest,
    now_ms: u64,
) -> (Option<serde_json::Value>, bool) {
    let track = Track {
        request: request.clone(),
        from: request.from.clone().unwrap_or(serde_json::Value::Null),
        base: None,
    };
    let sample = sample_track(&track, now_ms);
    (sample.value, sample.finished)
}

const FRAME_INTERVAL_MS: u64 = 16;
const MAX_TRACKS: usize = 256;

type TrackKey = (String, AnimationProperty);

#[derive(Clone)]
struct Track {
    request: AnimationRequest,
    from: serde_json::Value,
    base: Option<serde_json::Value>,
}

#[derive(Default)]
struct TimelineInner {
    pending: VecDeque<AnimationRequest>,
    tracks: Vec<Track>,
    overrides: BTreeMap<TrackKey, serde_json::Value>,
    override_invalidation: BTreeMap<TrackKey, InvalidationKind>,
    now_ms: u64,
    last_invalidation: InvalidationKind,
}

impl TimelineInner {
    fn next_deadline_ms(&self) -> Option<u64> {
        self.tracks
            .iter()
            .filter_map(|track| track_deadline(track, self.now_ms))
            .min()
    }
}

#[derive(Clone, Default)]
pub struct MotionTimeline {
    inner: Rc<RefCell<TimelineInner>>,
    observer: Rc<RefCell<Option<Rc<dyn MotionObserver>>>>,
}

impl AnimationSink for MotionTimeline {
    fn request(&self, request: &AnimationRequest) -> AnimationOutcome {
        let mut inner = self.inner.borrow_mut();
        let key = (request.target.clone(), request.property.clone());
        let keys: BTreeSet<_> = inner
            .tracks
            .iter()
            .map(track_key)
            .chain(inner.pending.iter().map(request_key))
            .chain(inner.overrides.keys().cloned())
            .collect();
        if !keys.contains(&key) && keys.len() >= MAX_TRACKS {
            return AnimationOutcome::Rejected("animation timeline is full".to_owned());
        }
        inner.pending.retain(|pending| request_key(pending) != key);
        inner.pending.push_back(request.clone());
        drop(inner);
        if let Some(observer) = self.observer.borrow().as_ref() {
            observer.animation_requested(
                &request.target,
                request.property.name(),
                request.requested_at_ms,
            );
        }
        AnimationOutcome::Accepted
    }
}

impl MotionTimeline {
    /// Report every accepted request, for host debug traces.
    pub fn set_observer(&self, observer: Rc<dyn MotionObserver>) {
        *self.observer.borrow_mut() = Some(observer);
    }

    /// Forward a skipped-declaration warning to the observer, if any.
    pub fn warn(&self, message: &str) {
        if let Some(observer) = self.observer.borrow().as_ref() {
            observer.motion_warning(message);
        }
    }

    pub fn take_pending(&self) -> Vec<AnimationRequest> {
        self.inner.borrow_mut().pending.drain(..).collect()
    }

    pub fn admit(
        &self,
        mut request: AnimationRequest,
        from: serde_json::Value,
        base: Option<serde_json::Value>,
    ) {
        let key = request_key(&request);
        let mut inner = self.inner.borrow_mut();
        inner.tracks.retain(|track| track_key(track) != key);
        inner.overrides.remove(&key);
        inner.override_invalidation.remove(&key);
        request.from = Some(from.clone());
        inner.tracks.push(Track {
            request,
            from,
            base,
        });
    }

    pub fn start_immediate(
        &self,
        mut request: AnimationRequest,
        from: serde_json::Value,
        base: Option<serde_json::Value>,
        now_ms: u64,
    ) -> bool {
        let key = request_key(&request);
        let mut inner = self.inner.borrow_mut();
        let keys: BTreeSet<_> = inner
            .tracks
            .iter()
            .map(track_key)
            .chain(inner.pending.iter().map(request_key))
            .chain(inner.overrides.keys().cloned())
            .collect();
        if !keys.contains(&key) && keys.len() >= MAX_TRACKS {
            return false;
        }
        inner.tracks.retain(|track| track_key(track) != key);
        inner.pending.retain(|pending| request_key(pending) != key);
        inner.overrides.remove(&key);
        inner.override_invalidation.remove(&key);
        request.from = Some(from.clone());
        inner.tracks.push(Track {
            request,
            from,
            base,
        });
        drop(inner);
        self.tick(now_ms);
        true
    }

    pub fn set_instant(
        &self,
        target: &str,
        property: &AnimationProperty,
        value: serde_json::Value,
        base: Option<&serde_json::Value>,
    ) {
        let key = (target.to_owned(), property.clone());
        let mut inner = self.inner.borrow_mut();
        inner.tracks.retain(|track| track_key(track) != key);
        inner.pending.retain(|request| request_key(request) != key);
        if base == Some(&value) {
            inner.overrides.remove(&key);
            inner.override_invalidation.remove(&key);
        } else {
            inner.overrides.insert(key.clone(), value);
            inner
                .override_invalidation
                .insert(key.clone(), invalidation_for(property));
        }
        inner.last_invalidation = invalidation_for(property);
    }

    pub fn tick(&self, now_ms: u64) -> InvalidationKind {
        let mut inner = self.inner.borrow_mut();
        inner.now_ms = inner.now_ms.max(now_ms);
        let now_ms = inner.now_ms;
        let tracks = std::mem::take(&mut inner.tracks);
        let mut retained = Vec::with_capacity(tracks.len());
        let mut invalidation = InvalidationKind::None;
        for track in tracks {
            let key = track_key(&track);
            let before = inner.overrides.get(&key).cloned();
            let sample = sample_track(&track, now_ms);
            match sample.value {
                Some(value) if track.base.as_ref() != Some(&value) => {
                    inner.overrides.insert(key.clone(), value);
                    let kind = invalidation_for(&track.request.property);
                    inner.override_invalidation.insert(key.clone(), kind);
                }
                _ => {
                    inner.overrides.remove(&key);
                    inner.override_invalidation.remove(&key);
                }
            }
            let after = inner.overrides.get(&key);
            if before.as_ref() != after {
                invalidation = invalidation.merge(invalidation_for(&track.request.property));
            }
            if !sample.finished {
                retained.push(track);
            }
        }
        inner.tracks = retained;
        inner.last_invalidation = invalidation;
        invalidation
    }

    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.inner.borrow().next_deadline_ms()
    }

    pub fn has_visual_state(&self) -> bool {
        !self.inner.borrow().overrides.is_empty()
    }

    pub fn binding_values(&self) -> BTreeMap<(String, BindingTarget), serde_json::Value> {
        self.inner
            .borrow()
            .overrides
            .iter()
            .filter_map(|((target, property), value)| {
                property
                    .binding_target()
                    .map(|binding| ((target.clone(), binding), value.clone()))
            })
            .collect()
    }

    pub fn current_override(
        &self,
        target: &str,
        property: &AnimationProperty,
    ) -> Option<serde_json::Value> {
        self.inner
            .borrow()
            .overrides
            .get(&(target.to_owned(), property.clone()))
            .cloned()
    }

    pub fn cancel(&self, target: &str, property: &AnimationProperty) -> Option<InvalidationKind> {
        let key = (target.to_owned(), property.clone());
        let mut inner = self.inner.borrow_mut();
        let before_tracks = inner.tracks.len();
        let before_pending = inner.pending.len();
        inner.tracks.retain(|track| track_key(track) != key);
        inner.pending.retain(|request| request_key(request) != key);
        let removed_override = inner.overrides.remove(&key).is_some();
        let invalidation = inner.override_invalidation.remove(&key);
        let removed_track = inner.tracks.len() != before_tracks;
        let removed_pending = inner.pending.len() != before_pending;
        if removed_override || removed_track {
            let kind = invalidation.unwrap_or_else(|| invalidation_for(property));
            inner.last_invalidation = kind;
            Some(kind)
        } else if removed_pending {
            inner.last_invalidation = InvalidationKind::None;
            Some(InvalidationKind::None)
        } else {
            None
        }
    }

    pub fn clear(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.pending.clear();
        inner.tracks.clear();
        inner.overrides.clear();
        inner.override_invalidation.clear();
        inner.last_invalidation = InvalidationKind::None;
    }

    pub fn track_count(&self) -> usize {
        self.inner.borrow().tracks.len()
    }

    pub fn has_active(&self, target: &str, property: &AnimationProperty) -> bool {
        self.inner
            .borrow()
            .tracks
            .iter()
            .any(|track| track.request.target == target && &track.request.property == property)
    }

    pub fn last_invalidation(&self) -> InvalidationKind {
        self.inner.borrow().last_invalidation
    }
}

struct TrackSample {
    value: Option<serde_json::Value>,
    finished: bool,
}

fn sample_track(track: &Track, now_ms: u64) -> TrackSample {
    let start = track
        .request
        .requested_at_ms
        .saturating_add(track.request.delay_ms);
    let total = track
        .request
        .duration_ms
        .saturating_mul(u64::from(track.request.iterations));
    let end = start.saturating_add(total);
    if now_ms < start {
        let value = matches!(
            track.request.fill_mode,
            AnimationFillMode::Backwards | AnimationFillMode::Both
        )
        .then(|| interpolate_track(track, directed_progress(track, 0, 0.0)));
        return TrackSample {
            value,
            finished: false,
        };
    }
    if now_ms >= end {
        let last_iteration = track.request.iterations.saturating_sub(1);
        let value = matches!(
            track.request.fill_mode,
            AnimationFillMode::Forwards | AnimationFillMode::Both
        )
        .then(|| interpolate_track(track, directed_progress(track, last_iteration, 1.0)));
        return TrackSample {
            value,
            finished: true,
        };
    }
    if track.request.duration_ms == 0 {
        let value = matches!(
            track.request.fill_mode,
            AnimationFillMode::Forwards | AnimationFillMode::Both
        )
        .then(|| interpolate_track(track, 1.0));
        return TrackSample {
            value,
            finished: true,
        };
    }
    let elapsed = now_ms - start;
    let iteration = (elapsed / track.request.duration_ms) as u32;
    let local = (elapsed % track.request.duration_ms) as f32 / track.request.duration_ms as f32;
    TrackSample {
        value: Some(interpolate_track(
            track,
            directed_progress(track, iteration, local),
        )),
        finished: false,
    }
}

fn directed_progress(track: &Track, iteration: u32, local: f32) -> f32 {
    let reverse = match track.request.direction {
        AnimationDirection::Normal => false,
        AnimationDirection::Reverse => true,
        AnimationDirection::Alternate => iteration % 2 == 1,
        // `% 2 == 0` rather than `is_multiple_of`: jian supports Rust 1.78.
        AnimationDirection::AlternateReverse => iteration % 2 == 0,
    };
    let directed = if reverse { 1.0 - local } else { local };
    ease(track.request.easing, directed.clamp(0.0, 1.0))
}

fn interpolate_track(track: &Track, progress: f32) -> serde_json::Value {
    let interpolate = interpolate_kind(&track.request.property);
    if let Some(stops) = &track.request.stops {
        return interpolate_stops(stops, interpolate, progress);
    }
    interpolate_pair(&track.from, &track.request.to, interpolate, progress)
}

fn track_deadline(track: &Track, now_ms: u64) -> Option<u64> {
    let start = track
        .request
        .requested_at_ms
        .saturating_add(track.request.delay_ms);
    let end = start.saturating_add(
        track
            .request
            .duration_ms
            .saturating_mul(u64::from(track.request.iterations)),
    );
    if now_ms < start {
        return Some(start);
    }
    if now_ms >= end {
        return None;
    }
    let elapsed = now_ms - start;
    let next_iteration = start
        .saturating_add(
            (elapsed / track.request.duration_ms + 1).saturating_mul(track.request.duration_ms),
        )
        .min(end);
    let interpolate = interpolate_kind(&track.request.property);
    if interpolate == AnimationInterpolate::Discrete {
        Some(next_iteration)
    } else {
        Some(now_ms.saturating_add(FRAME_INTERVAL_MS).min(next_iteration))
    }
}

fn request_key(request: &AnimationRequest) -> TrackKey {
    (request.target.clone(), request.property.clone())
}

fn track_key(track: &Track) -> TrackKey {
    request_key(&track.request)
}

#[cfg(test)]
#[path = "timeline_tests.rs"]
mod tests;
