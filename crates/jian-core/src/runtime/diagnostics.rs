use super::{ReportedActionOutcome, Runtime};

impl Runtime {
    pub fn load_warnings(&self) -> &[String] {
        &self.load_warnings
    }

    pub fn take_action_outcomes(&mut self) -> Vec<ReportedActionOutcome> {
        std::mem::take(&mut self.action_outcomes)
    }

    pub fn take_layout_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.layout_errors)
    }

    pub fn enable_action_reporting(&mut self) {
        self.action_reporting_enabled = true;
    }

    pub fn push_load_warning(&mut self, warning: impl Into<String>) {
        self.load_warnings.push(warning.into());
    }

    pub fn push_layout_error(&mut self, error: impl Into<String>) {
        self.layout_errors.push(error.into());
    }

    pub(super) fn collect_task_outcomes(&mut self) -> bool {
        let outcomes = self.task_queue.poll_all(self.now_ms);
        let completed = !outcomes.is_empty();
        if self.action_reporting_enabled {
            self.action_outcomes
                .extend(outcomes.into_iter().filter_map(|completed| {
                    (completed.outcome.result.is_err() || !completed.outcome.warnings.is_empty())
                        .then_some(ReportedActionOutcome {
                            outcome: completed.outcome,
                            source: completed.source,
                        })
                }));
        }
        completed
    }
}
