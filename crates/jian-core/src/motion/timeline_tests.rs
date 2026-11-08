use super::*;
use crate::action::animation_registry::animatable_property_registry;
use crate::action::services::Easing;
use serde_json::json;
use std::cell::Cell;

fn property(name: &str) -> AnimationProperty {
    let descriptor = animatable_property_registry()
        .get(name)
        .expect("registered");
    AnimationProperty::from_registered(name, descriptor.apply)
}

fn request(target: &str, iterations: u32) -> AnimationRequest {
    AnimationRequest {
        target: target.to_owned(),
        property: property("opacity"),
        from: Some(json!(0.0)),
        to: json!(1.0),
        stops: None,
        duration_ms: 100,
        delay_ms: 0,
        easing: Easing::Linear,
        iterations,
        direction: AnimationDirection::Normal,
        fill_mode: AnimationFillMode::Forwards,
        requested_at_ms: 0,
    }
}

#[test]
fn a_full_timeline_rejects_new_targets_but_keeps_existing_ones() {
    let timeline = MotionTimeline::default();
    for index in 0..MAX_TRACKS {
        let outcome = timeline.request(&request(&format!("node-{index}"), 1));
        assert_eq!(outcome, AnimationOutcome::Accepted);
    }
    assert!(matches!(
        timeline.request(&request("one-more", 1)),
        AnimationOutcome::Rejected(_)
    ));
    assert_eq!(
        timeline.request(&request("node-0", 1)),
        AnimationOutcome::Accepted
    );
}

#[test]
fn a_started_track_overrides_until_it_finishes() {
    let timeline = MotionTimeline::default();
    assert!(timeline.start_immediate(request("box", 1), json!(0.0), Some(json!(1.0)), 0));
    assert_eq!(timeline.track_count(), 1);
    assert_eq!(timeline.tick(50), InvalidationKind::PaintOnly);
    assert_eq!(
        timeline.current_override("box", &property("opacity")),
        Some(json!(0.5))
    );
    timeline.tick(100);
    assert_eq!(timeline.track_count(), 0);
    // Forwards fill lands on the base value, so no override remains.
    assert!(!timeline.has_visual_state());
    assert_eq!(timeline.next_deadline_ms(), None);
}

#[test]
fn a_nearly_infinite_count_keeps_the_track_alive() {
    let timeline = MotionTimeline::default();
    timeline.start_immediate(request("spinner", u32::MAX), json!(0.0), None, 0);
    timeline.tick(10_000_000);
    assert_eq!(timeline.track_count(), 1);
    assert!(timeline.next_deadline_ms().is_some());
}

#[test]
fn set_instant_clears_the_override_when_it_equals_the_base() {
    let timeline = MotionTimeline::default();
    let opacity = property("opacity");
    timeline.set_instant("box", &opacity, json!(0.3), Some(&json!(1.0)));
    assert_eq!(timeline.current_override("box", &opacity), Some(json!(0.3)));
    timeline.set_instant("box", &opacity, json!(1.0), Some(&json!(1.0)));
    assert_eq!(timeline.current_override("box", &opacity), None);
}

#[test]
fn cancel_reports_what_it_removed() {
    let timeline = MotionTimeline::default();
    let opacity = property("opacity");
    assert_eq!(timeline.cancel("box", &opacity), None);
    timeline.request(&request("box", 1));
    assert_eq!(
        timeline.cancel("box", &opacity),
        Some(InvalidationKind::None)
    );
    timeline.start_immediate(request("box", 1), json!(0.0), None, 0);
    assert_eq!(
        timeline.cancel("box", &opacity),
        Some(InvalidationKind::PaintOnly)
    );
}

#[test]
fn the_observer_hears_every_accepted_request() {
    struct Counter(Cell<usize>);
    impl MotionObserver for Counter {
        fn animation_requested(&self, target: &str, property: &str, at_ms: u64) {
            assert_eq!((target, property, at_ms), ("box", "opacity", 0));
            self.0.set(self.0.get() + 1);
        }
    }
    let timeline = MotionTimeline::default();
    let counter = Rc::new(Counter(Cell::new(0)));
    timeline.set_observer(counter.clone());
    timeline.request(&request("box", 1));
    assert_eq!(counter.0.get(), 1);
}

#[test]
fn warnings_reach_the_observer_and_are_optional() {
    struct Warnings(RefCell<Vec<String>>);
    impl MotionObserver for Warnings {
        fn animation_requested(&self, _: &str, _: &str, _: u64) {}
        fn motion_warning(&self, message: &str) {
            self.0.borrow_mut().push(message.to_owned());
        }
    }
    let timeline = MotionTimeline::default();
    timeline.warn("dropped before an observer exists");
    let warnings = Rc::new(Warnings(RefCell::new(Vec::new())));
    timeline.set_observer(warnings.clone());
    timeline.warn("budget exceeded");
    assert_eq!(*warnings.0.borrow(), ["budget exceeded"]);
}
