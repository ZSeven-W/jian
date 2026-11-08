//! Lifecycle, in-view, transition and switch triggers for document motion,
//! moved from OpenPencil's preview. They read the painted scene through
//! [`MotionSceneView`] and start tracks on a [`MotionTimeline`].

use super::in_view::InViewWindow;
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(doc)]
use super::{scene_view::MotionSceneView, timeline::MotionTimeline};

/// Per-screen trigger bookkeeping, cleared by [`MotionTriggers::reset_screen`].
#[derive(Default)]
struct MotionInner {
    lifecycle_started: bool,
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
}

#[cfg(test)]
#[path = "triggers_tests.rs"]
mod tests;
