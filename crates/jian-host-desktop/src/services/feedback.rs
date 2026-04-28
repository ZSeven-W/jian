//! `rfd::MessageDialog`-backed `FeedbackSink + AsyncFeedback`.
//!
//! Gated behind the `feedback` cargo feature. `rfd` 0.14 wraps each
//! platform's native dialog API:
//!
//! - macOS: `NSAlert` over the running app.
//! - Windows: `MessageBoxW`.
//! - Linux: `xdg-desktop-portal` if available, else GTK / KDE
//!   depending on what's installed.
//!
//! `toast` has no first-class native equivalent on every desktop, so
//! this impl uses a non-blocking `MessageDialog` with the Info /
//! Warning / Error icon picked from `FeedbackLevel`. Real toasts (a
//! transient strip in the window) ship in a future host-side widget,
//! tracked separately.

use async_trait::async_trait;
use jian_core::action::services::feedback::{AsyncFeedback, FeedbackLevel, FeedbackSink};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

/// Native-dialog `FeedbackSink + AsyncFeedback` for the desktop host.
///
/// `Default` is the canonical constructor — `rfd::MessageDialog` has
/// no per-instance state to thread through.
pub struct DesktopFeedback;

impl DesktopFeedback {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DesktopFeedback {
    fn default() -> Self {
        Self::new()
    }
}

fn dialog_level(level: FeedbackLevel) -> MessageLevel {
    match level {
        // rfd doesn't have a `Success` icon — Info is the closest
        // visual match (typically a blue `i`).
        FeedbackLevel::Success | FeedbackLevel::Info => MessageLevel::Info,
        FeedbackLevel::Warning => MessageLevel::Warning,
        FeedbackLevel::Error => MessageLevel::Error,
    }
}

impl FeedbackSink for DesktopFeedback {
    fn toast(&self, message: &str, level: FeedbackLevel, _duration_ms: u32) {
        // `MessageDialog::show` is blocking on every platform, which is
        // fine because the runtime calls toast from action handlers
        // (synchronous w.r.t. the action's lifecycle). The duration_ms
        // hint is ignored — native dialogs don't auto-dismiss; a real
        // in-window toast widget will honour it.
        MessageDialog::new()
            .set_level(dialog_level(level))
            .set_title("Notice")
            .set_description(message)
            .set_buttons(MessageButtons::Ok)
            .show();
    }

    fn alert(&self, title: &str, message: &str) {
        MessageDialog::new()
            .set_level(MessageLevel::Info)
            .set_title(title)
            .set_description(message)
            .set_buttons(MessageButtons::Ok)
            .show();
    }
}

#[async_trait(?Send)]
impl AsyncFeedback for DesktopFeedback {
    async fn confirm(&self, title: &str, message: &str) -> bool {
        // `MessageDialog::show()` returns a `MessageDialogResult`
        // describing which button the user picked. `Yes` / `Ok` map to
        // confirm; anything else (Cancel, dismissed via Esc, No) is a
        // rejection.
        let result = MessageDialog::new()
            .set_level(MessageLevel::Info)
            .set_title(title)
            .set_description(message)
            .set_buttons(MessageButtons::YesNo)
            .show();
        matches!(
            result,
            MessageDialogResult::Yes | MessageDialogResult::Ok | MessageDialogResult::Custom(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_level_maps_success_and_info_to_info() {
        assert!(matches!(
            dialog_level(FeedbackLevel::Success),
            MessageLevel::Info
        ));
        assert!(matches!(
            dialog_level(FeedbackLevel::Info),
            MessageLevel::Info
        ));
    }

    #[test]
    fn dialog_level_maps_warning_and_error_distinctly() {
        assert!(matches!(
            dialog_level(FeedbackLevel::Warning),
            MessageLevel::Warning
        ));
        assert!(matches!(
            dialog_level(FeedbackLevel::Error),
            MessageLevel::Error
        ));
    }

    // Live `MessageDialog::show` would block on a real screen, so the
    // dialog-rendering paths intentionally lack tests. The trait
    // contract (FeedbackSink / AsyncFeedback) is exercised by jian-core
    // via NullFeedback in headless tests.
}
