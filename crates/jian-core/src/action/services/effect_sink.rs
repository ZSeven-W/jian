//! Platform-neutral EffectSink (R3): the ONE channel effect-producing
//! actions (`open_url`, `copy`, `share`, `haptic`, `focus`, `blur`,
//! `toast`, `alert`, `confirm`; `dismiss_keyboard` in R5) use to hand a
//! request to the host. Jian never performs host effects itself — the
//! sink is a service like `FeedbackSink`, and a no-op diagnostic
//! implementation serves non-Preview runtimes.
//!
//! The Preview host (op-preview-core) injects an adapter that maps
//! requests into its `PreviewEffect` queue DTOs (frozen in
//! op-preview-contracts) with source, capability, and activation; the
//! platform-neutral layer carries only raw values so jian stays free of
//! host types.

use crate::action::context::EffectRequestContext;
use serde_json::Value;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

/// One host-side effect request, as emitted by an action.
#[derive(Debug, Clone, PartialEq)]
pub enum EffectRequest {
    OpenUrl {
        url: String,
    },
    Copy {
        text: String,
    },
    /// Share payload: host-interpreted (text/url mix), kept raw.
    Share {
        payload: Value,
    },
    Haptic {
        style: String,
    },
    FocusNode {
        node_id: String,
    },
    BlurFocus,
    DismissKeyboard,
    Toast {
        message: String,
    },
    Alert {
        title: String,
        message: String,
    },
    Confirm {
        title: String,
        message: String,
    },
}

/// What the sink did with a request. Actions map this onto their
/// continuation: `Ok(())` proceeds on success, `Err` follows the
/// declared error branch (or warns), `Unsupported` is a structured
/// no-op — never a crash or a retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectOutcome {
    /// The host accepted (and later completed) the request.
    Accepted,
    /// The host accepted and supplied a completion future for an authored
    /// continuation such as confirm.on_confirm/on_cancel.
    AcceptedWithCompletion(EffectCompletion),
    /// The host cannot perform this effect class at all.
    Unsupported,
    /// The host rejected the request (invalid payload, permission,
    /// expired activation, presentation failure).
    Rejected(String),
}

impl EffectOutcome {
    pub fn as_result(&self) -> Result<(), String> {
        match self {
            EffectOutcome::Accepted | EffectOutcome::AcceptedWithCompletion(_) => Ok(()),
            EffectOutcome::Unsupported => Err("unsupported".to_owned()),
            EffectOutcome::Rejected(detail) => Err(detail.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectCompletionResult {
    Success,
    Cancelled,
    Unsupported,
    Failed,
}

#[derive(Default)]
struct CompletionState {
    result: Option<EffectCompletionResult>,
    waker: Option<Waker>,
}

#[derive(Clone)]
pub struct EffectCompletion {
    id: u64,
    state: Rc<RefCell<CompletionState>>,
}

impl std::fmt::Debug for EffectCompletion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EffectCompletion")
            .field("id", &self.id)
            .finish()
    }
}

impl PartialEq for EffectCompletion {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && Rc::ptr_eq(&self.state, &other.state)
    }
}

impl Eq for EffectCompletion {}

impl EffectCompletion {
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Future for EffectCompletion {
    type Output = EffectCompletionResult;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.state.borrow_mut();
        if let Some(result) = state.result.clone() {
            Poll::Ready(result)
        } else {
            state.waker = Some(context.waker().clone());
            Poll::Pending
        }
    }
}

#[derive(Clone)]
pub struct EffectCompleter {
    state: Rc<RefCell<CompletionState>>,
}

impl EffectCompleter {
    pub fn complete(&self, result: EffectCompletionResult) -> bool {
        let mut state = self.state.borrow_mut();
        if state.result.is_some() {
            return false;
        }
        state.result = Some(result);
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
        true
    }
}

pub fn effect_completion_pair(id: u64) -> (EffectCompletion, EffectCompleter) {
    let state = Rc::new(RefCell::new(CompletionState::default()));
    (
        EffectCompletion {
            id,
            state: state.clone(),
        },
        EffectCompleter { state },
    )
}

/// The sink every action reaches through the ActionContext. Receives the
/// request plus its factual source context (handler key, node id,
/// activation id when the host certified fresh user intent).
pub trait EffectSink {
    fn request(&self, ctx: &EffectRequestContext, request: &EffectRequest) -> EffectOutcome;
}

/// No-op sink for non-Preview runtimes: reports `Unsupported`, and the
/// calling action turns that into a structured runtime warning — the
/// same warn-and-succeed behavior the effect stubs had before the sink
/// existed.
pub struct NullEffectSink;

impl EffectSink for NullEffectSink {
    fn request(&self, _ctx: &EffectRequestContext, _request: &EffectRequest) -> EffectOutcome {
        EffectOutcome::Unsupported
    }
}
