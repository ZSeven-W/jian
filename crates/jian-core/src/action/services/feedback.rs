use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FeedbackLevel {
    Info,
    Success,
    Warning,
    Error,
}

pub trait FeedbackSink {
    fn toast(&self, message: &str, level: FeedbackLevel, duration_ms: u32);
    fn alert(&self, title: &str, message: &str);
}

#[async_trait(?Send)]
pub trait AsyncFeedback {
    /// Present a Confirm dialog and return the user's choice (true=confirm).
    async fn confirm(&self, title: &str, message: &str) -> bool;
}
