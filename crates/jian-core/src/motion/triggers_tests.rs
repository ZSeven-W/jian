use super::*;

#[test]
fn the_lifecycle_starts_once_per_screen() {
    let triggers = MotionTriggers::default();
    assert!(!triggers.lifecycle_started());
    assert!(triggers.begin_lifecycle());
    assert!(!triggers.begin_lifecycle());
    triggers.reset_screen();
    assert!(!triggers.lifecycle_started());
    assert!(triggers.begin_lifecycle());
}

#[test]
fn the_in_view_window_survives_a_screen_reset() {
    let triggers = MotionTriggers::default();
    let window = InViewWindow::new(100.0, 0.0, 800.0, Vec::new());
    assert!(triggers.set_in_view_window(Some(window.clone())));
    assert!(!triggers.set_in_view_window(Some(window.clone())));
    triggers.reset_screen();
    assert_eq!(*triggers.in_view_window.borrow(), Some(window));
    assert!(triggers.set_in_view_window(None));
}
