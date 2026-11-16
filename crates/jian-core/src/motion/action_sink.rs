use super::{MotionControl, MotionTimeline};
use crate::action::services::{AnimationOutcome, AnimationRequest, AnimationSink};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Default)]
pub struct MotionActionSink {
    pub timeline: MotionTimeline,
    controls: Rc<RefCell<Vec<(MotionControl, u64)>>>,
}

impl MotionActionSink {
    pub fn take_controls(&self) -> Vec<(MotionControl, u64)> {
        std::mem::take(&mut *self.controls.borrow_mut())
    }

    pub fn clear(&self) {
        self.take_controls();
        self.timeline.clear();
    }
}

impl AnimationSink for MotionActionSink {
    fn request(&self, request: &AnimationRequest) -> AnimationOutcome {
        self.timeline.request(request)
    }

    fn control(&self, control: &MotionControl, now_ms: u64) -> AnimationOutcome {
        let mut controls = self.controls.borrow_mut();
        if controls.len() >= 256 {
            return AnimationOutcome::Rejected("too many pending motion controls".into());
        }
        controls.push((control.clone(), now_ms));
        AnimationOutcome::Accepted
    }
}
