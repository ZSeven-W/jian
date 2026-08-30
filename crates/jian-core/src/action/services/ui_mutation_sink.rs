//! Platform-neutral delivery for Preview-only UI mutations.
//!
//! Actions emit typed requests through this service instead of rewriting
//! document JSON. Preview hosts retain and apply the requests; ordinary Jian
//! runtimes install [NullUiMutationSink], whose unsupported result becomes a
//! runtime diagnostic at the action boundary.

/// Where a scroll_to target should land in its scroll container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAlignment {
    Start,
    Center,
    End,
    Nearest,
}

impl ScrollAlignment {
    pub fn parse(authored: &str) -> Option<Self> {
        match authored {
            "start" => Some(Self::Start),
            "center" => Some(Self::Center),
            "end" => Some(Self::End),
            "nearest" => Some(Self::Nearest),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Center => "center",
            Self::End => "end",
            Self::Nearest => "nearest",
        }
    }
}

/// One typed mutation against runtime-node presentation state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiMutationRequest {
    SetVisibility {
        node_id: String,
        visible: bool,
    },
    ToggleVisibility {
        node_id: String,
    },
    ScrollTo {
        target_id: String,
        alignment: ScrollAlignment,
    },
}

/// Host work invalidated by applying a UI mutation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UiMutationWork {
    pub redraw: bool,
    pub rebuild_hit_test: bool,
}

impl UiMutationWork {
    pub const NONE: Self = Self {
        redraw: false,
        rebuild_hit_test: false,
    };

    pub const REDRAW_AND_HIT_TEST: Self = Self {
        redraw: true,
        rebuild_hit_test: true,
    };

    pub fn merge(&mut self, other: Self) {
        self.redraw |= other.redraw;
        self.rebuild_hit_test |= other.rebuild_hit_test;
    }
}

/// Result of handing one mutation to the host-owned state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiMutationOutcome {
    Applied(UiMutationWork),
    Unsupported,
    Rejected(String),
}

/// Preview-neutral service carried by every action context.
pub trait UiMutationSink {
    fn apply(&self, request: &UiMutationRequest) -> UiMutationOutcome;
}

/// Diagnostic no-op for runtimes that do not host Preview UI state.
pub struct NullUiMutationSink;

impl UiMutationSink for NullUiMutationSink {
    fn apply(&self, _request: &UiMutationRequest) -> UiMutationOutcome {
        UiMutationOutcome::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_round_trips() {
        for alignment in [
            ScrollAlignment::Start,
            ScrollAlignment::Center,
            ScrollAlignment::End,
            ScrollAlignment::Nearest,
        ] {
            assert_eq!(ScrollAlignment::parse(alignment.as_str()), Some(alignment));
        }
        assert_eq!(ScrollAlignment::parse("middle"), None);
    }

    #[test]
    fn work_merge_is_monotonic() {
        let mut work = UiMutationWork::NONE;
        work.merge(UiMutationWork {
            redraw: true,
            rebuild_hit_test: false,
        });
        work.merge(UiMutationWork {
            redraw: false,
            rebuild_hit_test: true,
        });
        assert_eq!(work, UiMutationWork::REDRAW_AND_HIT_TEST);
    }
}
