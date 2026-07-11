use super::{
    ActionContext, ActionError, ActionResult, CancellationToken, ExecOutcome, SharedRegistry,
};
use futures::task::{waker_ref, ArcWake};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

#[derive(Default)]
pub struct TaskClock {
    now_ms: Cell<u64>,
    sleepers: RefCell<Vec<(u64, std::task::Waker)>>,
}

impl TaskClock {
    pub fn advance_to(&self, now_ms: u64) {
        self.now_ms.set(self.now_ms.get().max(now_ms));
        let now = self.now_ms.get();
        let mut sleepers = self.sleepers.borrow_mut();
        let mut index = 0;
        while index < sleepers.len() {
            if sleepers[index].0 <= now {
                sleepers.remove(index).1.wake();
            } else {
                index += 1;
            }
        }
    }

    pub fn sleep(self: &Arc<Self>, duration_ms: u64) -> Sleep {
        Sleep {
            clock: self.clone(),
            deadline: self.now_ms.get().saturating_add(duration_ms),
            registered: false,
        }
    }

    pub fn next_deadline(&self) -> Option<u64> {
        self.sleepers
            .borrow()
            .iter()
            .map(|(deadline, _)| *deadline)
            .min()
    }
}

pub struct Sleep {
    clock: Arc<TaskClock>,
    deadline: u64,
    registered: bool,
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.clock.now_ms.get() >= self.deadline {
            return Poll::Ready(());
        }
        if !self.registered {
            self.clock
                .sleepers
                .borrow_mut()
                .push((self.deadline, context.waker().clone()));
            self.registered = true;
        }
        Poll::Pending
    }
}

struct WakeFlag(AtomicBool);

impl ArcWake for WakeFlag {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.0.store(true, Ordering::Release);
    }
}

pub struct ActionTask {
    pub id: u64,
    pub generation: u64,
    pub source: Option<String>,
    future: Pin<Box<dyn Future<Output = ExecOutcome>>>,
    wake: Arc<WakeFlag>,
    cancel: CancellationToken,
}

#[derive(Default)]
pub struct TaskQueue {
    next_id: u64,
    tasks: Vec<ActionTask>,
}

impl TaskQueue {
    pub fn spawn_future(
        &mut self,
        future: impl Future<Output = ExecOutcome> + 'static,
        generation: u64,
        source: Option<String>,
    ) -> u64 {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        self.tasks.push(ActionTask {
            id,
            generation,
            source,
            future: Box::pin(future),
            wake: Arc::new(WakeFlag(AtomicBool::new(true))),
            cancel: CancellationToken::new(),
        });
        id
    }

    pub fn spawn(
        &mut self,
        registry: &SharedRegistry,
        list: &Value,
        context: ActionContext,
        generation: u64,
        source: Option<String>,
    ) -> Result<u64, ActionError> {
        let chain = registry.borrow().parse_list(list)?;
        let cancel = context.cancel.clone();
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        let future = Box::pin(async move {
            let result = chain.run_serial(&context).await;
            ExecOutcome {
                result,
                warnings: context.take_warnings(),
            }
        });
        self.tasks.push(ActionTask {
            id,
            generation,
            source,
            future,
            wake: Arc::new(WakeFlag(AtomicBool::new(true))),
            cancel,
        });
        Ok(id)
    }

    pub fn poll_all(&mut self, _now_ms: u64) -> Vec<ExecOutcome> {
        let mut completed = Vec::new();
        let mut index = 0;
        while index < self.tasks.len() {
            if !self.tasks[index].wake.0.swap(false, Ordering::AcqRel) {
                index += 1;
                continue;
            }
            let wake = self.tasks[index].wake.clone();
            let waker = waker_ref(&wake);
            let mut context = Context::from_waker(&waker);
            match self.tasks[index].future.as_mut().poll(&mut context) {
                Poll::Ready(outcome) => {
                    completed.push(outcome);
                    self.tasks.remove(index);
                }
                Poll::Pending => index += 1,
            }
        }
        completed
    }

    pub fn next_wake_ms(&self, now_ms: u64) -> Option<u64> {
        self.tasks
            .iter()
            .any(|task| task.wake.0.load(Ordering::Acquire))
            .then_some(now_ms)
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn cancel_generation(&mut self, generation: u64) {
        self.tasks.retain(|task| {
            if task.generation == generation {
                task.cancel.cancel();
                false
            } else {
                true
            }
        });
    }

    pub fn cancel_all(&mut self) {
        for task in &self.tasks {
            task.cancel.cancel();
        }
        self.tasks.clear();
    }
}

#[allow(dead_code)]
fn _result(_: ActionResult) {}
